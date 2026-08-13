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
cp -f "${INITSRC}/aginxos-init" "${WORK}/aginxos/aginxos-init"
chmod 755 "${WORK}/aginxos/aginxos-init"
# Also replace /init (in case rdinit is ignored)
rm -f "${WORK}/init"
cp -f "${INITSRC}/aginxos-init" "${WORK}/init"
chmod 755 "${WORK}/init"

if [[ -f "${INITSRC}/aginxos-probe" ]]; then
  cp -f "${INITSRC}/aginxos-probe" "${WORK}/aginxos/aginxos-probe"
  chmod 755 "${WORK}/aginxos/aginxos-probe"
fi

# Feature flags (empty files). Safe default: HOLD only, no modules/splash.
HOLD="${HOLD:-0}"
SPLASH="${SPLASH:-0}"
MODULES="${MODULES:-0}"
MODULES_FULL="${MODULES_FULL:-0}"
if [[ "${HOLD}" == "1" ]]; then
  : >"${WORK}/aginxos/hold"
  echo "note: HOLD=1"
fi
if [[ "${SPLASH}" == "1" ]]; then
  : >"${WORK}/aginxos/splash"
  echo "note: SPLASH=1"
fi
if [[ "${MODULES}" == "1" ]]; then
  : >"${WORK}/aginxos/load-modules"
  echo "note: MODULES=1 (safe allow-list only)"
fi
if [[ "${MODULES_FULL}" == "1" ]]; then
  : >"${WORK}/aginxos/load-modules-full"
  echo "note: MODULES_FULL=1 (RISKY)"
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
# Force kernel to run our binary as early userspace init (survives ramdisk merge order).
if [[ "${VCMD}" != *rdinit=* ]]; then
  VCMD="${VCMD} rdinit=/aginxos/aginxos-init"
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
