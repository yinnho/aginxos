# AginxOS — Pixel 5 (redfin) hardware log

Fill this in during bring-up. Do not invent nodes; only record what the device shows.

## Device

| Field | Value |
|-------|--------|
| Codename | redfin |
| SoC | SM7250 (Snapdragon 765G) |
| Bootloader | unlocked |
| Role | experimental |
| Serial | 13201FDD4001N8 |
| Current slot | a |
| Stock / last known good | factory `UP1A.231105.001.B2` (Android 14) |

## Probe output

```
AginxOS probe 0.1.0
uid=2000
kernel: Linux version 4.19.278-g7b0944645172-ab10812814 ... #1 SMP PREEMPT Thu Sep 7 05:43:03 UTC 2023
hostname: <unavailable: Permission denied (os error 13)>
input: /dev/input → event0, event1, event2, event3
dri: /dev/dri → card0, renderD128
```

Captured: 2026-08-13 via `./scripts/push-probe-android.sh` on stock Android userspace.

## Nodes

| Subsystem | Path / notes | Status |
|-----------|----------------|--------|
| Input | `/dev/input/event0`–`event3` | present (shell uid 2000) |
| DRM | `/dev/dri/card0`, `renderD128` | present |
| WLAN | | not probed yet |
| Modem / QRTR | | not probed yet |
| Firmware dir | extract from vendor locally | not yet |

## Boot experiments

| Date | boot.img | Result |
|------|----------|--------|
| 2026-08-13 | stock factory flash complete | Android boots; adb OK |
| 2026-08-13 | minimal ramdisk (`aginxos-init` only) | bootloader OK; black Google logo hang (no modules/display path). Recover via force reboot + stock boot flash. |
| 2026-08-13 | hybrid ramdisk (stock + wrap `/init` → DRM splash → `/init.android`) | `fastboot boot` sent OKAY; expect green splash ~4s then Android. Confirm on device. |

## Boot path finding (2026-08-13)

Pixel 5 loads **boot.img ramdisk + vendor_boot ramdisk**. Vendor entries overwrite boot for the same path, so patching only `boot.img` `/init` has no effect (`vendor` has `init -> /system/bin/init`).

Working approach:
- Patch `vendor_boot` ramdisk
- Set `rdinit=/aginxos/aginxos-init` on vendor cmdline
- Place binary at `/aginxos/aginxos-init`
- `HOLD=1` with `/aginxos/hold` confirmed: no Android for 70s+ (our pid1 holds)

Restore: `./scripts/restore-vendor-boot.sh`

## Progressive bring-up (2026-08-13 cont.)

| Experiment | Result |
|------------|--------|
| modules.load full insmod in init | **bootloop** |
| handoff to Android init after splash | **bootloop** |
| minimal HOLD (`rdinit`, mount only, no modules/splash) | **stable 60s+, no Android** |
| Feature flags in ramdisk | `/aginxos/hold`, `/aginxos/splash`, `/aginxos/load-modules` |

Next: handoff-only (no splash/modules), then one module at a time.

## Handoff breakthrough (2026-08-13)

| Approach | Result |
|----------|--------|
| `rdinit=/aginxos/aginxos-init` then Rust `execve` first_stage | **FAIL** (stuck / no Android) |
| `rdinit=/aginxos/first_stage_init` | **OK** Android boots |
| `rdinit=/aginxos/trampoline` (C) → `execve` first_stage | **OK** Android boots |
| Full `modules.load` in early init | **bootloop** |
| Minimal HOLD (mount+loop) | **OK** stable |

Production entry on redfin: **C trampoline** as `rdinit`, then stock first-stage init.
Rust `aginxos-init` remains for optional splash-test child / future work.

## Early splash (2026-08-13 cont.)

| Experiment | Result |
|------------|--------|
| SPLASH+incomplete `modules.allow` (no qseecom deps) | **bootloop** |
| SPLASH only, no modules | handoff path safe; paint usually fails (no `card0`/mode) |
| HOLD+SPLASH+full display dep list (36 kos incl. qseecom/msm_drm) | **HOLD stable 120s+, no bootloop** — confirm colors on device |

Flags: `/aginxos/hold`, `/aginxos/splash`, `/aginxos/load-modules`, `/aginxos/modules.allow`.

Expect (if DRM OK): green → red → blue → yellow full frames with thick white border (~3s each), then hold last frame.

## Early splash follow-up (2026-08-13 evening)

| Experiment | Result |
|------------|--------|
| HOLD+SPLASH+36 display kos | **HOLD stable**; screen stayed on **Google logo** (no modeset) |
| SPLASH+`modules.load` through `msm_drm` then handoff | **bootloop** → stock `vendor_boot` restore |
| SPLASH+36 kos then handoff (even after `delete_module`) | **bootloop** |
| Vendor.img inject `/vendor/bin/aginxos-splash` + new `.rc` | **recovery** (likely unlabeled / verity) |
| Same + SELinux xattrs + hook in `init.pixel.rc` | **recovery** again; stock vendor restore OK |
| SPLASH no modules + `/dev/mem` mmap `cont_splash_region@0xa0000000` | **bootloop** |
| Stock vendor + stock vendor_boot | **Android OK** |

DT has `/reserved-memory/cont_splash_region` at `0xa0000000` size `0x2300000`. Userspace cannot safely paint it on this kernel (`/dev/mem` missing/unsafe). Early `msm_drm` load from ramdisk does not produce a usable CRTC and poisons first_stage handoff.

Working signal that *our* pid1 ran: **HOLD** (stuck Google logo, no USB). Color needs either a rebuilt kernel (simpledrm / built-in msm + `/dev/fb`) or a first_stage-after hook that does not rewrite `vendor`/`vbmeta`.

## USB gadget recon (2026-08-26)

Source: unpacked `boot/stock-vendor_boot.img` vendor ramdisk (`modules.load`,
`modules.dep`, `system/etc/init/hw/init.rc`, `init.recovery.redfin.rc`,
`prop.default`, `system/bin/adbd`, `system/lib64/*`). All facts below are from
those files, not assumptions.

**What exists on redfin's downstream kernel:**

| Component | Status | Evidence |
|-----------|--------|----------|
| dwc3 controller | module chain, loads fine in stock | vendor cmdline `androidboot.usbcontroller=a600000.dwc3`; init.recovery waits on `/sys/devices/platform/soc/a600000.ssusb/a600000.dwc3/driver` |
| configfs | **built-in** | init.rc `sys.usb.configfs 1` path mounts and configures `/config/usb_gadget/g1` |
| ffs (FunctionFS) | **built-in** | init.rc creates `functions/ffs.adb`, mounts `functionfs adb /dev/usb-ffs/adb` |
| rndis | module `rndis.ko`, **zero deps** (self-contained) | `modules.dep`: empty dependency list |
| adbd + bionic runtime | **inside the vendor ramdisk** | `system/bin/adbd`, `system/bin/linker64`, `system/lib64/` with all 10 of adbd's DT_NEEDED libs (libadbd_fs, libadbd_auth, libcrypto, libc++, libbase, liblog, libselinux, libc, libm, libdl) |
| **acm (serial gadget)** | **absent** | no `usb_f_acm.ko`, never referenced in any init rc → original "acm first" plan is dead on this kernel |
| usb_f_ncm / ecm | absent | only rndis among network functions |

**USB IDs (stock recovery, prop.default):** vid `0x18D1`, adb pid `0xD001`,
fastboot pid `0x4EE0`.

**Gadget module chain (topological order per `modules.dep`):**

> Superseded same day — see "First USBADB attempt + recovery": this list omits
> the clock/power foundation the probes need at runtime.

```text
msm_ipc_logging.ko → msm_bus.ko → logbuffer.ko → roles.ko → tcpm.ko
→ pmic-voter.ko → phy-generic.ko → phy-msm-ssusb-qmp.ko → phy-msm-snps-hs.ko
→ dwc3.ko → usb-dwc3-msm.ko → qpnp_pdphy.ko
```

USB modules failing to load cannot poison the system (unlike the display set) —
worst case is "no UDC appears in /sys/class/udc".

**Gadget config sequence to mirror** (from init.rc configfs path): mount
configfs at `/config` → mkdir `usb_gadget/g1` → write idVendor/idProduct →
strings/0x409 → mkdir `functions/ffs.adb` → `configs/b.1` → symlink function
into config → mount functionfs at `/dev/usb-ffs/adb` (`uid=2000,gid=2000`) →
start adbd → write UDC name (from `/sys/class/udc/*`) to `g1/UDC`.

**Host-side note:** macOS (≥12.3) has no RNDIS driver — RNDIS is only useful
with a Linux host. `adb` is the correct console transport on macOS.

Next experiment: trampoline `/aginxos/usb-adb` flag + `modules.usb` list
(ROADMAP Phase 0). First run **with HOLD=1**.

## First USBADB attempt + recovery (2026-08-26)

**Observed (not yet root-caused at flash time):**

- Flashed `vendor_boot-test.img` (HOLD=1 SPLASH=0 MODULES=0 USBADB=1, slot a),
  rebooted → device never enumerated on adb/fastboot; host-side USB bus showed
  nothing. Device was recovered via long-press to fastboot.
- Recovery itself surfaced two **host-side confounders** that had been
  misread as device failures:
  1. **Stale `fastboot` processes on the macOS host hang while holding the
     device open** — subsequent `fastboot devices` then returns empty even
     though ioreg shows `Pixel 5` @ vid `0x18d1` pid `0x4ee0`. Kill stale
     fastboot pids before concluding "device gone".
  2. **The VIA Labs USB-C dock corrupts fastboot traffic**: through the dock,
     every fastboot command (`getvar`, `flash`) hung; direct to the Mac's own
     port, the same commands completed in ~5s. All future flashing is direct.
- After restoring stock `vendor_boot`+`boot` to **both** slots and setting
  active=a, device boots normal stock Android (adb works after re-enabling
  USB debugging in Settings — the restore wiped authorization).

**Root cause of no-UDC (from static analysis of the same ramdisk):**

The first `modules.usb` chain listed only the 12 dwc3/PD modules — it omitted
the clock/power foundation entirely. Stock's `modules.load` loads these BEFORE
any USB module:

```text
qcom-pdc.ko pinctrl-msm.ko pinctrl-lito.ko pinctrl-spmi-{gpio,mpp}.ko
clk-qcom.ko clk-rpmh.ko clk-aop-qmp.ko gcc-lito.ko cmd-db.ko smem.ko
qcom_rpmh.ko msm_bus_rpmh.ko rpmh-regulator.ko regmap-spmi.ko qcom-spmi-pmic.ko
```

Evidence this matters: `gcc-lito.ko` contains the `gcc_usb30_*` clock strings;
`dwc3.ko` / `phy-msm-snps-hs.ko` / `phy-msm-ssusb-qmp.ko` reference
`usb2_phy`/`usb3_phy` supplies. These dependencies are **runtime**
(clk_get/regulator_get against DT nodes), not symbol imports, so they do NOT
appear in `modules.dep` — a topologically-correct-per-modules.dep order of just
the 12 dwc3 modules still probes with no clocks and defers forever.

Also verified on-device (stock Android):
- UDC driver is `msm-dwc3` (= `usb-dwc3-msm.ko`), NOT `dwc3-qcom`.
- `/proc/modules` shows `dwc3_qcom`, `dwc3_of_simple`, `dwc3_haps` all loaded
  with refcount **0** — upstream-style glue that binds nothing on redfin.
  Omitting them from our chain is correct, not a gap.
- `CONFIG_USB_F_FS=y`, `CONFIG_USB_CONFIGFS=y` (from `/proc/config.gz`) — ffs
  built-in confirmed.

Fix applied: `pack-vendor-boot.sh` now writes a 28-module `modules.usb`
(foundation first, then Type-C/PD/PHY/controller), topological per
modules.dep. Not yet flashed.

## Second attempt + Magisk root (2026-08-26)

**Second USBADB attempt (37 modules):** added the SMMU/IOMMU chain
(`qtee_shm_bridge` → `iommu-logger`/`secure_buffer`/`arm-smmu-debug` →
`arm-smmu` + `msm_dma_iommu_mapping`), plus `refgen`/`spmi-pmic-arb`/
`fsa4480-i2c`, since stock loads all of these before the first USB module
and the ssusb DT node carries `iommus`. Chain verified: 37 modules,
topological per modules.dep, zero undefined-symbol gaps. **Still no USB
enumeration** (host saw no device; earlier "pixel on USB" during polling was
a false positive — the macOS `ioreg -l` IOKitDiagnostics blob contains the
substring "Pixel" even with no phone attached). Device stayed in HOLD (Google
logo, no panic). Root cause still a runtime probe defer, not visible without
a log channel.

**Why this kept failing blind:** the trampoline logs to `/dev/kmsg`, but on a
`user` build there is no root — `dmesg`, `/proc/last_kmsg`, and
`/sys/fs/pstore/` are all permission-denied, so every failed experiment was
unobservable. **Resolved by installing Magisk:**

- Bootloader unlocked (`fastboot getvar unlocked` = yes).
- Downloaded Magisk v28.1 APK, extracted `boot_patch.sh` + arm64 native libs
  (`magiskboot`/`magiskinit`/`magisk`/`init-ld`/`busybox`), ran the patch
  on-device (`/data/local/tmp/magisk_patch/boot_patch.sh stock-boot.img`),
  flashed `new-boot.img` to `boot_a`.
- `su` was initially rejected: Magisk default "Superuser Access" = Apps-only
  records an explicit **deny** for uid 2000 (shell) that then short-circuits
  future prompts (`logcat -s Magisk` showed `su: request rejected (2000)`).
  Fix: Settings → "Superuser Access" = **Apps and ADB**, then clear the stale
  shell deny.
- Result: `adb shell su -c id` → `uid=0(root)`; `dmesg` readable.
- ramoops confirmed present: `/proc/device-tree/reserved-memory/ramoops_region@B7E41000`
  (+ `alt_ramoops_region`, `ramoops_meta_region`), `pstore`/`ramoops` modules
  loaded. A warm reset (long-press) preserves the previous boot's console in
  `/sys/fs/pstore/console-ramoops` → every future failed experiment is now
  debuggable.

**Session end state:** stock `vendor_boot` + **Magisk-patched `boot`** on
slot a (boot_b still stock), active=a, Android 14 booted, adb + root working.

## Third attempt + probe methodology (2026-08-26 evening)

**Why attempt 2 failed — extcon suppliers (found via rooted stock dmesg):**

The ssusb DT node (`/soc/ssusb@a600000`) carries an `extcon` property with
**three phandles**: `usb-pdphy@1700` (0x51e), `qcom,qpnp-smb5` (0x51f),
`qcom,msm-eud@88e0000` (0x331). `dwc3_msm_probe` resolves all three via
`extcon_get_edev_by_phandle`, which returns `-EPROBE_DEFER` until every
supplier has registered. The 37-module chain had pdphy but was **missing
qpnp-smb5-charger and msm-eud entirely** → dwc3 deferred forever, no UDC.
Stock loads `eud.ko` at modules.load idx 58 and `qpnp-smb5-charger.ko` at
idx 145, both before the dwc3 probe fires (~1.649s in stock boot).

Also learned the hard way: **`rmmod eud` panics this kernel instantly**
(bootreason=kernel_panic, every time). EUD is load-only.

Fix applied: `modules.usb` grew to **42 modules** (+`qpnp-revid`,
+`pmic-voter`/`logbuffer` moved earlier, +`qpnp-battery`, +`of_batterydata`,
+`qpnp-smb5-charger`, +`eud`; `qpnp_pdphy.ko` moved before the PHYs).
Symbol-dep closure verified topological per modules.dep.

**Observability findings for HOLD mode (all confirmed this session):**

- ramoops/pstore is empty after warm reset (long-press) too — earlier note
  that long-press preserves console-ramoops was wrong; pstore is dead on
  this unit for every reset type tested.
- DRM paint from rdinit is impossible: even with the full 36-module display
  chain (2026-08-13) no modeset happens; without it there is no card0 at
  all. Screen-color verdicts are dead.
- LED sysfs writes don't light the physical torch/flash LEDs from raw
  sysfs (needs camera HAL). Vibrator needs the i2c chain (drv2624).
- Working channel: **reboot-timing probe.** `/aginxos/usb-probe` mode now:
  load modules → wait UDC 30s → UDC found = reboot immediately (visible
  restart); timeout = HOLD forever (frozen logo). One flash answers green/red.

**Result of the reboot-probe with the full 42-module chain: VERDICT RED.**
No auto-reboot in 120s; frozen Google logo. Even with all three extcon
suppliers loaded, dwc3 still does not complete probe in our environment.
Remaining suspects (untested): some other runtime dependency outside the
42-module closure (regulators? GDSC vote path? eud needs its TCSR write?),
or module load order relative to device-links. Next diagnostic would need
a real console (USB enumeration is what we're trying to build — chicken and
egg) or bisecting stock's full pre-USB load set (idx < 133).

Slot note: failed boots exhaust retry counts → slot marked unbootable →
device lands in fastboot on every reboot with "Pixel PID 0x4ee0" visible.
`fastboot set_active a` resets the counter; check `slot-unbootable:a`
after any string of failed boots before concluding the flash is bad.

**Session end state (2026-08-26 evening):** stock `vendor_boot` + Magisk
`boot` on slot a, active=a, Android booted, adb + root verified.

## Fourth attempt: intel-driven module additions (2026-08-26 night)

Deep dependency recon on stock (rooted) before flashing anything:

- ssusb's DT device-link suppliers all map into our chain: apps-smmu
  (arm-smmu), GDSC @10f004 (clk-qcom's `gdsc` driver - confirmed via
  kallsyms `gdsc_probe [clk_qcom]`), hsphy/gcc (gcc-lito), ad-hoc-bus
  @16e0000 (msm_bus_rpmh - the `msm_bus_device` driver), eud, smb5, pdphy.
- Regulator map: regulator.2 = usb30_prim_gdsc (enabled, 1 user = dwc3),
  regulator.74 = hsphy, QMP ssphy consumes vdda_usb_ss_dp_core
  (built-in reg-fixed-voltage) + bps_gdsc. All resolvable.
- `msm_scm.ko` (SCM driver, needed by eud's `qcom,secure-eud-en` path)
  was **missing** from our chain despite stock loading it at 0.862s,
  before eud (0.917s) and dwc3 probe (1.649s). modules.load *index* is
  misleading - init loads modules asynchronously; always diff by actual
  dmesg timestamp.
- Successful dwc3 probe signature on stock (checklist for our env):
  `Linked as a consumer to regulator.2` -> `Failed to get clk 'ref': -2`
  (benign, stock prints it too) -> `regulator.74` -> `DWC3 exited from
  low power mode`.

**Probe signal bug found and fixed:** the verdict reboot used cmd
`0x89335757`, which is not any `LINUX_REBOOT_CMD_*` - the syscall
returned EINVAL and GREEN looked identical to RED. Both call sites fixed
to `RB_AUTOBOOT` (0x01234567) / `RESTART2`+arg. **Validated on device**:
a static test binary calling RB_AUTOBOOT reboots the phone (uptime reset,
bootreason=reboot). Consequence: the first 43-module "RED" verdict was
INVALID; re-tested after the fix.

**Valid verdict with fixed signal: RED.** 43-module chain (42 + msm_scm
before eud) still produces no UDC - no auto-reboot in 121s, frozen logo.
H4 (eud-SCM hard-fail as the sole blocker) is refuted or insufficient.
The gap is not in: extcon suppliers, GDSC, clocks, PHYs, SMMU, bus
driver, or SCM. Remaining suspects: deferred-probe replay never firing
in our one-shot load environment, or a subtle runtime ordering/state
issue (rpmh/AOP) invisible without a console.

**Session end state (2026-08-26 night):** stock `vendor_boot` + Magisk
`boot` on slot a, active=a, Android booted, adb + root verified.

## USBDIAG mode + supplier root causes, runs 1-6 (2026-08-26 late)

New tooling: `/aginxos/usb-diag` flag (pack env `USBDIAG=1`). The
trampoline loads the `modules.usb` chain, dumps extcon + regulator state,
kicks `drivers_probe`, then hands off with modules loaded; the kernel ring
buffer survives exec, so the full log is read from booted Android via
`adb bugreport` (KERNEL LOG section). Trampoline boots have no root
(magiskinit is bypassed), so `su -c dmesg` does not work there — bugreport
is the channel. `dump_regulators()` added later: `ext_boost` proves
tps-regulator probed; `smb5-vbus`/`smb5-vconn` only exist after smb5 probed.

Observations across six USBDIAG boots (all: modules ok=N fail=0 skip=0,
Android handoff fine every time, USB modules pre-loaded do NOT poison
first_stage):

- run 1 (43 mods): load-order failures `eud: Unknown symbol scm_io_*`
  (needs qtee_shm_bridge earlier) and `qpnp_pdphy: Unknown symbol
  ext_vbus_*` (needs usb-dwc3-msm earlier). Chain rewritten in
  machine-validated modules.dep topological order.
- run 2 (43): all load; extcon0=eud only; `drivers_probe a600000.ssusb ->
  -22`; Android's own 21.5s module wave then completes smb5/pdphy/dwc3
  (gadget binds 26.5s). No debugfs: mount errno 19 = ENODEV — **debugfs is
  not built into this kernel**; /sys/kernel/debug does not exist and mkdir
  on sysfs is not permitted. devices_deferred is unavailable, period.
- run 3 (46 = +qcom-vadc-common, qcom-spmi-adc5, tps-regulator): from
  dtbo analysis (dtbo table split + dtc): smb5's `io-channels` point at
  `pm7250b_vadc` (qcom,spmi-adc5) and `ext-vbus-supply` at `ext_boost`
  (compatible "regulator-tps" = tps-regulator.ko). Verified on device:
  regulator.70 ext_boost present, smb5 probe starts, still defers
  *silently*.
- Source check (android-msm-redbull-4.19-android13, qpnp-smb5.c):
  `smb5_probe()` returns `-EPROBE_DEFER` with **no log line** unless
  `alarmtimer_get_rtcdev()` is non-NULL. Run 3 timeline matched exactly:
  init loads rtc-pm8xxx 21.503s -> `logbuffer: id:smblib registered`
  1.7ms later -> smb5 success 21.522s.
- run 4 (47 = +rtc-pm8xxx): **extcon1 = smb5 registers in our env**
  (0.897s). pdphy still fails `usbpd_create failed: -517` in a tight
  retry loop.
- pdphy source (pd_engine.c `usbpd_create`): after smb5's "usb" psy it
  waits on `power_supply_get_by_name("wireless")` (DT `goog,wlc-supported`)
  — a logbuffer-only message, invisible in dmesg. Registered by
  p9221_charger (Qi RX, i2c 1-003b on geni bus 98c000.i2c). Run 4: p9221
  loads but the geni i2c controller devices defer on their GPI DMA
  supplier (900000.qcom,gpi-dma) — only Android's wave brings it.
- run 5 (50 = +qcom-geni-se, i2c-qcom-geni, p9221_charger): i2c buses
  still deferred (gpi-dma absent). Same -517 loop.
- run 6 (52 = +virt-dma, gpi): **GREEN.** Chain completes fully in the
  rdinit environment: i2c_geni 98c000 up 0.905s -> p9221 probe 0.929s
  (same benign error lines as stock) -> `id:wireless registered` 0.931s
  -> pdphy usbpd/tcpm 0.980s -> ssusb/dwc3 probe -> ssphy 0.990s ->
  **`udc=a600000.dwc3 PROBE VERDICT GREEN` at 1.069s** (105ms after the
  last module load).

Final supplier chain (52-module `modules.usb`, order machine-validated
against modules.dep): foundation -> virt-dma/gpi -> geni-se/i2c-geni ->
vadc/adc5/tps/rtc-pm8xxx -> p9221_charger -> smb5 suppliers -> dwc3 ->
PHYs -> pdphy. Method notes that paid off: dtbo decompile (not just the
base DTB — all PMIC children live in the dtbo overlay), driver source
reading for silent `-EPROBE_DEFER` paths, and per-run bugreport diffs.
`drivers_probe` write stays -22/EINVAL in our env — irrelevant now;
natural deferred-probe replay works once suppliers exist.

Next: `USBADB=1` (gadget console with the 52-module chain), first with
`HOLD=1`.

**Session end state (2026-08-26 late):** stock `vendor_boot` restored and
flashed (serial 13201FDD4001N8 confirmed before flash), `set_active a`,
Android rebooted. Working tree holds the 52-module chain + diag tooling,
not yet committed.

## USB gadget bisect v6-v24: every configfs touch bootloops (2026-08-27)

Goal of `USBADB=1` was a working ffs.adb console in the rdinit env. It has
never enumerated, and every handoff run that creates anything under
`/config/usb_gadget` bootloops the device. 20 flashes isolated where it
breaks; all runs on serial 13201FDD4001N8.

Matrix (gates are flag files; see trampoline.c comments for exact cut
points; outcome ANDROID_OK = adb shell after handoff, LOOP = slot retries
exhausted to fastboot, FREEZE = silent hold):

| run | config | outcome |
|-----|--------|---------|
| v6-v11 | USBADB variants (full gadget, adbd kill fixes, umount fixes) | LOOP |
| v12 | gadget tree, skip ffs/adbd/bind | LOOP |
| v13 | configfs mount, NO tree | **ANDROID_OK** |
| v14 | gate sat *before* mkdir g1 (flawed) | ANDROID_OK (no g1 made) |
| v15 | g1 + 4 prop writes | LOOP |
| v16 | g1 + idVendor/idProduct only | LOOP |
| v17 | mkdir g1 only, zero writes | LOOP |
| v18 | usb_gadget dir, no g1 (control) | **ANDROID_OK** |
| v19 | full gadget, cleanup skipped | LOOP |
| v20 | UDC "" write guarded by g_bound | LOOP |
| v21 | no modules + gate (flawed: forced no-UDC early return, g1 never created) | uninformative |
| v22/v23 | +3 s settle + extcon dump before configfs | LOOP |
| v24 | HOLD, full gadget + bind, never teardown | FREEZE 9+ min, no host USB, no fastboot |

Readings:

- Minimal repro: 52-module chain loaded + `mkdir /config/usb_gadget/g1` +
  Android handoff = LOOP. configfs mount alone is safe (v13), `usb_gadget`
  dir alone is safe (v18).
- BUT v13 also proves the kernel is not simply "modules + g1 = panic":
  in v13's handoff Android's own init later creates g1, loads ffs, binds
  adbd — USB works, the bugreport that documents all of this came over
  that very link. So *who* creates the gadget, and *when*, matters.
- Failure-domain is unresolved: every LOOP run also ran `cleanup_gadget`
  (rmdir chain) before handoff — creation and teardown paths are both
  candidates; v19 (skip cleanup) LOOPs too, and v24 (HOLD, no teardown,
  no handoff) freezes but HOLD freezes by design, so v24 is NOT panic
  evidence — its only datum is "no enumeration even with nothing pending".
- Kernel panic vs PID-1 userspace death were never separated: no log
  survives either (ring buffer dies on reboot; ramoops dead on this
  unit). "Attempted to kill init" from a fault in our own PID 1 code
  would look identical from outside.
- Disproved along the way: adbd-child-holds-root (v11 SIGKILL+reap),
  UDC-unbind-write panic (v20 guard), probe race (v22/v23 settle),
  leftover mounts (v8 class fix, separate real bug, fixed).

v24 recovery: manual Power+VolDown force-restart to fastboot; stock
`vendor_boot` restored + `set_active a` + Android verified 2026-08-27.

Next (v25, in flight): fork isolation — entire `usb_console()` in a child,
PID 1 waitpids with 180 s timeout and kmsgs exit code/signal, then hands
off. A userspace fault then only kills the child and the run reaches
Android: bugreport shows the last stage marker + the waitpid status.
Clean child + LOOP would finally confirm a kernel-side fault.

## v25 fork isolation: no kernel panic exists; symlink bug found (2026-08-27)

v25 (`USBADB=1` full gadget, whole `usb_console()` in a forked child, PID 1
waitpids and hands off): **ANDROID_OK at 52 s** — first run ever to reach
Android with the gadget path executed. Bugreport kernel log:

- child (pid 200) starts 0.497 s, `modules ok=52` at 0.997 s,
  `udc=a600000.dwc3` at 1.098 s
- extcon2 (pdphy) reads `USB=0` at 1.098 s but **`USB=1` at 4.098 s** —
  VBUS/pull-up conditions turn good during the settle window
- configfs mounted 4.098 s; kernel `file system registered` at 4.099082 s
  (= the functionfs instance created by `mkdir functions/ffs.adb`, proving
  g1 + strings + functions all built fine)
- `ffs symlink errno=2` at 4.099150 s → usb_console early-returns, child
  `exited code=0`, parent runs the FULL cleanup (UDC skip, rmdir chain,
  configfs umount) **in PID 1** with zero trouble, execs first_stage 4.199 s

Findings:

- **There is no kernel panic in the creation or teardown paths.** The exact
  syscalls that bootlooped v6-v24 (mkdir g1, prop writes, ffs instance,
  rmdir gadget) are all harmless — when issued from a non-PID-1 process,
  and the teardown even from PID 1 itself. The v6-v24 "panic" was a
  PID-1-process-domain failure (exact mechanism still unidentified; fork
  isolation sidesteps it completely).
- **The symlink was a real userland bug**: configfs `symlink()` resolves
  the target with kern_path() from the *caller's cwd* (we run at `/`), so
  `"../../functions/ffs.adb"` → `/functions/ffs.adb` → ENOENT. Stock
  init.usb.rc uses absolute source paths. Every run since v6 silently
  aborted the gadget one step before the bind. Fixed: absolute target +
  mkdir/write failures now logged (`mk`/`wf_log` helpers).

v26 (`USBADB=1 HOLD=1` + symlink fix): **still no enumeration** — host saw
no 18d1:d001/d002, no adb, silent freeze. P1 persists with a correctly
built tree. New prime suspect: adbd itself dies in our env (no property
area → adbd exits/crashes before writing ffs ep0 descriptors → composite
bind has no function descriptors → nothing to enumerate). HOLD mode has no
readback channel; manual Power+VolDown recovery needed.

v27 (next): handoff variant of v26 — child also waitpid-watches adbd and
kmsgs exit code/signal; bind result + udc-state land in the ring buffer
and come back via `adb bugreport`.

## v27: P1 localized — adbd aborts before writing ffs descriptors (2026-08-27)

v27 (`USBADB=1` handoff, symlink fixed + `mk`/`wf_log` + adbd waitpid
logging): ANDROID_OK at 42 s. Bugreport kernel log, child pid 202:

- modules ok=52 at 1.005 s, udc at 1.105 s, extcon2/pdphy `USB=1` at
  4.106 s (VBUS good), configfs tree built with **zero mkdir/write errors**
  (symlink accepted — fix confirmed), functionfs instance registered
- **`adbd KILLED signal=6` at 4.157 s** (~50 ms after exec),
  ep1 never appeared
- **`UDC bind FAILED errno=19`** (ENOENT — no function descriptors,
  because nobody wrote them to ffs ep0)
- udc-state `not attached` at 7 s and 29 s; child exited code=0; PID 1
  cleanup + handoff clean

So P1 (no enumeration ever) is fully explained by adbd dying instantly:
gadget tree, ffs, and controller were all ready. Static checks on the
extracted vendor ramdisk: all 10 direct NEEDED libs of adbd present in
/system/lib64, and the full transitive closure is present — not a missing
.so. SIGABRT is either a bionic linker fatal (symbol/version) or adbd's
own `fatal()`; its stderr went to PID 1's empty fd 2 and was lost.

v28 (in flight): adbd child's fd 0/1/2 dup2'd to /dev/kmsg (no O_CLOEXEC)
plus `LD_LIBRARY_PATH=/system/lib64` — bionic/adbd fatal text should land
in the ring buffer verbatim.

## v28-v30: adbd runs, gadget enumerates (2026-08-27)

v28 (adbd stdio dup2'd to /dev/kmsg): adbd still SIGABRT, but the reason
landed in the ring buffer — **`getentropy failed: No such file or
directory`** (pid 273 = adbd). bionic's getentropy() needs /dev/urandom
and fatal()s on ENOENT. This kernel has **no devtmpfs** (stock mounts
tmpfs on /dev and lets ueventd mknod; our try_mount failed silently all
along), so /dev held only the kmsg node the fallback created. Kernel side
of the failed bind also logged: `udc a600000.dwc3: failed to start g1:
-19`.

v29 (ensure_fs logs the devtmpfs errno and mknods null/zero/full/random/
urandom/kmsg — the major-1 char drivers are always registered, only the
nodes were missing): **adbd runs** — `adbd started`, ep0 opened,
`read descriptors`/`read strings`, `UsbFfsConnection constructed`,
`USB event: FUNCTIONFS_BIND`, **`usb gadget BOUND`**, kernel
`android_work: sent uevent USB_STATE=CONNECTED`, `udc-state: addressed`,
`current_speed: high-speed`. Within the fixed 25 s observation window the
host never sent SET_CONFIGURATION (state stayed `addressed`; SPUSB hides
unconfigured devices, so the host-side monitor saw nothing). Non-fatal
noise: linker warning (no /linkerconfig/ld.config.txt), `Failed to get
adbd socket: Operation not permitted`, `uninitialized urandom read`
(no CRNG seed without hwrng).

v30 (same code + HOLD, i.e. unlimited window, no teardown): **full
enumeration — `adb devices` shows `aginxredfin unauthorized`.** macOS
just needed more than 25 s to configure the gadget. adbd is serving; the
RSA auth wall is the only thing left (no property area → adbd defaults
ro.adb.secure=true, and there is no UI to accept the dialog).

v31 (same + adb_keys seeding): first flash **bootlooped to fastboot**;
re-flash of the identical image stayed up (nondeterministic, unexplained —
the seeding runs in the fork-isolated child and cannot reboot the box).
Gadget enumerated, adbd serving, but `adb devices` stayed **unauthorized**
— the seeded keys file was provably ignored (below). One manual
Power+VolDown recovery needed (unauthorized = no adb control from HOLD).

Offline root-cause of the auth wall (2026-08-27, from the ramdisk binaries):
- `ro.adb.secure` appears in **no binary of this build** (adbd, lib64/*).
  Android 14 adbd decides auth via `__android_log_is_debuggable()`
  (liblog → property `ro.debuggable`); no property area → default "0"
  → auth forced on. Older docs blame ro.adb.secure — wrong for A14.
- Disassembly of `/system/lib64/libadbd_auth.so`: the strings
  `/data/misc/adb/adb_keys` and `/adb_keys` have **zero code references**
  (dead constants). This build's adbd_auth gets trusted keys only over the
  framework socket ("received new framework connection"), which never
  exists in the rdinit env. Seeding the file could never work.

v32 (authorized root adb — goal state reached 2026-08-27): stage a real
bionic property area in the rdinit env with two same-length value bytes
patched: `ro.debuggable 0→1` (auth off), `ro.secure 1→0`
(should_drop_privileges() keeps adbd root).
- Source of the area: this device's own live `/dev/__properties__/`,
  pulled while booted normally on stock vendor_boot. **Device is
  Magisk-rooted** (`su` works, context `u:r:magisk:s0`; boot partition is
  Magisk-patched — treat `boot/stock-boot.img` as "known-good restore
  point", its "stock" label unverified). Property files are root-only
  (dir is `--x`).
- Transfer trap: `su -c cat` output crosses a pty with ONLCR — every
  0x0a becomes 0d 0a (+1 byte, wrong md5). `base64` + strip CR/LF is the
  faithful channel (md5-verified against device).
- prop_info layout (reverse-engineered, verified): value is 92 bytes
  before the inline name, 4-byte serial 96 bytes before it, encoded
  little-endian as `(value_len << 24) | count << 16`. Same-length value
  swaps need no serial change.
- Minimal area: 3 files (`properties_serial`, `property_info` = serialized
  contexts trie, `u:object_r:userdebug_or_eng_prop:s0`). Missing context
  areas resolve lazily to "not found" (bionic logs "Access denied finding
  property X" to stderr — harmless, and shows which contexts are absent).
- Validated against real bionic BEFORE flashing: `unshare -m` +
  `mount --make-rprivate /` + tmpfs over `/dev/__properties__` on the
  booted device → `getprop` inside the ns read `debuggable=1 secure=0`.
  (Root mounts are `shared:` — make-rprivate first or the test mount
  propagates system-wide.)
- Flashed `HOLD=1 SPLASH=0 MODULES=0 USBADB=1`. Result: **authorized in
  10 s** (`adb devices` → `aginxosredfin device`), `adb shell id` →
  `uid=0(root)`, `getprop ro.debuggable`→1, `ro.secure`→0, `uname` →
  `Linux (none) 4.19.278-g7b0944645172-ab10812814 aarch64 Toybox`,
  `ps` shows PID 1 = trampoline and no Android processes, uptime 22 s.
  kmsg: 52/52 modules, gadget BOUND at 4.2 s, UDC `configured`
  high-speed at 7.2 s, "staged 3 property area files" + "toybox linked
  as /system/bin/sh" before adbd came up.
- Shell: the vendor ramdisk has no `/system/bin/sh` — the device's own
  `/system/bin/toybox` (deps all present in the ramdisk lib64 set) is
  copied to `boot/out/toybox` (local only) and linked by the trampoline
  as sh + applets (id, ls, cat, getprop, dmesg, uname, ps, ...).

Device state (2026-08-27, end of session): **left running the v32 test
image** (HOLD, authorized root adb console alive). Recovery no longer
needs button combos: `adb reboot bootloader` → flash
`boot/stock-vendor_boot.img` (or re-run v32 via
`USBADB=1 HOLD=1 SPLASH=0 MODULES=0 ./scripts/flash-early-splash.sh`).


## v33: aginxos-init is PID 1; reboot-to-fastboot dead ends mapped (2026-08-27)

**ptmx root cause (pre-v33, live on v32):** from the rdinit console,
`adb reboot bootloader` and interactive `adb shell` fail with
`error: failed to create pty master: No such file or directory` — the
error is *device-side*: adbd cannot open `/dev/ptmx`. The rdinit `/dev`
never had one (tmpfs + mknod'd nodes since v29; devtmpfs won't mount on
this kernel). Live fix (mkdir `/dev/pts`, mount devpts, mknod ptmx) made
one-shot `adb shell` still work — the pty was never the blocker for
those. Permanent fix in v33: `ensure_devpts()` in the trampoline, run
before adbd; kmsg shows `devpts mounted, /dev/ptmx ready`.

**`adb reboot` is dead in the rdinit env (with ptmx present too):** A14
adbd reboots by writing `sys.powerctl=reboot,...` through the init
property service (`libc: Using old property service protocol`); with no
init running, the write goes nowhere and the adb client just hangs
(2-min timeout observed). Staging the property *area* (v32) satisfies
bionic *reads* — there is still no *writer* on the socket side.

**`toybox reboot bootloader` loses the mode string:** the kernel reboots
but lands in normal boot (observed twice: `FASTBOOT: none`, then
`ADB-BACK: aginxosredfin device`). Environment facts from this console:
`/proc/partitions` lists only ram0-15 (no UFS/storage driver → the misc
partition BCB is unwritable); `/proc/device-tree` is absent; `/lib/modules`
has no pm8150-pon / qcom,pon / reboot-mode / nvmem-reboot-mode modules;
`insmod qcom-spmi-sdam.ko` loads and registers
(`SDAM base=0xb100 size=128 registered successfully`) but a following
`reboot bootloader` *still* boots normally — no reboot-mode consumer
binds. **Corrects the v32 closing note: recovery from a HOLD image does
need button combos — manual Power+VolDown is the only proven fastboot
entry from this env** (used again this session to flash v33).

**v33 (PID 1 takeover — goal state reached 2026-08-27):** flashed
`USBADB=1 HOLD=1 SPLASH=0 MODULES=0`. The parent waits out the usb
console child's setup window, then kmsg:
`usb child exited code=0` (t=29.2 s) → `HOLD (no handoff)` →
`exec aginxos-init (PID 1 takeover)` → `aginxos-init: start v0.2.0
pid=1 hold=true` → `basics ok` → `HOLD — aginxos-init is PID 1` →
`hold heartbeat 1` (t=39.2 s). `ps`: **PID 1 = aginxos-init** (S),
adbd (PID 272) reparented to it, its `sh`/`ps` children under adbd,
**no zombies** (the reaper works — adbd's one-shot shells exit and are
collected). Console stays authorized root throughout. First real
AginxOS userspace as init.

Device state (2026-08-27, end of session): **left running the v33 test
image** (HOLD, aginxos-init as PID 1, authorized root adb console alive).
Restore needs manual Power+VolDown → flash `boot/stock-vendor_boot.img`.

## UFS storage up; software fastboot route found (v34, 2026-08-27)

**Correction to the two entries above (2026-08-27, same day, later):** the
`toybox reboot` / `toybox reboot bootloader` "lands in normal boot" results
were **invalid — no reboot ever happened**. The ramdisk toybox build has no
reboot applet (`toybox: Unknown command reboot`, rc=127) and the earlier
pipelines swallowed stderr. The 868 s "post-reboot" uptime was the same
unbroken session. Retracted: "mode string lost", "SDAM doesn't bind" —
untested at the time. (The sdam module-load observation itself stands.)

**UFS bring-up (live insmod, no flash):** stock ships
`ufshcd-core/ufshcd-pltfrm/ufs_qcom` plus the whole PHY family. The working
chain (order from modinfo `depends=`, all rc=0, ~2 s total):
`phy-qcom-ufs → phy-qcom-ufs-qmp-v4 → phy-qcom-ufs-qmp-v4-lito (lito =
SM7250) → ufshcd-core → ufshcd-pltfrm → ufs_qcom`. Probe log: QC ICE 3.1.81
found, PHY gear 3 / 2 lanes / FAST MODE, `SKhynix H9HQ16AFAMMDAR` with 6
LUNs → sda–sdf. No panic. /dev has no nodes for them (tmpfs + no ueventd) —
mknod from /proc/partitions (79 nodes).

**Partition map (GPT read from each LUN, pulled + parsed on host):** GPT
LBA units on sda are **4096-byte** (first_lba × 8 = 512-unit /sys start).
misc = sda3 (1 MiB). boot_a/b, vendor_boot_a/b, dtbo, modem, klog,
metadata, vbmeta_system, super, userdata all on sda. sdb/sdc = xbl(+config)
a/b, sdd = cdt/ddr, sdf = modemst/fsg/fsc, sde = the 42-partition firmware
LUN (aop/tz/hyp/abl/keymaster/.../devinfo/splash/logfs/uefivarstore).

**BCB route dead:** wrote `bootloader`, then `boot-fastboot` into the
bootloader_message.command field of misc (sda3) — both boots proceeded
normally (tested with RESTART2 and with plain RB_AUTOBOOT). This ABL does
not honor BCB for fastboot entry. misc was restored to its original
content afterwards (command field zeros; the vendor area at offset 2048
holding `theme-dark` untouched).

**RESTART2("bootloader") needs msm-poweroff loaded:** with no handler the
mode string goes nowhere (normal boot, `androidboot.bootreason=reboot`).
`CONFIG_POWER_RESET_QCOM=m` → `msm-poweroff.ko`, whose stock `depends=`
chain is: `qcom_hwspinlock` (this is what unblocks smem's *deferred probe
pending* on its hwlock supplier — without it msm_minidump fails with
"SMEM is not initialized") → `smem_state` → `msm_minidump` → `watchdog_v2`
→ `qpnp-power-on` (PON driver, SPMI; registers power-on reason input
device) → `msm-poweroff` (registers the restart handler;
`set_restart_msg`). With the chain live:
**`reboot(2) RESTART2 "bootloader"` → fastboot in ~6 s.**

**v34 (baked in):** modules.usb grew 52 → 58 (hwspinlock inserted *before*
smem; smem_state/minidump/watchdog/qpnp-power-on/msm-poweroff appended) —
`modules ok=58 fail=0`, restart handler registered by t=1.02 s. aginxos-init
v0.3.0 gained `reboot [mode]` (raw SYS_reboot with RESTART2 mode string).
Verified from a clean v34 boot: `/aginxos/aginxos-init reboot bootloader`
→ fastboot in 6 s, serial confirmed. PID 1 takeover unchanged (v0.3.0 as
PID 1, takeover at t=29.3 s after the usb child's window). **This
supersedes every earlier "manual Power+VolDown only" note: recovery is
now one adb command from any HOLD image.**

Device state (2026-08-27, end of session): **left running the v34 test
image** (HOLD, aginxos-init v0.3.0 as PID 1, authorized root adb, reboot
chain live). misc partition restored to original. Recovery:
`adb shell /aginxos/aginxos-init reboot bootloader` → flash
`boot/stock-vendor_boot.img`.

## v35: aginxos-init owns storage (2026-08-27)

New `/aginxos/storage` flag (pack env `STORAGE=1`): after the PID 1 takeover,
aginxos-init loads the 6-module UFS chain itself and mknods every block node
from /proc/partitions — boot-time storage as an init responsibility, no
manual insmod.

First flash failed instructively: every module logged
`storage mod fail ...: Function not implemented` (ENOSYS). Root cause: the
`load_module()` helper had hardcoded `SYS_finit_module = 313` — the x86_64
number, not aarch64's 438. The path had never executed on device before
(the allowlist has been empty since the bootloop era), so the wrong constant
sat unnoticed through three versions. Fixed to `libc::SYS_finit_module`.

Second flash (v0.4.0, fixed): kmsg
`aginxos-init: storage up: ufs mods ok=6, 95 block nodes` at t=29.29 s,
right after the takeover. `/dev/sda3` (8,3), `/dev/sde42` (259,26), `/dev/sdf`
(8,80) present with correct major:minor pairs from /proc/partitions; misc
reads back the restored all-zero BCB. PID 1 = aginxos-init v0.4.0
throughout.

Device state (2026-08-27, end of session): **left running the v35 test
image** (HOLD, STORAGE=1, aginxos-init v0.4.0 as PID 1 with storage up,
authorized root adb, software fastboot route). Recovery:
`adb shell /aginxos/aginxos-init reboot bootloader` → flash
`boot/stock-vendor_boot.img`.

## v0.5.0: /dev/block/by-name (2026-08-27)

aginxos-init now parses each LUN's GPT itself (entries at LBA2, 128-byte
records, UTF-16LE names; all six LUNs are 4096-byte logical blocks — probed)
and maps name→partition by matching `first_lba × 8` against the kernel's
`/sys/block/*/start` (512-byte units). Boot log:
`storage up: ufs mods ok=6, 95 block nodes, 73 by-name links (by-name ok)` —
73 = every named GPT entry across sda–sdf. Spot-checked:
misc→sda3, super→sda18, userdata→sda19, vendor_boot_a→sda8, boot_a/b,
vbmeta*, metadata, logfs all present. (The mojibake entries seen in the
earlier host-side dd+parse were parse artifacts, not partition names —
they vanish once entries are read from the proper LBA2 offset.)

Device state unchanged in kind: v35 image (now aginxos-init v0.5.0) HOLD +
STORAGE, storage and by-name up at t=29.4 s.

## v0.6.0: super sub-partitions mounted at boot (2026-08-27)

Goal reached: from a clean boot, aginxos-init (PID 1) now mounts
**system_a, vendor_a, product_a, system_ext_a** ext4-ro at `/<name>` — the
Android dynamic partitions are first-class readable filesystems in our
world, ~0.15 s after the storage chain comes up. New `/aginxos/super` flag
(pack env `SUPER=1`, implies STORAGE).

Chain, in boot order: UFS modules → /proc/partitions nodes → GPT parse →
by-name → **liblp super metadata parse** (geometry@0x1000, header@0x3000,
tables at header+header_size; partition names are plain ASCII[36];
first_extent/num_extents are u32@40/@44; extents are *packed*
`{u64 num_sectors; u32 target_type; u64 source_data}` with source_data at
offset 12 — not 16) → dm ioctls on /dev/mapper/control (mknod 10:236) →
DM_DEV_CREATE + DM_TABLE_LOAD `linear <super maj:min> <src> <len>` +
DM_DEV_RESUME → mknod /dev/dm-N → mount(2) ext4 ro. Verified extent map
(tile, no gaps in the used region): product_a [3072, 5467648), system_a
[5468160, 7192544), system_ext_a [7193088, 7879392), system_b
[7879680, 7919352), vendor_a [7919616, 9396192) — 512-byte sectors inside
super. The earlier "system_ext overlaps system" reading was an artifact of
reading source_data at +16.

Three lessons, each cost a flash cycle or was caught just in time:

- **DM_DEV_CREATE is picky about the version tuple and buffer size.**
  Version 4.37 with an exact 200-byte data_size → silent EINVAL (no dmesg).
  Querying DM_VERSION first and reusing the kernel's tuple (4.39.0) with a
  4096-byte buffer works. Also: DM_DEV_CREATE returns the dm dev in
  *userspace new encoding* (253:0 → 0xfd00) — pass it to mknod as-is.
- **UFS reads right after probe can return garbage.** First boot run
  (t=44.8 s) fed the kernel corrupted extents (dmesg:
  `linear: Invalid device sector` ×2, `start=30726 not aligned` — real value
  3072); the same code minutes later read everything correctly. Fixed with
  double-read-until-identical + tiling/bounds validation in
  `parse_super_stable` (observed: one 50 ms retry, then stable).
- **Rust Strings are not NUL-terminated — the boot-blocker bug.**
  `copy_from(ptr, len + 1)` read one byte past the String allocation and
  overwrote the zeroed ioctl buffer with heap garbage. product_a's params
  became "259:29 30726" (stray '6') on *every* boot while the others drew
  zeros or non-digits by luck, and live runs never reproduced it (different
  heap history). The kernel messages pinning it: `dm-linear: Invalid device
  sector` (trailing junk fails `%llu%c` sscanf) and
  `start=15731712 … not aligned to h/w logical block size 4096`.

Also observed: the super partition's device number differs between boots
(259:29 one boot, 259:2 the next — LUN scan order varies), so resolving
super through by-name + /proc/partitions at runtime is required, not a
convenience.

Boot log (final flash, HOLD+SPLASH+USBADB+STORAGE+SUPER):

    aginxos-init: start v0.6.0 pid=1 hold=true ... storage=true super=true
    aginxos-init: storage up: ufs mods ok=6, 95 block nodes, 73 by-name links (by-name ok)
    aginxos-init: super: stable after 1 retries
    aginxos-init: super: system_a 1724384 sectors @ 5468160
    aginxos-init: super: mounted system_a at /system_a        (dm-0)
    ... vendor_a (dm-1), product_a (dm-2), system_ext_a (dm-3)
    aginxos-init: super up: mounted 4 [system_a,vendor_a,product_a,system_ext_a]

`/proc/mounts`: `/dev/dm-0..3 → /system_a /vendor_a /product_a
/system_ext_a ext4 ro,relatime`. Full Android trees readable
(system_a has apex/bin/init, vendor_a has bin/etc/firmware, product_a
app/bin/fonts, system_ext_a app/bin/etc).

Operator subcommands added: `aginxos-init parse-super <file>` (metadata
dump) and `aginxos-init mount-super` (live re-run of the super flag — how
the String-NUL bug was isolated without extra flashes; note a re-run leaves
already-mounted names with `DM_DEV_CREATE: EBUSY`, which is expected).

Device state (2026-08-27, end of session): **running the v0.6.0 test
image** (HOLD+SPLASH+USBADB+STORAGE+SUPER, aginxos-init v0.6.0 as PID 1,
four super partitions mounted ro, authorized root adb). Recovery unchanged:
`adb shell /aginxos/aginxos-init reboot bootloader` → flash
`boot/stock-vendor_boot.img`.

## v0.7.0: M2 — rootfs on userdata, switch_root, busybox init (2026-08-27)

Milestone 2 reached and **verified reproducible across cold boots**: from a
clean boot the device now runs *our* rootfs — aginxos-init (PID 1 in the
initramfs) mounts a 512 MB ext4 image on `userdata`, carries the live mounts
across, `switch_root`s, and busybox init becomes the new PID 1 with a
respawned adbd as the console. Android userspace no longer runs.

Image built host-side: `scripts/build-rootfs.sh` assembles
`/system` + `default.prop` + `*_contexts` from the unpacked vendor ramdisk,
static busybox 1.36.1 (`boot/rootfs/busybox`), `/etc` templates
(`boot/rootfs/etc/`), and the musl release binaries, then
`mke2fs -t ext4 -F -d` (the android-platform-tools build, e2fsprogs 1.46.6)
writes raw ext4 → `fastboot flash userdata out/rootfs.img`. New flags:
`/aginxos/rootfs` (pack env `ROOTFS=1`, implies STORAGE) and
`/aginxos/keep-adbd` (`KEEPADBD=1`, diagnostic only).

Cold-boot timing of the final image (HOLD+SPLASH+USBADB+ROOTFS): adbd
console up t≈21 s; old adbd killed at the switch → USB drops ≈ t+44 s;
re-enumeration ≈ +12 s later, now respawned by busybox init. In the new
root: `/proc/1/comm` = `init`, adbd PPID 1, `/proc/mounts` shows
`/dev/sda19 / ext4 rw`, `uptime`/`hostname` (aginxos) work, rcS marker
fresh, ownership normalized to root:root.

Getting there cost two dark boots; every failure mode below is observed:

- **`/dev` is not a mount in the initramfs.** The trampoline mknod'd console,
  urandom, `__properties__` and block nodes directly into the initramfs root
  directory, so `/proc/mounts` never lists /dev — any "move all mounts"
  loop silently skips it. First ROOTFS boot: the new adbd died at
  `getentropy failed: No such file or directory` (no /dev/urandom), and with
  no adbd alive the already-torn-down UDC never re-enumerated → device dark
  until forced power-off. Fix: explicit `mount("/dev", newroot/dev,
  MS_BIND|MS_REC)` *before* iterating /proc/mounts.
- **adbd's death tears the gadget down.** Closing ffs ep0 clears
  `/config/usb_gadget/g1/UDC` (observed empty in /var/wdt.log). A fresh adbd
  opening ep0 on the *surviving* ffs mount re-activates the function; then
  `echo a600000.dwc3 > .../UDC` re-binds and USB re-enumerates (~12 s).
  Mounting a *new* functionfs over /dev/usb-ffs/adb is wrong — it shadows
  the instance g1 is linked to and the bind becomes a no-op. The respawn
  wrapper (`etc/init.d/adbd`) therefore self-binds the UDC with a retry
  loop, backgrounded so `exec adbd` isn't delayed.
- **Processes don't cross a covering root mount.** With KEEPADBD=1 the kept
  trampoline adbd kept its old fs root, so `adb shell` aborted
  (SIGABRT, shell_service.cpp:385 "Failed to get SELinux context"), and
  `adb reboot` failed ("failed to create pty master" — no /dev/ptmx in its
  world). Diagnostic value only; the shipping path kills the old adbd and
  lets busybox init respawn it in the new root.
- **MS_MOVE vs MS_BIND:** moving mounts broke the kept console (above);
  `remounts_into(newroot, keep)` chooses bind for the diagnostic path and
  move otherwise.
- **busybox init ordering:** `::sysinit:` (rcS) completes before
  `::respawn:` entries start — rcS does the one-shot setup (applet install,
  uid-501 → root chown of the mke2fs-built tree, idempotent mounts, lo up)
  and the respawn wrapper handles everything adbd needs per instance.
- **No pstore on this kernel** (empty /sys/fs/pstore, no ramoops module):
  kmsg dies with the old world, so `/var/adbd.log` + `/var/wdt.log` on the
  ext4 root are the only cross-boot evidence. That's how the getentropy and
  UDC-teardown failures were actually diagnosed (keep-adbd build + bind
  mounts + `/proc/1/root` as a bridge into the new root).
- **Slot retry counter:** several failed boots in a row drop redfin to
  fastboot on its own; `fastboot set_active a` before reboot resets it.

Recipe committed alongside: `boot/rootfs/` (busybox + etc templates,
byte-identical to the proven on-device files — verified by md5 against the
live rootfs) and `scripts/build-rootfs.sh`. The built image lives in
`out/rootfs.img` (gitignored).

Device state (2026-08-27, end of session): **running the AginxOS rootfs**
(v0.7.0 test vendor_boot, HOLD+SPLASH+USBADB+ROOTFS, keep-adbd off; userdata
is our ext4 — Android userdata is gone). Root adb authorized, serial string
`aginxosredfin`. Recovery: `adb shell /aginxos/aginxos-init reboot bootloader`
→ `scripts/restore-vendor-boot.sh` (back to stock vendor_boot, still our
userdata); full Android restore = flash-all from `.factory/`.

## M3: touch input proven on device (2026-08-27)

Touch works. The full chain — SPI controller, qrtr, display stack through
msm_drm, panel registration, then the vendor-side touch modules — was loaded
live from an adb shell in the AginxOS rootfs, the Samsung controller
registered as `sec_touchscreen`, and a physical swipe/drag produced 400
input events captured to `/dev/input/event2` and decoded host-side (4 touch
downs, BTN_TOUCH edges, MT slots with tracking ids, X 967–2036 in the 2×
precision space of the 1080-wide panel, Y 26–1001 native, pressure values,
SYN_REPORT framing). Every claim below is from probe output, kmsg, or the
captured events.

**The hardware (DT + kmsg, not datasheets):** the touchscreen is a Samsung
S6SY79X on **SPI** (`spi@880000`, address spi0.0) — not i2c. DT node has
`compatible = "sec,sec_ts"`, `sec,firmware_name = s6sy79x.bin`, gpios on the
TLMM phandle 0x19 (irq 9, reset 8, switch 35), `avdd-supply` from
rpmh-regulator-ldoa17, and — decisive for bring-up order — `sec,panel_map`
pointing at the three DSI panel nodes (s6e3hc2 dvt/evt/gamma): sec_ts probe
defers (-517) until msm_drm registers a panel. Kernel 4.19.278;
`CONFIG_TOUCHSCREEN_SYNAPTICS_DSX_v27=y` is a red herring (no synaptics node
exists) — the real driver is the vendor module `sec_touch.ko`.

**Module geography (probed):** the vendor ramdisk's `/lib/modules` (218
modules) holds the display and qrtr stacks; the *late* modules —
`sec_touch.ko`, `touch_offload.ko`, `heatmap.ko`, `touchscreen_tbn.ko`,
`qmi_helpers.ko`, plus the WLAN pair (`wlan.ko`, `google_wlan_mac.ko`) and
firmware — live in `/vendor_a` (`/vendor_a/lib/modules`,
`/vendor_a/firmware`). redfin has **no vendor_dlkm** in the super metadata;
the rootfs world reaches these via the super mounts from v0.6.0.

**Validated load order** (insmod from the rootfs shell, this exact sequence
registered the touchscreen):

    spi-geni-qcom rpmsg_core qrtr qrtr-smd ion-alloc qseecom hdcp_qseecom
    msm_hdcp msm_ext_display llcc-slice dispcc-lito qpnp-amoled-regulator
    msm_drm   →   (~60 s: dsi_prop fw fallback = panel registration)
    qmi_helpers touch_offload heatmap touchscreen_tbn sec_touch

Lessons paid for along that order:

- **qrtr before sec_touch is mandatory.** With qrtr absent, `qmi_handle_init`
  returns -97 and sec_ts then *dereferences the ERR_PTR* → kernel OOPS
  (paging request ffffffffffffffbf). After the OOPS the device was half-dead:
  sysfs `unbind` of spi0.0 wedged forever; only a reboot clears it.
- **qpnp-amoled-regulator + dispcc-lito before msm_drm** (panel rails +
  clocks). Without them dsi_display defers forever, no panel ever registers,
  and sec_ts dies at DT parse.
- **Deferred probe is our friend:** insmodding sec_touch *before* panel
  registration just defers (-517) and the kernel retries it on its own when
  the provider appears — no blind sleeps needed.
- **dsi_prop:** msm_drm requests `dsi_prop` firmware that exists nowhere on
  this unit (absent on stock Android too); the request waits out the 60 s
  sysfs fallback, and *that timeout is what releases panel registration*.
  Benign — budget for it.

**Firmware loading:** the direct kernel path never fires on this kernel even
with `/sys/module/firmware_class/parameters/path` pointed at
`/vendor_a/firmware` — requests land in the sysfs fallback
(`/sys/class/firmware/<name>/`). The feeder that works: poll for the sysfs
dir, then `echo 1 > loading; cat fw > data; echo 0 > loading`. `s6sy79x.bin`
must be fed this way or sec_ts's post-probe fw update times out.

**/dev/input:** nothing udevs in our world — the input tree only exists
after `mdev -s` runs *after* the touchscreen registers (rcS runs one early
pass; the touch script runs another post-registration). Symptom when
forgotten: `dd`/reads on `/dev/input/event2` fail with silent ENOENT (the
directory itself is missing).

**Why the battery dies on the Mac cable (observed, not yet fixed):** the
64-module usb base already loads the full charging chain (pmic-voter,
p9221, qpnp-battery, of_batterydata, qpnp-smb5-charger, tcpm, qpnp_pdphy,
fsa4480) — and during the M3 session kmsg shows it alive and working:
`SMB5 status - usb:present=1 type=6 batt:present=1 health=1 charge=3`,
`QPNP SMB5 probed successfully`, usbpd + tcpm registered. type=6 is a
BC1.2-classified port (CDP), i.e. 5 V ≤1.5 A ≈ 7.5 W in. What is NOT
loaded is the Google policy layer (`google_charger.ko`, `google-battery.ko`
sit in the ramdisk unused). Net effect observed twice: plugged into the Mac
with the splash backlight at max and the SoC busy, the pack still drains to
the PMIC UVLO cut. Practical rule until quantified: charge the device
powered off from a real PD charger (PMIC charges autonomously, no OS
needed); a Mac port trickle-charges a powered-off unit but cannot keep the
OS running. Unmeasured: actual `current_now` in vs out — worth a
power_supply readout in rcS when the device is back.

**Reboot escape (re-confirmed):** only `/aginxos/aginxos-init reboot` works
reliably; `adb reboot` hangs on the ptmx gap and busybox `reboot`/`reboot -f`
are no-ops in the rootfs world.

**Battery-death forensics:** two mid-session black screens were the battery,
not the kernel — kmsg ends with `PMIC input: code=116 ... os=1` (PMIC
power-cut on discharge, UVLO) with no panic anywhere. A dark phone right
after a flash is *not* evidence of a bad image; check whether USB shows the
device at all before bisecting.

**MODULES_FULL does not boot (observed):** packing the trampoline with
`MODULES_FULL=1` (entire modules.load, 218 entries) produced a boot that
never reached the adb console and dropped the device to fastboot with the
slot retry counter burned — real resets, not a hang. Not bisected module by
module; instead abandoned: msm_drm sits at line 211/218, so the stock
"through msm_drm" loadfile mode is nearly the same list and just as risky.
The shipping design is the 64-module USB/storage base in the trampoline
(modules.usb) plus the touch chain loaded from the rootfs world — the exact
path proven live above.

**Integration state (not yet observed):** `scripts/build-rootfs.sh` now
stages the 13 ramdisk-half modules into `/lib/modules` in the image and
`/etc/init.d/touch-bringup` (backgrounded from rcS) loads the full chain
with the firmware feeder and post-registration `mdev -s`. First boot of that
image was flashed 2026-08-27 but **not observed** — the device showed
nothing on USB 65 s after `fastboot reboot` (no adb, no fastboot fallback;
consistent with the battery deaths observed the same day, unverified) and is
on the charger. Verification pending: boot, then `/var/touch.log` should
show the chain completing with zero manual steps.

Device state (2026-08-27, mid-experiment): vendor_boot_a = test image
(HOLD+SPLASH+USBADB+ROOTFS, no modules flag), userdata = rootfs with the
touch chain staged, slot a active, device powered off/charging — boot of
this image unverified.
