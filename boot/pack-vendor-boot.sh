#!/usr/bin/env bash
# Patch vendor_boot so /init is aginxos-init (vendor ramdisk overwrites boot ramdisk /init).
#
# Root cause: Pixel redfin loads boot + vendor_boot ramdisks; vendor's `init` symlink
# replaces anything we put in boot.img's ramdisk.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TOOLS="${ROOT}/boot/tools"
OUTDIR="${ROOT}/boot/out"
INITSRC="${ROOT}/boot/initramfs"
STOCK_VB="${ROOT}/boot/stock-vendor_boot.img"
UNPACK_VB="${OUTDIR}/vendor_boot_unpack"
WORK="${OUTDIR}/vendor-ramdisk-root"
OUT_VB="${OUTDIR}/vendor_boot-test.img"
HOLD="${HOLD:-0}"

mkdir -p "${OUTDIR}"

if [[ ! -f "${STOCK_VB}" ]]; then
  echo "missing ${STOCK_VB}" >&2
  exit 1
fi
if [[ ! -f "${INITSRC}/aginxos-init" ]]; then
  echo "missing ${INITSRC}/aginxos-init" >&2
  exit 1
fi
if [[ ! -f "${TOOLS}/mkbootimg.py" ]]; then
  echo "run ./boot/fetch-tools.sh first" >&2
  exit 1
fi

echo "==> unpack stock vendor_boot"
rm -rf "${UNPACK_VB}"
mkdir -p "${UNPACK_VB}"
python3 "${TOOLS}/unpack_bootimg.py" --boot_img "${STOCK_VB}" --out "${UNPACK_VB}" --format=info \
  | tee "${UNPACK_VB}/info.txt"

VR="${UNPACK_VB}/vendor_ramdisk"
DTB="${UNPACK_VB}/dtb"
test -f "${VR}"
test -f "${DTB}"

echo "==> extract vendor ramdisk"
rm -rf "${WORK}"
mkdir -p "${WORK}"
lz4 -dc "${VR}" | (cd "${WORK}" && cpio -idm)

echo "==> inject aginxos-init (rdinit + /init overwrite)"
if [[ ! -f "${WORK}/system/bin/init" ]]; then
  echo "vendor ramdisk missing /system/bin/init" >&2
  exit 1
fi
# Preserve Android init for handoff
cp -f "${WORK}/system/bin/init" "${WORK}/system/bin/init.android"
chmod 755 "${WORK}/system/bin/init.android"
rm -f "${WORK}/init.android"
cp -f "${WORK}/system/bin/init.android" "${WORK}/init.android"

mkdir -p "${WORK}/aginxos"

# C trampoline is the rdinit entry (Rust handoff is unreliable on redfin).
TRAMP="${ROOT}/boot/initramfs/trampoline"
if [[ ! -x "${TRAMP}" ]]; then
  echo "==> build C trampoline"
  zig cc -target aarch64-linux-musl -static -O2 \
    -o "${TRAMP}" "${ROOT}/boot/trampoline/trampoline.c"
fi
cp -f "${TRAMP}" "${WORK}/aginxos/trampoline"
chmod 755 "${WORK}/aginxos/trampoline"

# Optional Rust helper (splash-test child)
if [[ -f "${INITSRC}/aginxos-init" ]]; then
  cp -f "${INITSRC}/aginxos-init" "${WORK}/aginxos/aginxos-init"
  chmod 755 "${WORK}/aginxos/aginxos-init"
fi

# Static first-stage init from stock boot.img
if [[ -f "${INITSRC}/first_stage_init" ]]; then
  cp -f "${INITSRC}/first_stage_init" "${WORK}/aginxos/first_stage_init"
  chmod 755 "${WORK}/aginxos/first_stage_init"
  echo "note: included first_stage_init"
else
  echo "error: missing boot/initramfs/first_stage_init" >&2
  exit 1
fi

if [[ -f "${INITSRC}/aginxos-probe" ]]; then
  cp -f "${INITSRC}/aginxos-probe" "${WORK}/aginxos/aginxos-probe"
  chmod 755 "${WORK}/aginxos/aginxos-probe"
fi

# Feature flags (empty files). Safe default: HOLD only, no modules/splash.
HOLD="${HOLD:-0}"
SPLASH="${SPLASH:-0}"
# MODULES: 0 | 1 (small allow-list) | drm (stock modules.load through msm_drm)
MODULES="${MODULES:-0}"
MODULES_FULL="${MODULES_FULL:-0}"
if [[ "${HOLD}" == "1" ]]; then
  : >"${WORK}/aginxos/hold"
  echo "note: HOLD=1"
fi
if [[ "${SPLASH}" == "1" ]]; then
  : >"${WORK}/aginxos/splash"
  echo "note: SPLASH=1 (DRM paint; needs MODULES=drm or MODULES=1)"
fi
if [[ "${MODULES}" == "drm" || "${MODULES}" == "DRM" ]]; then
  # Preferred: same order Android first_stage uses, stop at msm_drm.ko
  : >"${WORK}/aginxos/load-modules-loadfile"
  echo "note: MODULES=drm → load /lib/modules/modules.load through msm_drm.ko"
elif [[ "${MODULES}" == "1" ]]; then
  : >"${WORK}/aginxos/load-modules"
  cat >"${WORK}/aginxos/modules.allow" <<'EOF'
# pinctrl / clocks / bus
pinctrl-msm.ko
pinctrl-spmi-gpio.ko
pinctrl-spmi-mpp.ko
pinctrl-lito.ko
msm_bus.ko
clk-qcom.ko
clk-aop-qmp.ko
cmd-db.ko
msm_ipc_logging.ko
qcom_rpmh.ko
clk-rpmh.ko
dispcc-lito.ko
gcc-lito.ko
llcc-slice.ko
llcc-lito.ko
# memory / iommu / ion / tz
qtee_shm_bridge.ko
secure_buffer.ko
msm_dma_iommu_mapping.ko
ion-alloc.ko
msm_bus_rpmh.ko
iommu-logger.ko
arm-smmu-debug.ko
arm-smmu.ko
# regulators / i2c / panel power
regmap-spmi.ko
qcom-spmi-pmic.ko
qcom-i2c-pmic.ko
qpnp-amoled-regulator.ko
rpmh-regulator.ko
qcom-geni-se.ko
i2c-qcom-geni.ko
# display + drm
fsa4480-i2c.ko
msm_ext_display.ko
qseecom.ko
hdcp_qseecom.ko
msm_hdcp.ko
msm_drm.ko
EOF
  echo "note: MODULES=1 wrote small display modules.allow"
fi
if [[ "${MODULES_FULL}" == "1" ]]; then
  : >"${WORK}/aginxos/load-modules-full"
  echo "note: MODULES_FULL=1 loads entire modules.load (RISKY)"
fi
# USBADB=1: ffs.adb gadget console (adbd is in this ramdisk already; see docs/HARDWARE.md)
# USBDIAG=1: same module chain, but diagnostics instead of gadget — extcon +
#            deferred-probe dumps, drivers_probe replay kick, then Android
#            handoff so the full kernel log can be read via adb+root dmesg.
USBADB="${USBADB:-0}"
USBDIAG="${USBDIAG:-0}"
# USBNOBIND=1 (with USBADB=1): full gadget setup but skip the final UDC bind,
# then hand off - bisects bind vs pre-bind stages and preserves the run's
# kmsg for reading from booted Android. Log-collection mode, not console mode.
USBNOBIND="${USBNOBIND:-0}"
if [[ "${USBADB}" == "1" || "${USBDIAG}" == "1" ]]; then
  cat >"${WORK}/aginxos/modules.usb" <<'EOF'
# USB gadget console chain. Raw finit_module in listed order.
# ROOT CAUSE (found 2026-08-26 via USBDIAG + bugreport kernel log): the chain
# must be a true modules.dep topological order. Two prior orderings failed:
#   - eud.ko needs qtee_shm_bridge.ko (exports scm_io_read/write) BEFORE it
#   - qpnp_pdphy.ko needs usb-dwc3-msm.ko (exports ext_vbus_register_notify)
#     BEFORE it — dwc3 defers until pdphy's extcon registers, and pdphy's
#     module load is exactly what replays the deferred probe (stock does the
#     same: dwc3-msm loads 1.25s, pdphy 1.30s, dwc3 probe completes 1.63s).
# This order is machine-validated against modules.dep (all edges satisfied).
# NOTE: rmmod eud panics this kernel — load-only, never unload.
# Foundation — clocks, power, pinctrl, bus
msm_ipc_logging.ko
msm_bus.ko
pinctrl-msm.ko
pinctrl-lito.ko
pinctrl-spmi-gpio.ko
pinctrl-spmi-mpp.ko
cmd-db.ko
smem.ko
qcom_rpmh.ko
clk-rpmh.ko
clk-aop-qmp.ko
clk-qcom.ko
gcc-lito.ko
qcom-pdc.ko
msm_bus_rpmh.ko
refgen.ko
spmi-pmic-arb.ko
regmap-spmi.ko
qcom-spmi-pmic.ko
rpmh-regulator.ko
fsa4480-i2c.ko
# qtee + IOMMU/SMMU chain (qtee MUST precede eud: it exports scm_io_*)
qtee_shm_bridge.ko
iommu-logger.ko
secure_buffer.ko
arm-smmu-debug.ko
arm-smmu.ko
msm_dma_iommu_mapping.ko
# extcon supplier: qpnp-smb5 (charger)
# DT-level suppliers smb5 probe waits on (dtbo analysis 2026-08-26):
#   io-channels = pm7250b_vadc ("qcom,spmi-adc5")     -> adc5 + vadc-common
#   ext-vbus-supply = ext_boost ("regulator-tps")     -> tps-regulator.ko
# pdphy's connector node also consumes a vadc channel, and pdphy waits on
# smb5-vbus/vconn (smb5 child regulators) + ext_boost.
# SOURCE-level supplier (qpnp-smb5.c, verified vs android-msm-redbull-4.19):
#   smb5_probe() returns -EPROBE_DEFER SILENTLY unless alarmtimer_get_rtcdev()
#   is non-NULL -> needs rtc-pm8xxx (pm8150_rtc). Confirmed on device
#   2026-08-26: rtc-pm8xxx load at 21.503s -> "logbuffer: id:smblib
#   registered" 1.7ms later -> smb5 probe success at 21.522s.
# pdphy's usbpd_create -517 also unblocks with smb5: it defers on
# power_supply_get_by_name("usb") and find_votable("USB_ICL"), both created
# by smb5_probe. Chain: rtc -> smb5 -> pdphy -> ssphy/ssusb -> dwc3 -> UDC.
# pdphy's usbpd_create also needs the "wireless" power supply (DT has
# goog,wlc-supported): registered by p9221_charger (Qi RX, i2c 1-003b on
# geni i2c). Verified on device 2026-08-26: p9221 "id:wireless registered"
# at 21.4553s -> pdphy usbpd_create OK 2ms later. p9221 probe prints some
# benign errors (pin group 99, one i2c -107) — stock logs the same.
# The geni i2c controller devices themselves defer on their GPI DMA
# supplier (900000.qcom,gpi-dma) — virt-dma + gpi must come first, else
# the buses (and p9221 on 98c000.i2c) only probe on Android's wave.
virt-dma.ko
gpi.ko
qcom-geni-se.ko
i2c-qcom-geni.ko
qcom-vadc-common.ko
qpnp-revid.ko
qcom-spmi-adc5.ko
tps-regulator.ko
rtc-pm8xxx.ko
pmic-voter.ko
logbuffer.ko
p9221_charger.ko
qpnp-battery.ko
of_batterydata.ko
qpnp-smb5-charger.ko
# SCM + extcon supplier: msm-eud (load-only; rmmod panics)
msm_scm.ko
eud.ko
# dwc3 controller — loads BEFORE pdphy: pdphy needs its ext_vbus_* exports,
# and pdphy's later registration is the deferred-probe replay dwc3 waits for
dwc3.ko
usb-dwc3-msm.ko
# PHYs
phy-generic.ko
phy-msm-ssusb-qmp.ko
phy-msm-snps-hs.ko
# typec + extcon supplier: usb-pdphy (LAST — depends on usb-dwc3-msm)
roles.ko
tcpm.ko
qpnp_pdphy.ko
EOF
fi
if [[ "${USBADB}" == "1" ]]; then
  : >"${WORK}/aginxos/usb-adb"
  echo "note: USBADB=1 → ffs.adb console (first test with HOLD=1)"
fi
# USBCFGONLY=1: mount configfs, create no gadget tree (bisect v13)
USBCFGONLY="${USBCFGONLY:-0}"
# USBG1ONLY=1: create /config/usb_gadget/g1 only, nothing else (bisect v14)
USBG1ONLY="${USBG1ONLY:-0}"
# USBPROPSONLY=1: g1 + property writes, no functions/configs (bisect v15)
USBPROPSONLY="${USBPROPSONLY:-0}"
# USBVIDPIDONLY=1: g1 + idVendor/idProduct writes only (bisect v16)
USBVIDPIDONLY="${USBVIDPIDONLY:-0}"
# USBMKG1ONLY=1: mkdir usb_gadget/g1 only, no writes (bisect v17)
USBMKG1ONLY="${USBMKG1ONLY:-0}"
# USBNOG1=1: usb_gadget dir only, never mkdir g1 (control, bisect v18)
USBNOG1="${USBNOG1:-0}"
# USBNOCLEANUP=1: build gadget, skip teardown before handoff (bisect v19)
USBNOCLEANUP="${USBNOCLEANUP:-0}"
# USBNOMODS=1: usb_console with NO module load, straight to configfs g1 (bisect v21)
USBNOMODS="${USBNOMODS:-0}"
if [[ "${USBNOMODS}" == "1" ]]; then
  : >"${WORK}/aginxos/usb-nomods"
  echo "note: USBNOMODS=1 -> no modules, mkdir g1 only (bisect v21)"
fi
if [[ "${USBNOCLEANUP}" == "1" ]]; then
  : >"${WORK}/aginxos/usb-nocleanup"
  echo "note: USBNOCLEANUP=1 -> no gadget teardown before handoff (bisect v19)"
fi
if [[ "${USBNOG1}" == "1" ]]; then
  : >"${WORK}/aginxos/usb-nog1"
  echo "note: USBNOG1=1 -> usb_gadget dir only, no g1 (control v18)"
fi
if [[ "${USBMKG1ONLY}" == "1" ]]; then
  : >"${WORK}/aginxos/usb-mkg1-only"
  echo "note: USBMKG1ONLY=1 -> mkdir g1 only, no prop writes (bisect v17)"
fi
if [[ "${USBVIDPIDONLY}" == "1" ]]; then
  : >"${WORK}/aginxos/usb-vidpid-only"
  echo "note: USBVIDPIDONLY=1 -> g1 + vid/pid only (bisect v16)"
fi
if [[ "${USBPROPSONLY}" == "1" ]]; then
  : >"${WORK}/aginxos/usb-props-only"
  echo "note: USBPROPSONLY=1 -> g1 + props only (bisect v15)"
fi
if [[ "${USBG1ONLY}" == "1" ]]; then
  : >"${WORK}/aginxos/usb-g1-only"
  echo "note: USBG1ONLY=1 -> g1 dir only (bisect v14)"
fi
if [[ "${USBCFGONLY}" == "1" ]]; then
  : >"${WORK}/aginxos/usb-configfs-only"
  echo "note: USBCFGONLY=1 -> configfs mount only, no gadget tree (bisect v13)"
fi
if [[ "${USBNOBIND}" == "1" ]]; then
  : >"${WORK}/aginxos/usb-nobind"
  echo "note: USBNOBIND=1 -> gadget setup, no UDC bind, Android handoff (log-collection mode)"
fi
# USBNOBIND=1: skip final UDC bind (log-collection bisect)
# USBNOFFS=1: stop after configfs tree, skip ffs mount + adbd (bisect v12)
USBNOBIND="${USBNOBIND:-0}"
USBNOFFS="${USBNOFFS:-0}"
if [[ "${USBNOFFS}" == "1" ]]; then
  : >"${WORK}/aginxos/usb-noffs"
  echo "note: USBNOFFS=1 -> configfs tree only, no ffs/adbd/bind (bisect)"
fi
if [[ "${USBNOBIND}" == "1" ]]; then
  : >"${WORK}/aginxos/usb-nobind"
  echo "note: USBNOBIND=1 -> gadget setup, no UDC bind, Android handoff (log-collection mode)"
fi
if [[ "${USBDIAG}" == "1" ]]; then
  : >"${WORK}/aginxos/usb-diag"
  echo "note: USBDIAG=1 → module load + extcon/deferred dumps + drivers_probe kick, then handoff"
fi
# USBPROBE=1: load modules + check UDC, then HOLD + paint the verdict on
# screen (green = UDC appeared, red = no UDC). ramoops is dead on this unit,
# so in HOLD mode the screen is the only observable channel.
USBPROBE="${USBPROBE:-0}"
if [[ "${USBPROBE}" == "1" ]]; then
  : >"${WORK}/aginxos/usb-probe"
  echo "note: USBPROBE=1 → UDC probe, HOLD + screen verdict (green/red)"
fi

echo "==> repack vendor ramdisk (lz4 -l)"
VRD_OUT="${OUTDIR}/vendor_ramdisk.lz4"
(
  cd "${WORK}"
  find . -print0 | cpio --null --create --format=newc 2>/dev/null | lz4 -l -12 >"${VRD_OUT}" \
    || find . | cpio -o -H newc | lz4 -l -12 >"${VRD_OUT}"
)
echo "vendor ramdisk $(wc -c <"${VRD_OUT}") bytes"

# cmdline from unpack pretty-info
VCMD=$(python3 - <<'PY' "${UNPACK_VB}/info.txt"
import sys,re
text=open(sys.argv[1]).read()
# format: vendor command line args: ...
for line in text.splitlines():
    if "command line" in line.lower() and ":" in line:
        print(line.split(":",1)[1].strip())
        break
PY
)
if [[ -z "${VCMD}" ]]; then
  VCMD="console=ttyMSM0,115200n8 androidboot.console=ttyMSM0 androidboot.hardware=redfin"
fi
# Proven entry on redfin: C trampoline → first_stage_init → Android
if [[ "${VCMD}" != *rdinit=* ]]; then
  VCMD="${VCMD} rdinit=/aginxos/trampoline"
fi
echo "vendor_cmdline: ${VCMD:0:100}..."

echo "==> mkbootimg vendor_boot → ${OUT_VB}"
python3 "${TOOLS}/mkbootimg.py" \
  --header_version 3 \
  --pagesize 4096 \
  --base 0x00000000 \
  --kernel_offset 0x00008000 \
  --ramdisk_offset 0x01000000 \
  --tags_offset 0x00000100 \
  --dtb "${DTB}" \
  --dtb_offset 0x01f00000 \
  --vendor_cmdline "${VCMD}" \
  --vendor_ramdisk "${VRD_OUT}" \
  --vendor_boot "${OUT_VB}"

ls -lh "${OUT_VB}"
file "${OUT_VB}"
echo
echo "Flash temporary (restore later with stock-vendor_boot.img):"
echo "  fastboot flash vendor_boot ${OUT_VB}"
echo "  fastboot reboot"
echo "Restore:"
echo "  fastboot flash vendor_boot ${STOCK_VB} && fastboot reboot"
