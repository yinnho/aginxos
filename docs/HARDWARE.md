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
with the firmware feeder and post-registration `mdev -s`.

**Integration observed (2026-08-27, two consecutive boots):** after charge
recovery, the image booted and `/var/touch.log` shows the whole chain
completing with zero manual steps — all 13 ramdisk-half insmods ok, feeder
fed `s6sy79x.bin` at try 63 (the dsi_prop 60 s timeout releases panel
registration, then sec_ts's deferred probe fires), `sec_touchscreen`
registered at **t+64 s** from rcS, nodes created via `mdev -s`. A physical
swipe captured from `/dev/input/event2` right after boot decoded as a clean
single-finger drag (BTN_TOUCH, slot #0, smooth position trace, pressure
36–45, **200 Hz report rate**). The earlier "nothing on USB" scare after
this flash was the battery again, not the image.

Two follow-ups noticed, not yet addressed: the panel keeps showing the
bootloader's Google logo — msm_drm now loads in the rootfs phase, *after*
the trampoline's splash sequence ran with no DRM available, so nothing ever
paints (backlight is on, `card0-DSI-1` registered; painting needs a
userspace DRM client in the rootfs). And the pack still drained to the PMIC
cut while on the Mac despite `usb` psy reporting 5.17 V / current_max 3 A —
no `battery` psy exists (that's `google-battery.ko`, not loaded), so net
flow is unmeasurable from userspace as built.

Device state (2026-08-27, end of session): **running the AginxOS rootfs
with boot-integrated touch** (vendor_boot_a = HOLD+SPLASH+USBADB+ROOTFS, no
modules flag; userdata = rootfs with the touch chain staged; slot a active,
touch verified two boots in a row). Root adb as `aginxosredfin`. Recovery
unchanged: `adb shell /aginxos/aginxos-init reboot bootloader` →
`scripts/restore-vendor-boot.sh`; full Android restore = flash-all from
`.factory/`.

## M3b: panel painting — green splash persistent on screen (2026-08-27)

The screen now lights green (white-bordered) at boot and stays. Full chain
of what was wrong, each step observed:

- **Black screen ≠ dead system.** The panel showed the bootloader's Google
  logo via *cont-splash scanout* — independent of KMS — then went black
  when that stopped, while the OS kept running (adb, touch, backlight all
  alive). Nobody had ever done a KMS mode set: `card0-DSI-1` sat at
  `status=connected, enabled=disabled`, with valid modes present
  (`1080x2340x60x60948cmd`, `90x94812cmd` — command-mode DSI).
- **v1 painter failed on two counts** (boot/rootfs/src/splash2.c v1):
  (a) GETRESOURCES rejects a second call when count_fbs/encoders are
  nonzero with null pointers — zero the counts; (b) it searched CRTCs for
  `mode_valid` (never true here) and fell back to a *synthetic* video-mode
  timing that a cmd-mode panel refuses. v2 probes connectors with the
  encoder-list pointer set (without it the kernel reports enc=0) and takes
  the connector's real mode; skip the Virtual connector (type 15, garbage
  modes) — the panel is conn 29 (type 16 DSI) / enc 28 / crtc 105.
- **Wrong panel variant = dead panel.** The bootloader detects the panel
  and appends `msm_drm.dsi_display0=qcom,mdss_dsi_s6e3hc2_dvt_dsc_1080p_cmd:`
  to the cmdline — but the driver's bind fell back to the PLAIN
  `s6e3hc2_dsc_1080p_cmd` DT node (no match on the cmdline string; param
  format is `<node>:<configX>`). The DT diff between the nodes is exactly
  two properties: `google,mdss-dsi-te2-info` and
  `google,mdss-dsi-te2-lp-threshold` — only dvt has them. With the plain
  node bound, the first mode set died at post-enable: `TE check failed →
  esd ... PANEL_DEAD` and `wait_for_idle: -110` (sticky — connector torn
  down, no re-bind possible: the driver suppresses bind attrs; debugfs is
  not compiled in). `/proc/interrupts` showed the `TE_GPIO` (msmgpio 10)
  registered with count 0 — the panel never asserted TE.
  Fix: insmod msm_drm with the variant explicitly:
  `insmod msm_drm.ko dsi_display0=qcom,mdss_dsi_s6e3hc2_dvt_dsc_1080p_cmd:0`
  → bind message shows dvt, `cont_splash enabled in 1 of 1 display(s)`
  appears (new), and the mode set goes through clean.
- **The painter must stay resident.** With dvt bound, SETCRTC rc=0 and the
  green really hit glass (user saw it) — but the painter exiting drops DRM
  master, and dsi_backlight's early/late-dpms hooks turned the panel right
  back off (`dsi_backlight_early_dpms ... state:0x0` 28 ms after SETCRTC):
  green flash, then black. Fix: run `/bin/splash <color> hold` as a daemon
  (`nohup ... &` from touch-bringup) — it sleeps forever holding the fd.
  Result: `enabled/On`, splash pid alive, user-confirmed persistent green.
- TE count is still 0, yet no ESD/PANEL_DEAD with dvt bound (minutes
  stable) — the dvt node's ESD path evidently doesn't depend on that TE
  line. Watch it; if the panel ever dies in service, ESD is the suspect.

Recipe: `boot/rootfs/src/splash2.c` (zig cc → /bin/splash at image build),
touch-bringup launches the green daemon after touch registers; rcS runs a
kmsg follower (`/var/kmsg-follow.log`) and a 5 s heartbeat
(`/var/heartbeat.log`) so the next full-device loss discriminates
power-cut vs hang (the afternoon losses had neither running — battery
theory stands from the morning PMIC code=116 forensics, unconfirmed for
the afternoon ones).

## M3c: battery gauge + charge policy proven on device (2026-08-27)

Goal: 电量识别 — a readable battery percentage, plus the Google charging
policy layer so the device actually charges while in AginxOS.

Observed result (live session, then 2 integrated boots):

- `/sys/class/power_supply/battery/capacity` = **100**, `status`
  Charging, `voltage_now` ~4.32–4.33 V, `current_now` +0.9–1.1 A while on
  the Mac's USB-C port, `temp` 33.x, `health` Good,
  `charge_counter` 4187000.
- Charging from the Mac port now measures ~1 A in — this **supersedes**
  the earlier M3 note reading "net discharge off the Mac port": with the
  full Google policy stack up, the input side negotiates properly.
- Boot-integrated: rcS backgrounds `/etc/init.d/battery-bringup`, all 7
  modules load, `battery` psy appears at t+2 s (kmsg: `eeprom ID=
  82300172012, len=10, defer_cnt=0` → `QG Battery-profile loaded` →
  `using SHUTDOWN_SOC @ PON ocv_uv=4422000uV soc=100`). Verified on two
  consecutive reboots, same boot as the green touch splash.

The chain (all 7 .ko from the vendor_boot ramdisk unpack, staged into
/lib/modules by build-rootfs.sh):

    google-bms → at24 → qpnp-qgauge → sm7250_bms →
    google-battery → google_charger (→ qti_qmi_sensor, needs qmi_helpers)

Two non-obvious findings, both observed:

1. **qpnp-qgauge silently defers forever without the EEPROM driver.**
   Its probe's first real call is `gbms_storage_read("batt_eeprom")`
   (found by disassembling qpnp-qgauge.ko: the -517 site is the printk
   right after that call — no inner error line ever prints). The
   batt_eeprom storage entry is registered by **at24.ko** — Google's
   at24 driver calls gbms_storage_register when it binds the physical
   m24c08@50 EEPROM on i2c 98c000 (bus already up in the modules.usb
   base; p9221 shares it). Without at24, dmesg shows only
   `QG-K: qpnp_qg_probe: Failed to get battery type, rc=-517` retried a
   few times, then silence — the deferred-probe list stops getting
   kicked and the device sits unbound (`...:qpnp,qg` with no driver
   symlink; the driver dir does have bind/unbind attrs, unlike
   msm-dsi-display). With at24 loaded first, defer_cnt=0: qg probes on
   its first attempt.
2. **The name wiring in DT is literal.** `/soc/google,battery` says
   `google,fg-psy-name = "bms"` — google_battery retries
   `failed to get "bms" power supply` every ~270 ms until a psy with
   exactly that name exists. qpnp-qgauge registers it. Meanwhile the
   BMS node's own `google,psy-name = "sm7250_bms"` is what
   sm7250_bms.ko registers (`resistance_id=9971`, `status=Charging` —
   a working psy in its own right, but no `capacity`).

Side notes from the debug: the pm7250b ADC5 (iio:device1) registers all
21 DT channels — the therm/batt-id ones appear as `in_temp_*_raw`
(`in_temp_bat_id_raw` = 2622 at the time), an `in_voltage` grep hides
them. /sys/class/power_supply after bring-up: battery, bms, dc, main,
pc_port, sm7250_bms, tcpm-source-psy-usbpd0, usb, wireless.

Recipe: `boot/rootfs/etc/init.d/battery-bringup` (insmod order above,
then a 60×2 s wait for the battery psy, logging one reading to
/var/battery.log). The logged first reading can be empty — the psy node
exists before google_battery's first poll populates it; the live sysfs
reads are the real check.

## M3d: Wi-Fi blocker — modem DOG stall root-caused to user-PD firmware fetch, read-only evidence (2026-08-27)

Standing symptom (observed earlier, unchanged): modem root PD PIL-boots,
registers QRTR services (svc 43/0x1202 locator, svc 66/0xB401
servreg-notif, SSCTL), answers nothing, then `dog_hal_common.c:180
[tmr_slave3] DOG detects stalled initialization` kills it every 65.6 s
(SSR loop). WLAN MSA stays all zeros, WLFW (svc 0x45) never registers,
`wlan.ko` says "FW is in bad state 0x4180". This window was read-only
research (no flashing, no new daemons); everything below is observed
from the device or from pulled stock images.

Observed — device tree (the stock contract):

- `qcom,icnss@18800000` has `qcom,wlan-msa-fixed-region` = phandle 0x9a
  → `pil_wlan_fw_region@8ba00000` (reg 0x8ba00000, size 0x2000000 = 32
  MiB). That region **is** the MSA icnss advertises to WLFW.
- `qcom,mss@4080000` `memory-region` = phandle 0x74 →
  `modem_wlan_region@8c000000` (size 0x1180000 = 17.5 MiB), **nested
  inside** pil_wlan_fw_region — the wlan user PD runs in that carve-out.
- Full reserved-memory map captured (hyp 0x80000000, smem 0x80900000,
  pil_adsp 0x89200000, pil_cdsp 0x87400000, … nothing at 0xb0000000 —
  that address space belongs to the WLAN processor's own bus view, not
  an AP carve-out).

Observed — where modem firmware actually lives:

- `/vendor` is the real partition (dm-1 → /vendor_a, ext4 ro). It has
  **no modem.mdt at all**. Stock fstab.sm7250 mounts
  `/dev/block/bootdevice/by-name/modem` (vfat, flashed by `fastboot
  flash radio`) at `/vendor/firmware_mnt` — empty mountpoint in our env.
- In our env the modem partition is mounted at `/mnt/modem` (image/
  holds modem.mdt + b00…b23), firmware_class path =
  `/vendor_a/firmware`, and copies live in `/lib/firmware` — that is why
  PIL boots the root PD fine.

Observed — the servreg database (all five jsn in /vendor/firmware read:
adspr/adsps/adspua/cdspr/modemuw):

- modemuw.jsn: domain `msm/modem/wlan_pd`, qmi_instance_id **180 (0xB4)**,
  services `kernel/elf_loader`, `tms/servreg`, `wlan/fw`. Instance 180
  matches the observed root-PD servreg-notif registration 66/0xB401
  (180<<8|1) exactly.
- Linaro pd-mapper source (fetched): pd-mapper **is the AP-side
  servreg-locator server** — publishes (svc 64, version 257,
  **instance 0**) and answers GET_DOMAIN_LIST from the jsn maps.
  Qualcomm's /vendor/bin/pd-mapper (strings) plays the same role
  ("Servloc server", handles locator indication-register) and reads jsn
  from both `/vendor/firmware` and `/vendor/firmware_mnt/image`.

Observed — the modem image itself names the fetch mechanism (pulled
modem.b22, 10 MB; strings):

```
… saipan_xml … msm/modem/wlan_pd . wlan_process . wlanmdsp.mbn .
  /readonly/firmware/image/wlanmdsp.mbn .
  /readonly/vendor/firmware_mnt/image/wlanmdsp.mbn .
  /readonly/vendor/firmware/wlanmdsp.mbn .        ← exists in our env
  /readonly/vendor/firmware/wlanmdsp.otaupdate.m…
msm/modem/test_pd . test_process . testpd.mbn . …
```

and the same segment carries the RFS/TFTP client source names
(`rfs_tftp.c`, `tftp_client.c`, `tftp_protocol.c`,
`tftp_socket_ipcr_modem.c`). modem.b23 (25.8 MB) contains
"elf_loader" (the same servreg DB baked into the big segment).

Observed — wlanmdsp.mbn ELF32 phdrs (local parse):

```
ph1 0xb0823000 (hash, fl 0x2200000), ph2 0xb0000000 filesz 0x2e0e7c,
ph3 0xb0300000 memsz 0x501e14, ph5 0xb0802000 — span ≈ 8.2 MiB
```

Observed — AP side cannot be the loader:

- No kernel module registers a QMI server (only qmi_helpers exports
  `qmi_add_server`; msm_icnss/service-locator/service-notifier are
  clients, `add_lookup` only). PIL loads exactly `modem.mdt` + `%s.b%02d`
  (peripheral-loader.c:1021). The WLFW QMI catalog
  (wlan_firmware_service_v01.h) has **no** image-download message.
  ⇒ the only channel for wlanmdsp.mbn into the modem is the modem's
  own TFTP/RFS fetch over QRTR (stock: rmt_storage + tftp_server).

Observed — stock daemon inventory (init.sm7250.rc + init.redfin.rc):
qrtr-ns -f, pd-mapper, pm-service, pm-proxy, cnss-daemon -n -l,
modem_svc -q, rmt_storage, tftp_server, subsystem_ramdump, ssr_setup,
netmgrd, mdm_helper; mpssrfs.rc pre-creates /data/vendor/rfs/mpss
(rmt_storage's serving tree — absent in our env, /data not mounted).

Observed — kernel side is ready and waiting: dmesg at t=8034 s shows
`qcom_smd_qrtr_probe` (modem SSR edge re-appearing) then
`service-notifier: Connection established between QMI handle and 180
service` — icnss's PDR listener re-attaches to the wlan_pd notification
instance every cycle. The stall is entirely modem-side.

Correction to an earlier negative: the qrtr-probe experiment covered
(64, 257) and (4096, 1–12). 257 is pd-mapper's **version**, not its
instance (it publishes instance 0), and the rmtfs/tftp instance space
per domain is not 1–12. "Modem sent zero lookups" is therefore **not**
evidence about the fetch path — the probe never occupied the slots the
modem would use.

Working model (labelled model, not yet device-proven): root-PD TMS
spawns user PDs from its baked saipan_xml; the wlan_pd spawn blocks
fetching wlanmdsp.mbn via the QRTR TFTP/RFS file service (svc 4096);
with no fetch path the root PD's init never completes and DOG kills it
at 65.6 s. Our env has the file at a path the modem searches, but the
QRTR service stack (qrtr-ns + pd-mapper + rmt_storage/tftp_server) was
never up before the modem's first boot, and it is untested whether
late-started daemons can be discovered by an already-booted modem
(AP→modem NEW_SERVER announcement path unverified).

Planned single experiment (NOT run yet, per no-test directive): boot
once with ordering fixed instead of adding daemons later — load base
modules except subsys-pil-tz/peripheral-loader, bring up qrtr-ns,
pd-mapper, rmt_storage, tftp_server (with rfs dirs and
/vendor/firmware_mnt mounted from by-name/modem), *then* load
subsys-pil-tz so the modem first-boots into a fully provisioned AP.
Success criteria: DOG silent >5 min, MSA non-zero, WLFW svc 0x45
appears, then cnss-daemon BDF download, then wlan0.

## M3e: t=0 daemon ordering works — WLFW handshake, FW_READY, wlan0 (2026-08-28)

M3d's planned experiment, run. Result: the success criteria were met and
overtaken — wlan0 exists. Two root causes beyond daemon ordering had to
be fixed on the way; both are now standing fixes in
`boot/rootfs/etc/init.d/radio-bringup` and `boot/rootfs/src/fake-props.c`.

Observed — daemon ordering (boot 1 of the day):

- `radio-bringup` starts binderfs (binder-init), fake-servicemanager
  (fake-sm over /dev/hwbinder), qrtr-ns, pd-mapper, pm-service,
  rmt_storage, tftp_server *before* `subsys-pil-tz` is insmod'd, then
  cnss-daemon with `LD_PRELOAD="trace_open.so fake-props.so"`.
- Modem boots clean: no DOG, no SSR. wlan_pd spawns — `wlan_pd Up`
  indication at t≈61 s, WLFW QMI service (0x45) connects, icnss state
  0x980. MSA programming proceeds.
- pm-service's QMI registration is **not** what gates the spawn (task
  #46): the spawn completed identically with pm-service merely holding
  its service table fake — the modem needs the *lookup* path
  (pd-mapper + qrtr-ns), not pm-service's power calls, to get through
  user-PD init.

Observed — first blocker, BDF filename (cnss-daemon log):

- With no property service, cnss-daemon's `property_get("ro.hardware",
  …, "default")` falls back to "default" → tries
  `bdwlan-…-default…bin` down to `bdwlan-default.bin`, all miss:
  `Failed to read BDF file`. Fix: fake-props now serves
  `ro.hardware=redfin`, `ro.boot.hardware=redfin`,
  `ro.boot.hardware.radio.subtype=2`.
- After the fix (same boot, daemon restart):
  `Using BDF file: /vendor/firmware/bdwlan-redfin.bin`,
  `bdf type 0,result 0, error 0`, then
  `[  69.604414] icnss: WLAN FW is ready: 0xd87` — 8.5 s after WLFW
  connect, same timing as stock.

Observed — second blocker, qcacld probe `-EFAULT` (silent):

- Symptom: `icnss: Driver probe failed: -14, state: 0x40d87` at probe
  time, with **zero** `wlan:` lines in dmesg (qdf print control is not
  registered that early — probe failures before HDD attach print
  nothing, so this failure mode is invisible by default).
- Root cause (source: qcacld + ipa3 on this 4.19):
  `cds_smmu_mem_map_setup()` returns failure → `-EFAULT` when
  `wlan_smmu_enabled != ipa_smmu_enabled`. The icnss iommu domain is a
  translation domain (SMMU on) but `ipa_get_smmu_params(WLAN_CLIENT)`
  reported bypass, because ipa3's real init (SMMU attach, GSI, uC PIL,
  QMI) runs in a work item queued **only** by a write to the `/dev/ipa`
  char dev — stock does `write /dev/ipa 1` in vendor init.rc:212.
  Without ueventd, /dev/ipa never existed and the write never happened.
- Fix in radio-bringup: `grep -w ipa /proc/devices` → mknod `/dev/ipa c
  <maj> 0` → `printf 1 > /dev/ipa`. Observed effect within 20 ms:
  `IPA FW loaded successfully`, GSI enable, QMI IPA init. (Parse with
  `set -- $(grep …)`, **not** awk — this busybox's awk applet
  segfaults, which silently ate the parse once and reproduced the
  -EFAULT boot.)
- `s1_bypass_arr[]` starts all-true in ipa.c and is only corrected by
  `ipa_smmu_wlan_cb_probe` inside that deferred work — the mismatch is
  structural without the write, not a race.

Observed — after both fixes (boot 3, the milestone):

```
[ 54.756] IPA FW loaded successfully
[ 69.604] icnss: WLAN FW is ready: 0xd87
[ 69.717] wlan: hdd_update_tgt_cfg: hw_mac is zero
[ 69.737] wlan: hdd_platform_wlan_mac: provisioned MAC [0]f8:1a:2b:35:c2:ae
[ 69.737] wlan: hdd_initialize_mac_address: using MAC from platform driver
[ 70.012] IPv6: ADDRCONF(NETDEV_UP): wlan0: link is not ready
```

- qcacld probe **passes** (no `Driver probe failed`). qdf logging comes
  alive from here on (`wlan: [pid:X:HDD]` lines appear).
- MAC: DMS get-MAC over QMI fails (error 16) — **non-gating**;
  google_wlan_mac platform driver supplies the provisioned MAC
  f8:1a:2b:35:c2:ae (same as stock reports).
- Netdevs: `wlan0 wlan1 p2p0 wifi-aware0` (+ `rmnet_ipa0` from IPA).
  `ip link set wlan0 up` succeeds; NO-CARRIER pre-association is
  expected.
- firmware_class sysfs fallback server (fwfallback loop in
  radio-bringup, serving /sys/class/firmware/* from /vendor/firmware)
  logged **zero** requests — attach completed without it; the
  WCNSS_qcom_cfg.ini request either never goes through the fallback
  path or succeeded via the direct fw path. Left running, harmless.

Session-end device state: AginxOS test boot on slot a (retries reset to
3), vendor_boot-test flashed, rootfs on userdata with the fixed
radio-bringup/fake-props.so live-pushed, `/bin/nlscan` pushed (M3f).
Stock restore path unchanged (`scripts/restore-vendor-boot.sh`).

## M3f: nlscan — first RF scan on AginxOS, 15 BSS observed (2026-08-28)

To prove the radio actually receives (not just that a netdev exists),
wrote `boot/rootfs/src/nlscan.c` — a static musl nl80211 client
(trigger scan + dump results), since busybox has no wireless tools and
we ship no libnl. Two generic-netlink gotchas on the way, both now
comments in the source: nlmsg_len must include GENL_HDRLEN or attrs
overwrite the genlmsghdr (kernel answers -EINVAL / -EOPNOTSUPP), and
scan triggers need NLM_F_ACK or success is silent (client blocks).

Observed on device (`/bin/nlscan wlan0`, family id 21, ifindex 9):

```
scan triggered, waiting…
44:d1:fa:de:11:70  ch=4    -39.00 dBm  Legrand AP
74:39:89:07:a7:43  ch=?    -43.00 dBm  <hidden>/2602
76:39:89:07:a7:44  ch=55   -64.00 dBm  2602_5G
74:39:89:07:a0:a7  ch=?    -77.00 dBm  2602
f8:6f:b0:c3:4a:02  ch=6    -91.00 dBm  2401
… 15 BSS total, RSSI range -39…-91 dBm, 2.4+5 GHz
```

- genl family dump confirms `nl80211` (21) and `cld80211` (27)
  registered.
- `/proc/net/wireless` lists wlan0/wlan1/p2p0/wifi-aware0.
- M3 Wi-Fi bring-up is **done**: modem → WLFW → FW_READY → qcacld →
  netdevs → real RF scan with sane RSSI. Not yet attempted:
  association/authentication (needs a wpa_supplicant-class userland or
  raw nl80211 AUTH/ASSOC), IP provisioning.

## M4: Wi-Fi association + WPA2-PSK 4-way handshake + DHCP (2026-08-28)

`boot/rootfs/src/wifi-join.c`: self-contained WPA2-PSK supplicant (no
libnl, no wpa_supplicant). DISCONNECT → CONNECT (fw does auth+assoc) →
EAPOL M1..M4 over an AF_PACKET socket → PTK/GTK NEW_KEY installs. Crypto
embedded (SHA-1/HMAC/PBKDF2/AES-128/RFC 3394 unwrap). Against "Legrand
AP" (44:d1:fa:de:16:b9, WPA2-mixed: **TKIP group / CCMP pairwise**,
EAPOL version 1):

```
connected (CONNECT event, status 0)
M2 sent #1 (rc 135)
eapol: ver 1 type 3 len 175 desc 2 ki 13ca kl 16   ← M3
M3 MIC verified — passphrase correct
GTK kde idx 1 len 32                                ← TKIP group key
M4 sent
udhcpc: lease of 192.168.0.166 obtained from 192.168.0.1
ping 192.168.0.1  → 3/3, 0% loss, ~1.6–4 ms
ping 223.5.5.5     → 3/3, 0% loss, ~5 ms            ← internet reachable
```

Driver facts observed getting there:

- **CONNECT attrs must mirror the AP's group cipher.** This AP is
  WPA2-mixed (group TKIP, pairwise {TKIP,CCMP}); advertising group CCMP
  in NL80211_CMD_CONNECT makes the qcacld SME scan-cache filter reject
  the entry — CONNECT then never emits an event. Mirroring
  `grp000fac02` fixes it. Same mirror is applied to the GTK NEW_KEY
  cipher (32-byte key for TKIP group).
- **TRIGGER_SCAN must carry an IE** (we send our 22-byte RSNE): HDD
  stores it as scan_add_ie → roam_profile->nAddIEScanLength, and
  csr_scan_for_ssid's `qdf_mem_malloc(nAddIEScanLength)` is a
  **malloc(0) → NULL → NOMEM** without one — the join path aborts
  before the SME filter ever runs (dmesg `csr_scan_for_ssid:1395`).
- **EAPOL socket must bind ETH_P_PAE, not ETH_P_ALL.** HDD's
  `hdd_is_tx_allowed()` exempts EAPOL from the pre-keys CONN peer state
  only by checking skb->protocol (stamped from sll_protocol); with
  ETH_P_ALL every TX is dropped (tx_packets == tx_dropped). With
  ETH_P_PAE dmesg shows `EAPOL-2 TX … status: succ`.
- **Re-CONNECT while associated returns -EALREADY** (114); the tool now
  sends DISCONNECT and waits for the event first.
- udhcpc wins a lease but applies nothing without its event hook —
  added `usr/share/udhcpc/default.script` (addr/route/resolv.conf) to
  the rootfs; verified it auto-configures after join.

Three bugs in our own crypto/protocol produced the identical on-air
signature — AP retransmits M1 4× then **deauth reason 15**
(WLAN_REASON_4WAY_HANDSHAKE_TIMEOUT), on every AP/passphrase tried.
Recording them because each maps to a hostapd check (source-read of
wpa_auth.c, not a device observation): Key Information bits are
**B7=ACK, B8=MIC** (M1 ki 0x008a is standard; an M2 with 0x008a is
dropped as "Key Ack set"), the EAPOL-Key MIC covers the **full EAPOL
PDU from the version byte** (wpa_verify_key_mic hashes the
ieee802_1x_hdr onward), and the PTK PRF input is
`label‖0x00‖data‖counter` (wpa hashes strlen(label)+1). An AP that
retransmits-M1-and-deauths-15 with a spec-correct M2 = wrong MIC = PMK
mismatch; everything else (RSNE compare, state machine) deauths
immediately with a different reason code.

Session-end device state: AginxOS test boot, slot a, our test
vendor_boot (not stock), rootfs **rebuilt and flashed to userdata** —
the run above is from that clean image (not live-pushed binaries).
wlan0 associated to "Legrand AP" with lease 192.168.0.166; association
is per-boot manual (`/bin/wifi-join`), not persisted. Stock restore
path unchanged.

## M5: boot card on panel + automatic Wi-Fi + internet check (2026-08-28)

Goal: cold boot ends with a branded on-screen card proving every bring-up
stage, Wi-Fi joined automatically, and a real HTTP fetch through baidu —
recordable on video, no adb involved.

New pieces (all on the flashed image, observed cold-booting):
- `/bin/bootcard` — DRM boot-status renderer. Holds DRM master for its
  whole life (it replaces the M3 green splash; bring-up scripts now report
  `key ok|fail|run [detail]` lines into `/run/boot.state` instead of
  painting). Polls the file every 150 ms, re-renders on change. Layout:
  power emblem + "AginxOS" wordmark + 10-row checklist (KERNEL ROOTFS
  DISPLAY TOUCH BATTERY MODEM WLAN WIFI DHCP INTERNET) + BOOT COMPLETE
  banner. 5x8 string-art font embedded in the source; host-side layout
  verified via `bootcard --ppm` renders.
- `/bin/httpget` — minimal HTTP/1.0 fetcher (getaddrinfo → TCP → GET).
  Needed because **this busybox's wget applet segfaults** (observed
  2026-08-28: any URL, raw-IP included; DNS via nslookup is fine — same
  broken-applet family as its awk).
- `/etc/init.d/net-bringup` — waits for wlan0, reads `/etc/wifi.conf`
  (KEY=VALUE; real file lives on the device only, repo ships
  `wifi.conf.example`), joins with wifi-join, DHCP with udhcpc, then
  fetches `http://www.baidu.com/`. Join+DHCP retried once as a pair
  (a real boot lost DHCP to a busy AP for udhcpc's whole 30 s window —
  observed mid-session).

Panel facts learned today:
- **The scanout latches fb contents at SETCRTC time.** splash2 always
  painted *then* mode-set, so it never mattered; bootcard's first version
  mode-set an empty buffer and painted afterwards — panel stayed black
  with the backlight on while every ioctl succeeded. Render-before-
  SETCRTC is now structural in bootcard (`drm_prepare`/`drm_modeset`
  split).
- **A previous DRM master's exit unbinds the connector's encoder**
  (GETCONNECTOR returns encoder_id=0 with the compatible-encoder list
  still populated — splash2's bound-encoder-only filter then finds
  nothing: "no enabled-path connector" at t+2142 s). bootcard falls back
  to the first compatible encoder and rebinds via SETCRTC.
- **DRM_IOCTL_MODE_PAGE_FLIP is refused (ENOENT)** on this driver in the
  legacy SETCRTC configuration; bootcard falls back to re-SETCRTC every
  ~2 s to latch new frames. Good enough for the boot card; real flips
  need the atomic API.

Observed (cold boot, recorded on video 2026-08-28): logo ~60 s (panel
registration), then the card appears; rows flip green as
touch/battery 100%/modem/wlan0 report; wifi-join completes the 4-way
handshake unattended (`wifi ok Legrand AP`), udhcpc applies
192.168.0.166, httpget returns `HTTP 200 696249 bytes` off
www.baidu.com — `done ok`, BOOT COMPLETE banner on panel. Total
boot-to-internet ≈ 2 min. One earlier take lost DHCP (AP busy) and one
reboot landed in fastboot (slot retry counter — we never mark the slot
boot-successful, so every reboot burns one; `fastboot set_active a`
recovers. After ~7 reboots expect a fastboot stop).

Session-end device state: AginxOS M5 image on userdata (built from this
tree + live-pushed net-bringup retry patch, identical content), slot a
(set_active re-armed), our test vendor_boot (not stock), Wi-Fi config
present, card showing BOOT COMPLETE. Stock restore path unchanged.

## M6/M7: SIM proven over raw QMI; registration blocker is network-side; offline-mode wedge recovered via factory restore (2026-08-28)

All modem work driven by our own `/bin/qmi-req` (qmi-req.c: multi-message
single-client mode, `raw:` vendor-frame replay, `QR_SLEEP`, new `QR_TIMEOUT`,
1024-byte hex dump) on the AginxOS boot (ipacm + fake-sm per rcS, netmgrd
deliberately killed).

Modem = qrtr node 0. Service ports this boot: NAS 0:58, DPM 0:60, UIM 0:62,
WDS 0:63, WDA 0:65, DMS 0:77.

M6 — SIM detection (UIM GET_CARD_STATUS 0x2F): card PRESENT; apps CSIM +
USIM(type 2, READY) + ISIM; PIN1 DISABLED (not a PIN-lock problem); CT ICCID
89861114900206766670.

M6 — NAS state (Get Serving System 0x24 / Get System Info 0x4D): home network
460-11 "CT"; camping LTE band 3 (EARFCN 1850), TAC 0x4035, RSRP -106 → -98 dBm
after repositioning; **registration never completes**: reg state flaps 0/2
(NOT_REGISTERED / SEARCHING — correction: an earlier "REGISTERED_HOME" reading
was a misdecode of enum 2), cs/ps attach DETACHED, service status
LIMITED/LIMITED_REGIONAL. Network Register (automatic and manual 460-11)
accepted but ineffective; PS Attach → NO_EFFECT; no reject info TLV recorded.

M7 — WDS START_NET (bind mux + ip-family + APN ctnet, one client): CALL_FAILED,
verbose 0x07D1 = `WDS_VCER_CM_NO_SRV_V01` — consistent with no registration.

Self-inflicted offline wedge: DMS SET_OPERATING_MODE (0x2E) OFFLINE(3)
succeeded; afterwards ONLINE(0), LOW_POWER(1) and OFFLINE→SHUTTING_DOWN(5)→
ONLINE were all rejected with QMI_ERR_INVALID_TRANSITION (0x3C = 60). RESET(4)
is acked but no SSR happens (ports unchanged, mode still 3;
/sys/class/remoteproc is empty, so no host-side SSR either). GET_OPERATING_MODE
carries no offline_reason TLV. The wedge survived a full phone reboot. dmesg:
rmt_storage writes whole 2.5 MB modem_fs1/modem_fs2 (modemst) images every
~5 min — the offline state appears persisted in modem EFS. `adb reboot
bootloader` and on-device `reboot bootloader` from our rootfs both failed to
reach fastboot (a manual "bootloader" string written to misc + adb reboot also
did not); manual Power+VolDown worked.

Stock control experiment — factory flash-all (up1a.231105.001.b2 from
`.factory/`): bootloader/radio/boot/dtbo/vbmeta/vendor_boot flashed clean; the
2.7 GB product transfer dropped twice mid-stream with USB `e00002ed` (host
side; replug fixed — a 149 MB `fastboot stage` then ran in 4 s); `fastboot -w
update` re-run to completion. SIM LOADED, baseband g7250-00264-230619-B, LTE
band 3 PCI 250 TAC 16437, RSRP -98 dBm level 4, 中国电信, EHPLMN 46011/46003.
**Stock shows the same registration failure**: PS registration flips
IN_SERVICE (CHN-CT, with a complete data call at least once — rmnet_data2
10.146.156.235/29, DNS 218.2.2.2/218.4.4.4, default route, APN ctlte) to
OUT_OF_SERVICE within seconds (LOST_CONNECTION), then NOT_REG_SEARCHING;
emergency-only service; CS never registers (CT has no CS); IMS never attempts
registration (ImsRegistration null); rejectCause 0 throughout; airplane-mode
cycle reproduces it.

Conclusion so far: the M7 blocker is not AginxOS userspace — the network
admits attach then detaches this SIM/device. Next: verify the SIM's
subscription state (activation/plan) and test with an alternate SIM (CM/CU) to
separate SIM-account issues from CT policy toward this device.

Session-end device state: **full stock Android** (factory flash-all complete,
bootloader unlocked, slot a, fresh Android userdata — the AginxOS rootfs that
was on userdata is gone, rebuild with `scripts/build-rootfs.sh`; our test
vendor_boot must be re-packed/flashed to return to AginxOS). adb authorized.

### M6/M7 continued (2026-08-29): 5G already enabled; NR-only gives no service

Back on the AginxOS boot (rootfs rebuilt, test vendor_boot flashed, slot a, adb
`aginxosredfin`; netmgrd killed, modem driven by `/bin/qmi-req` only).

Race v1 (`/bin/m7-race`, fired on ps-flip while reg=02): the netmgrd-style setup
burst fails structurally — bind-mux 0x00A2, ip-family 0x004D, ind-register
0x0003, event-report 0x00AF all → QMI err **0x47** even on an idle modem, and
event-report 0x0001 → 0x11; START_NET 0x0020 during the blip → **0x0F**. The
0x47s need rild/DPM context and are unrelated to the data call (kernel side
verified fine: ipa3 loaded, `rmnet_ipa0` exists, dmesg `QMI_IPA_INIT_MODEV_
DRIVER_REQ` handshake OK).

Attach dynamics: ps attach blips (02→01→02, 1–2 s) roughly every ~40 s; reg
never reaches 01. LOW_POWER(2 s)→ONLINE radio cycle re-triggers attach attempts.
NAS GET_SYSTEM_INFO 0x40: only TLV 0x10 len 1 val 09 — with libqmi
QmiNasRadioInterface (verified via gh from linux-mobile-broadband/libqmi)
**09 = TD-SCDMA** (correction: an earlier session read this as "CDMA domain");
no LTE or NR serving TLV at all.

GET_SYSTEM_SELECTION_PREFERENCE 0x34 (295-byte response): **mode preference
TLV 0x11 = 0x005F — all RATs including 5GNR (0x40)**; disabled-modes TLV 0x22 =
0x0000; NR5G SA band mask (TLV 0x2C, 64 B) and NR5G NSA band mask (TLV 0x2D,
64 B) both populated; band pref 0x7FFF_FFFF_FFFF_FFFF; LTE band pref
0x01E7_FFDF_3FFF; network selection automatic; service domain = 2 (PS-only).
So the modem was never "stuck on LTE" — 5G is enabled by default config.

NR-only experiment (test "just use 5G, skip LTE"): SET 0x33 mode-pref 0x0040
accepted (res=0), radio-cycled, watched 90 s — serving-system radio-interface
TLV 0x11 = **0x00 NONE**, the PLMN TLV 0x12 (present whenever LTE cells were
visible before) is **absent**, reg stays 02; 0x40 unchanged. No CT NR cell is
even campable for this device/SIM, and NSA NR needs an LTE anchor — the exact
step CT kicks. Mode pref restored to 0x005F (readback verified `1102005f00`).

Conclusion unchanged and now triply confirmed (stock control + QMI observation
+ NR-only test): CT network-side policy toward this device is the blocker, not
RAT selection or our stack. `m7-race2` (reg=01-gated START_NET, APN ctlte,
480 s auto radio-cycle) left running in background; decisive next step remains
an alternate SIM (CM/CU).

### netmgrd-on-AginxOS tooling parked; raw QMI stays the control path (2026-08-28/29)

Second route attempted beside raw QMI: run the vendor data stack itself.
With fake-sm answering netmgrd's getService("netd"), a 4-byte Status::ok
reply gave it a wild sp<> → SIGSEGV in NetmgrNetdClientInit (observed
2026-08-28); after the null-binder fix netmgrd runs but its WDS setup
still returns 0x47s without the full rild/DPM context. Tooling built for
that route: sock-trace (socket-call tracer), netd-stub (HIDL netd
stub), crash-tracer + coredump-on (crash visibility without
tombstoned/core dumps), dsi-call (direct libdsi_netctrl client,
started, parked; its Qualcomm headers stay local per DECISIONS §7
spirit). All committed as experiment sources; the control path for M7
remains `/bin/qmi-req`.

### M6/M7 continued (2026-08-29): PDC carrier-config inventory — no CN config; START_NET 0x0F = CALL_FAILED

Correction first: PDC (QMI svc 0x24) lives at QRTR **0:28**, not port 76 — 76
is DSD (0x2A), and the "ctlte"/"sos" strings seen there earlier are the modem's
own APN-name table reported via DSD, not PDC config names.

With reporting registered (0x20, TLV 0x10=1) PDC answers asynchronously: a bare
ack, then a QMI **indication** (flags=04) carrying the token. `qmi-req` gained
`QR_DRAIN=<ms>` to keep reading after the response. This pinned the wire format
for all services: 7-byte header (flags u8, txn u8, msg id **big-endian** u16,
zero pad u8, TLV length **little-endian** u16), TLVs from byte 7, each TLV
`type u8 + len u16 LE` (14 = 7+7 response, 149 = 7+142 indication check out).

LIST_CONFIGS 0x24 (type=software) → 25 configs; GET_CONFIG_INFO 0x28 on each
(token-mapped from the indication):

| # | name | size B | # | name | size B |
|---|------|--------|---|------|--------|
| 1 | **WildCard (ACTIVE)** | 16324 | 14 | Singtel_Commercial | 54104 |
| 2 | SW_DEFAULT | 160168 | 15 | FarEastOne_Taiwan_Commercial | 57188 |
| 3 | WildCard_IMS | 48324 | 16 | ChunghwaTel_Taiwan_Commercial | 56580 |
| 4 | Ubigi | 17564 | 17 | PTCRB | 51604 |
| 5 | Global | 34788 | 18 | Xfinity | 100872 |
| 6 | TestSIM_IMS | 45416 | 19 | Visible | 122872 |
| 7 | TestSIM | 19816 | 20 | VoLTE_Videotron | 62588 |
| 8 | EIOTTestSIM_MTV | 61200 | 21 | hVoLTE-Verizon | 100788 |
| 9 | EIOTTestSIM | 51912 | 22 | Commercial-USCC-FI | 59460 |
| 10 | WildCard_APT (SEA) | 53336 | 23 | Commercial-USCC | 59312 |
| 11 | TStar_Taiwan_Commercial | 46800 | 24 | PublicMobile | 74044 |
| 12 | TaiwanMobile_Commercial | 60092 | 25 | Telus_Lab | 64524 |
| 13 | StarHub_Singapore_Commercial | 57180 | | | |

Active id (GET_SELECTED 0x22) = `54375ac9e15de4f582a033cc00082a35b628e715`
(entry 1). Every entry's TLV 0x15 carries its source path under
`/readonly/vendor/mbn/mcfg_sw/generic/Pixel/…`. **No China carrier config of
any kind is present** (generic + SEA + NA profiles = US-market firmware load),
so "activate the telecom MBN" has no target; the only move left on this lever
would be PDC LOAD 0x26 of an external CN mcfg_sw.mbn — and stock Android, with
the same WildCard active plus the full rild/IMS/CarrierConfig stack, was kicked
by CT as well, so a CN MBN is unlikely to change the outcome.

`m7-race2` overnight (2 h, radio cycle every 480 s): every cycle reopens a
registration window — reg=01 ps=01 for **~1 s** — and all three same-second
START_NET attempts return err 0x000F before reg drops to 02. Correction to the
earlier reading: **0x000F is QMI_PROTOCOL_ERROR_CALL_FAILED (15)** —
NETWORK_NOT_READY is 17/0x11, the error the netmgrd burst got. So the modem
does attempt the PDN inside the window and the network refuses it, then
detaches. While searching the modem reports current PLMN 460-11 "CT".

Session-end device state: AginxOS test boot, slot a, adb `aginxosredfin`,
m7-race2 self-expired, modem on CT limited service (reg=02, ps=01). No boot
image changes this session; stock restore points untouched.

### M7 WDS bind mystery solved: legacy 0x2F vs mux 0xA2 instances (2026-08-29)

The all-day "BIND_MUX_DATA_PORT refused MALFORMED" on fresh boots was **not**
DPM state, timing, or instance numbering — the WDS instance that boot simply
only implements the legacy bind:

- boot 19:54 (yesterday, call UP, 10.148.224.59): WDS instance accepted
  `0xA2 BIND_MUX_DATA_PORT` (bound at 0:62 that boot).
- boots after (0:63 instances): `0xA2` → MALFORMED 0x01 on every attempt —
  including seconds after a sock-traced netmgrd run whose DPM OPEN_PORT got an
  err-0 ack. **`0x2F BIND_DATA_PORT` (empty TLVs) → err 0 + handle TLV 0x01**
  on the same server. The DPM open is necessary-but-not-sufficient for 0xA2;
  0x2F needs nothing beyond a live WDS server.

Correction of the 08-29 dual-instance note: qrtr-lookup(svc 1, inst 0 =
wildcard) shows exactly ONE WDS server per boot. The "inst0@81" reading was a
misdecode — port 81 carries service **0x49**, not a second WDS. Similarly the
"netmgrd looks up inst0" claim: its lookup is svc 1 inst 0 (wildcard).

netmgrd traced end-to-end under the shim stack (rc=1): DPM GET_CAPABILITIES
0x22 → OPEN_PORT 0x20 (TLV 0x11: count 1, {4,1,2,16}) err-0 ack → rtnetlink
RTM_NEWLINK creates rmnet_data0 (mux 1, kind rmnet) → WDS wildcard lookup →
**exits before sending any WDS bind** this boot (on earlier boots it died at
the bind send). Only its DPM open matters to us; its WDS client never comes
up under the shims either way.

Two rootfs traps hit and fixed:

- `/run` is persistent ext4 (survives reboots) but the modem's DPM open dies
  with each modem boot — a `/run/m7-dpm.done` guard made cell-bringup skip
  netmgrd on a later boot and bind was refused all boot. Guard removed.
- sock-trace printed `q[2],q[3]` as node/port; sockaddr_qrtr is
  {family, node, port} = q[0],q[1],q[2] — earlier "qrtr node 60" lines were
  the port. Fixed.

DMS SET_OPERATING_MODE 0x2E takes a **u8** value TLV (`01 01 00 0X`); a u32
TLV is MALFORMED. (m7-race2's LOW_POWER↔ONLINE cycle bytes were right all
along.)

Current failure mode with the legacy bind is purely network-side:
START_NET → err 0x46 + call end reason 4 = **GENERIC_FADE**; NAS: reg 2
(searching), cs/ps 2 (detached). `m7-leg2` racer deployed (legacy bind + APN
ctlte + radio cycle every ~8 min = the m7-race2 window recipe).

## M7 root cause found: modem latched in DMS operating_mode 5 SHUTTING_DOWN (2026-08-29 evening)

The whole "WDS flavor" mystery of the afternoon has one root cause, found by
querying DMS GET_OPERATING_MODE (0x2D, svc 2) on the stuck modem:
TLV 0x01 = **05 = SHUTTING_DOWN** — the modem has believed the platform is
powering off since boot Y's spontaneous crash (~20:05). That single state
explains every refusal observed since: WDS BIND_MUX_DATA_PORT 0xA2, WDA
SET_DATA_FORMAT, DMS SET_OPERATING_MODE 0x2E (u8 AND u32 forms), and PDC
REGISTER all return err 0x0001 MALFORMED, while reads (NAS 0x24, UIM, DMS
0x2D), the legacy WDS bind 0x2F and DPM 0x20/0x22 keep working. Boot Y
(19:54, call up, 0xA2 + 0x2E accepted) was the last mode-0 modem.

Correction of the "network-side flapping" theory above: the ~1 s CT
registration windows were **self-made by the 0x2E LOW_POWER↔ONLINE cycles**
(m7-race2's recipe). With 0x2E refused (mode 5), reg stays 02 (searching)
forever — the m7-leg4 racer polled for 88 min without a single reg=01.

Levers proven this session:

- **Modem power-cycle without rebooting the AP** (new capability):
  `echo 0 > /sys/kernel/boot_cdsp/boot` = graceful subsystem_put shutdown;
  boot again by holding `/dev/subsys_modem` open
  (`setsid sleep 86400 < /dev/subsys_modem &` — the open is the
  subsystem_get that PIL-boots it). The echo-0 put is a NO-OP while
  pm-service/pm-proxy hold votes: kill them first, or state stays ONLINE.
  Holding fd open survives the adb shell that started it.
- Mode 5 **survives** (falsified as the store): subsystem power cycle +
  fresh PIL (state OFFLINE→ONLINE, WDS re-registers), pm-service/pm-proxy
  killed before the fresh boot, a full AP crash→fastboot→reboot cycle, and
  **a modemst1+modemst2 wipe + EFS rebuild from fsg** (rmt_storage served
  modem_fs2; whole qrtr layout shifted: WDS@68, DMS@82, UIM@67). Backups of
  the pre-wipe modemst1/2 in `.local/modemst-backup-20260829/` (md5
  14f2b92c…, a7eaad0f…), copies on device at /data/modemst*.pre-wipe.img.
  SIM still detected after the rebuild (UIM answers).
- qrtr port layouts alternate between boots (WDS 62 vs 63 vs 68) — they are
  registration-order jitter, NOT a flavor fingerprint; the 21:49 fresh boot
  came up in the "62" layout with mode still 5.

Second spontaneous crash: ~23:22 the device dropped to fastboot (slot retry
drained again; recovered with `fastboot set_active a` + reboot). The racer
was idle-cycling at that moment and reg had been 02 for 88 min — this crash
is NOT correlated with call attempts. pstore empty (no ramoops console), so
likely a hard SoC reset rather than a kernel panic. Unexplained so far.

Next: boot stock Android once (stock boot.img + stock vendor_boot) and check
whether its RIL stack registers / clears the mode — separates
our-environment from device-persistent state (SMEM/PMIC/PDC/protect stores,
which no lever above reaches).

## Stock-boot test: mode 5 survives factory wipe + cold power-off; HOLD-flag regression found and fixed (2026-08-29 morning)

Follow-up to the mode-5 latch. Sequence and observed results:

- **Stock boot loop recovered by `fastboot -w`.** Stock boot_a +
  stock vendor_boot_a (both md5-verified byte-identical to
  `boot/stock-*.img`) fell into the recovery rescue loop ("无法加载安卓系统")
  with slot-retry-count:a draining. `fastboot -w` (erase userdata + metadata)
  + `set_active a` fixed it on the first try — the blocker was persistent
  userdata/metadata state (rescue-party counters), NOT the firmware images.
  Side effect: the wipe destroyed the on-device backups
  `/data/aginx-test-*.img` (the only device-local copies of the 08-29
  vendor_boot; rebuild from tree works, so no loss) and our userdata rootfs
  (rebuilt from `scripts/build-rootfs.sh`).
- **Stock Android also fails to register.** After the wipe, stock booted
  clean (boot_completed=1, SIM LOADED) but telephony stays
  OUT_OF_SERVICE: modem camps CHN-CT LTE band 5 (earfcn 1850, full
  CellIdentity, emergency-capable) yet regState stays
  NOT_REG_MT_SEARCHING_OP_EM with rejectCause=0 — the network is not
  rejecting; the modem never completes attach. NitzStateMachine's latest
  network time is frozen at 08-28 23:20:54 (the second spontaneous crash):
  no successful registration since. This matches our-mode-5 behavior
  exactly, so the latch is not cleared by a full stock RIL boot.
- **Cold power-off does NOT clear it either.** `reboot -p` (full PMIC
  power cut, device confirmed off USB), user power-on, stock boot: same
  OUT_OF_SERVICE on the same cell. Mode 5 (or the equivalent registration
  block) therefore lives outside RAM/SMEM/PMIC-volatile state — remaining
  stores: modem flash config or network-side.
- Shell-level radio control is sealed on stock user builds: AF_QIPCRTR
  socket() from shell is SELinux-denied, airplane-mode broadcast and
  `cmd connectivity airplane-mode` do not reach RIL (no RADIO_POWER
  transitions in logcat).
- **Root cause of this morning's AginxOS bootloop: missing HOLD flag.**
  The trampoline execs aginxos-init (PID 1 takeover) only inside
  `if (exists("/aginxos/hold"))` — without HOLD it falls through to the
  Android handoff, and first_stage panics on our ext4 userdata → bootloop
  (retry drain, adb flapping in the ~t4-10 s trampoline window,
  /proc/1/comm=trampoline, no /var/kmsg-follow.log because rcS never ran).
  The documented M2 recipe (HOLD+SPLASH+USBADB+ROOTFS) had HOLD for a
  reason; vendor-boot.md's "safe default" wording understates it — ROOTFS
  boots REQUIRE HOLD=1. Fixed by repacking `HOLD=1 USBADB=1 ROOTFS=1
  KEEPADBD=1`: clean switch_root (kmsg "rootfs: exec /sbin/init"), stable
  past the 65 s modem-DOG window (uptime 190+ s at time of writing).
- Battery eliminated as a crash factor: fastboot battery-voltage 4447 mV,
  soc-ok yes, device on USB throughout.

Device state at this entry: running our stack — stock boot_a (identical to
`boot/stock-boot.img`) + vendor_boot-test (HOLD/USBADB/ROOTFS/KEEPADBD) +
userdata rootfs with radio-bringup enabled. Stock vendor_boot NOT currently
flashed. Next: query DMS operating mode from our OS (raw QMI available
there), then M7.

## flash-all + full modem-storage wipe: mode 5 still latched; Wi-Fi restored (2026-08-29 afternoon)

Continuation of the mode-5 hunt. All results observed on device:

- **SIM ruled out**: the CT SIM registers and works normally in a second
  phone (Huawei). Carrier/account/network are fine; the fault is in this
  device.
- **fsc wiped** (128 KiB, sdf5; backup `.local/fsc-backup-20260829.img`,
  md5 8c117e83…): modem power-cycled (boot_cdsp/boot 0 + /dev/subsys_modem
  holder), fresh PIL — DMS 0x2D still returns mode **5**.
- **radio (modem firmware) reflashed** from factory image
  (radio-redfin-g7250-00264-230619-b-10346159.img): still mode 5.
- **Full modem-storage factory reset**: fsg + modemst1 + modemst2 + fsc ALL
  zeroed (fsg backup `.local/fsg-backup-20260829.img`, md5 e2a61af1…,
  2,621,440 B), fresh PIL: EFS rebuilt from firmware defaults — still
  mode 5. fsg differs from the pre-wipe modemst at ~2.61M/2.62M byte
  positions (no poison sync from the wedged EFS into fsg).
- **Full factory flash-all** (bootloader r3-0.6 + radio + UP1A.231105.001.b2
  images + -w): stock boots to the setup wizard, SIM LOADED, but telephony
  remains OUT_OF_SERVICE. flash-all does not clear the latch either.
- Conclusion: the registration block survives zeroing every modem-writable
  partition we can reach, a full firmware reflash, and cold power-off. No
  remaining AP-side software store can hold it; either the block is
  computed at modem boot from a condition we cannot observe, or it is
  modem-internal state. Open question for the next phase: whether loading a
  CN-carrier mcfg (PDC LOAD) changes modem behavior — but PDC writes are
  refused while mode 5 holds.
- **Wi-Fi restored** (M5 regression fixed): the userdata wipes had deleted
  the device-only `/etc/wifi.conf`, and the recreated copy had the SSID
  misspelled — the real SSID is "Legrand AP" (ch 5). With the corrected
  config written on device: join ok, DHCP 192.168.0.166, baidu.com HTTP
  200. Boot card shows `done ok`. Reminder: wifi.conf must be recreated on
  device after any userdata wipe.

Device state: AginxOS (vendor_boot HOLD+USBADB+ROOTFS, userdata rootfs with
radio-bringup), modem wedged at DMS mode 5, Wi-Fi + internet up. fsg /
modemst1/2 / fsc currently zeroed (host backups in .local/).

### M6/M7 (2026-08-29 evening): qmi-req hexv bug voided a day of "refused" results; mode 5 is just the boot default; CN config path opened

**The "unkillable mode-5 latch" never existed.** `qmi-req`'s `hexv()` had an
inverted range check (`c>='0'&&c>='9'`) that mapped every hex digit 0-8 to
-1, so every hex TLV arg went out as `ff` bytes and the modem answered
MALFORMED 0x0001. Every qmi-req write since the tool gained hex args was
corrupted — DMS mode-sets, WDS 0xA2 binds, PDC register/select — and the
"mode 5 refuses all writes" conclusion (plus the modemst/fsg/fsc-wipe and
flash-all falsifications built on it) was chasing our own tool bug. Reads
(empty TLV) were unaffected, which is why mode 5 *readings* were real.
Fixed in f07ea11. pdc-load (raw byte path) was never affected.

With a correct parser, observed on device:

- DMS 0x2E SET ONLINE/LOW_POWER **accepted**; mode 5 (SHUTTING_DOWN) is the
  modem's boot default, not a latch. Mode holds 0 (ONLINE) once set.
- PDC LOAD 0x26 works under any operating mode. Duplicate config id →
  err 0x29 (INVALID_ID). A Xiaomi-signed mbn (Redmi K30 5G/picasso CT
  config) was **accepted cross-OEM** — PDC LOAD verifies hashes, tolerates
  foreign signature chains.
- PDC SET_SELECTED 0x23 / ACTIVATE 0x27 accepted; selection survives
  reboot; activation applies across the next modem SSR.

**Xiaomi CT config crash-loops redfin's modem.** With the picasso CT
Commercial VoLTE config selected+activated: `Fatal error on modem!` x5,
each at RAT bring-up after DMS ONLINE, each followed by an SSR. Root cause
(config diff via sbaresearch/mbn-mcfg-tools): the CT config is built for
China Telecom's **CDMA** legacy — policy rat_capability `C H L 5G`,
ue_mode 1X_CSFB_PREF, ~60 CDMA/1x/HDR/TDS NV items and EFS files —
but redfin's US modem build has the CDMA/EV-DO stacks removed, so the
config commands bring-up of absent protocol stacks → err_fatal. Rolled
back to WildCard (crash loop stopped immediately). Also observed: the
Pixel's own WildCard has **zero 460-xx policy rules, no APN profiles at
all, and rat caps at G W L (no 5G)** — CT has nothing to attach with.

**modemst1/2 + fsg + fsc restore.** The 08-29 zeroing had left the modem
on blank NV (flash-all does not touch these partitions). Restored all four
from .local/ backups (md5-verified after dd). Immediately afterwards the
modem registered on CT again: reg=01 windows of ~12-19 s with PS attach,
PLMN 460-11 "CT" visible — the first full registrations since boot Y.
SW_DEFAULT (160168 B generic) selected+activated instead gave steady
LIMITED service, no windows — worse than WildCard; reverted.

**Patched WildCard+CT config (no crash, profiles live, PDN still
refused).** Built `out/mcfg/patched-wildcard-ct.mbn` (17140 B, sha1
1990e8b0…98bab): WildCard base + CT policy rule (serving/IMSI 460-03,
460-11, MCC 460 → rat_capability G W L 5G, ue_mode NORMAL) + CT data
profiles from the Xiaomi config (Profile0/1 ctnet, Profile3 ctwap), zero
CDMA-stack items. Loaded/selected/activated cleanly, **no fatals**. WDS
GET_PROFILE_SETTINGS idx 0 now returns APN `ctnet` — the profile is live
in the modem. Registration windows still occur (one opened within seconds
of DMS ONLINE), but WDS START_NET inside the window is refused with
err 0x46 (+ call_end_reason TLV 0) for **all** variants tried: 3GPP
profile-index TLV 0x31, bare no-APN, APN ctnet, APN ctlte. Windows last
~12-19 s then reg drops to 02.

Working hypothesis for the refusal: the network rejects the *attach*
itself (EMM cause unknown — being captured via NAS serving-system
indications during the next window drop), so the transient reg=01 is a
registration that never fully settles; the PDN refusal is downstream of
that. Boot Y (same WildCard, pre-wipe) proves CT *does* occasionally
accept this device and complete a data call, so acceptance is
stochastic/network-side, not a hard block.

Session-end device state: AginxOS boot, slot a, adb aginxosredfin;
modemst1/2+fsg+fsc restored from backup; PDC selected = patched
WildCard+CT (rollback = SET_SELECTED 54375ac9…e715 WildCard); DMS ONLINE
set manually after each boot/SSR (no auto-set wired yet); m7-v4 racer +
NAS indication watcher running on device; Wi-Fi unaffected. qmi-req
binary in the rootfs tree refreshed with the hexv fix.

### M6/M7 (2026-08-29 night): attach PDN was dangling → ctlte profile written; PS attach is *steady*; drop is CS-only; stock parity reached

TLV decode session against libqmi service jsons (`/tmp/nas-real.json`,
`/tmp/wds.json`, `/tmp/qmi-enums-wds.h` — ikto-art GitHub mirror):

- NAS Get System Info (0x4D) TLV 0x19 LTE System Info v2 while camped on
  CT: domain=PS, capability=PS, **roaming=home, forbidden=0, reject-info
  NOT valid**, PLMN 460-11, TAC 0x4035. So no EMM reject is being reported
  most of the time.
- **Attach PDN list was dangling**: WDS Get LTE Attach PDN List (0x94)
  showed current list = [profile 1], but 3GPP profile 1 did not exist
  (the patched mbn had written ctnet to *3GPP2* profile 0, not 3GPP 1).
  Wrote 3GPP profile 1 = APN `ctlte`, PDP IPv4v6 via WDS Modify Profile
  0x28 (verified via Get Profile Settings 0x2B TLV `01 02 00 00 01`).
  Get LTE Attach Parameters (0x85) afterwards: **attach now runs with
  APN `ctlte`** — profile/attach wiring matches stock Android's call
  (boot Y used APN ctlte).
- NAS Network Reject indication (0x68, enabled via Indication Register
  0x03 TLV `21 02 00 01 00`) fired exactly once in ~1 h of cycling:
  **radio=LTE, domain=CS, cause 0x12 (EMM #18 "CS domain not
  available")** — CT rejects the combined attach's CS leg. Set NAS
  0x33 service-domain=PS-only + usage=data-centric (verified via 0x34
  readback: TLV 0x18 = 1). No more rejects since.
- **PS attach is steady, not flapping**: 90 s of 2 s-interval NAS 0x24
  polls show `ps=1` continuously. What cycles is the *CS* registration
  (`reg=2 searching`) and the QMI LTE service-status byte (01 limited ↔
  02 available for ~4 s). The earlier "network drops us every 75 s"
  reading conflated CS search with PS detach.
- START_NET (0x20) still returns **err 0x46 = INVALID_OPERATION**
  (libqmi qmi-errors.h enum) with call_end_reason 0, for bare/APN/
  profile-index/IP-family variants, with legacy bind 0x2F succeeding
  first on the same client. Bind Mux Data Port 0xA2 (ep embedded iface 0
  mux 1) → err 0x03 INTERNAL; WDA Set/Get Data Format (0x20/0x21 at
  svc 0x11) → err 0x10 NOT_PROVISIONED (unused on this platform).
  Kernel side: rmnet mux channels unsupported (`ip link add type rmnet`
  → RTNETLINK Not supported); rmnet_ipa0 exists (operstate down).

**Interpretation.** AginxOS QMI state now matches what stock Android had
when its call succeeded (ctlte attach PDN, PS attached, home PLMN, no
reject). The stock control experiment already recorded the same
admit-then-drop behavior with a complete data call occurring at least
once — so the remaining blocker is network/SIM-side admission
probability, not our stack. `m7-v5` racer on device retries
START_NET(APN=ctlte, IPv4) every 20 s indefinitely and configures
rmnet_ipa0 + route + resolv.conf + `cell ok` in /run/boot.state if a
call lands. Open questions for the user: CT SIM subscription/data-plan
state, whether this SIM gets stable data in another phone, and whether a
CMCC/CU SIM is available for the control test.

Session-end device state: AginxOS boot, slot a, adb aginxosredfin; PDC
selected = patched WildCard+CT; attach PDN list = [1] (ctlte, no auth);
NAS service-domain PS-only (permanent); DMS ONLINE; m7-v5 racer +
sysinfo poller + network-reject watcher running; Wi-Fi unaffected.

### M6/M7 (2026-08-29 late night): official CT OpenMkt mbn active; NR still absent; START_NET 0x46 unchanged

User confirmed the same SIM registers 5G on a Huawei phone (China
Telecom, same location), so SIM/plan/coverage are fine. Tested whether
the Pixel 5 side is a carrier-config problem:

- **Switched PDC active config to the official `China_CT_Commercial_OpenMkt.mbn`**
  (sha1 `9f3f896773baff333a5981cc608a3e6ed4871cd8`, already in the
  device PDC store) via Set Selected 0x23 + Activate 0x27, verified
  active after reboot. This replaces our patched wildcard — the exact
  config stock CT devices ship.
- Under the official CT config: mode pref 0x5F, DMS online, LTE camps on
  460-11 TAC 0x4035. Same as patched wildcard.
- **Network scan under CT config: 3 PLMNs (460-11 CT, 460-00 CMCC,
  460-01 UNICOM), all RAT 0x08 LTE — zero NR cells**, identical to the
  wildcard result. Scan type 0x10 (5GNR) and 0x14 (LTE+NR) both rejected
  with err 0x30 InvalidArgument — this firmware's Network Scan 0x21
  does not accept the NR bit at all.
- **NR-only retry under CT config**: mode pref 0x40 applied (readback
  confirmed), radio cycled, 90 s of polls all reg=02 no-service; NAS
  0x4D NR5G svc status TLV 0x4A = `04 00 00`. Restored 0x5F.
- NAS 0x34 NR5G SA/NSA band masks (TLV 0x2C/0x2D) contain n78 under
  both configs — the config side is *not* filtering out CT's band.
- PS-only service domain (TLV 0x18 = 2, set last night) **persists
  across reboot**.
- START_NET (ctlte) under the official CT config: still **err 0x46
  INVALID_OPERATION**, call_end_reason 0 — unchanged.
- After every boot the modem comes up in DMS operating mode 05
  (low power) under the CT config; DMS 0x2E online is required
  manually. Registration afterwards: PS attach steady, CS searching
  (expected, PS-only set).
- m7-v7 racer running on device (START_NET ctlte every ~32 s forever,
  auto rmnet_ipa0 + route + resolv.conf + `cell ok` on success).
  WDS 0:62 NAS 0:57 DMS 0:77 this boot.

**Conclusion.** Pixel 5 hardware supports CT 5G (n78 in band masks,
public specs) and neither the wildcard nor the official CT mbn exposes
any NR cell here, while a Huawei (a CT-certified/入库 device) gets 5G at
the same spot. Together with the stock-Android control showing the same
admit-then-drop LTE behavior, this points to network-side admission
(CT 5G SA terminal whitelist), not to any config file we can change.
No further mbn/policyman edit is indicated; remaining lever for data is
catching a stochastic LTE admission window (racer running), or a
CMCC/CU SIM control (user has only CT).

Session-end device state: AginxOS boot, slot a, adb aginxosredfin; PDC
selected = official China_CT_Commercial_OpenMkt (`9f3f8967…1cd8`,
rollback = SET_SELECTED `54375ac9…e715` WildCard); mode pref 0x5F;
service-domain PS-only; DMS set ONLINE this boot; m7-v7 racer running;
Wi-Fi unaffected.

## M8: phone joins the aginx network — full relay round-trip to the on-device clone (2026-08-29)

Acceptance: from the Mac, `agc agent://cf49973e.relay.aginx.net/clone-creator
"只回：ok"` over the public relay, answered by the clone running on the
phone. Observed:

- aginx gateway + aginx-carrier cross-built `aarch64-unknown-linux-musl`
  (cargo-zigbuild), fully static: aginx 4.8 MB, aginx-carrier 29 MB.
- **rustls port required**: gateway used native-tls (OpenSSL, musl-hostile).
  Ported relay client + ACP relay client + test example to tokio-rustls 0.26
  / rustls 0.23 (ring) / webpki-roots (aginx repo, 2 call sites + 1 example).
  **Gotcha observed on device**: tokio-rustls 0.26 default features pull
  `aws-lc-rs`; with ring also enabled rustls panics at runtime ("Could not
  automatically determine the process-level CryptoProvider"). Fix:
  `default-features = false, features = ["ring", "tls12", "logging"]`.
  aws-lc-rs then leaves Cargo.lock entirely.
- Installer v0 (`agpkg`, shell): sha256-verify → stage `.name.new` → atomic
  rename into `/var/bin`, keeps `.name.prev` for rollback. Both binaries
  installed through it on device.
- Gateway registers with relay.aginx.net:8443 (TLS handshake confirmed,
  ESTABLISHED held; 106.75.32.216). Requires correct clock — `ntpd -q -n -p
  ntp.aliyun.com` after net-bringup (busybox `date -s` misparses GNU
  format; it once set the year to 2029).
- Owner binding flow works headless: `aginx pair` on device (code, 300 s
  TTL) → `agc --bind <code>` on Mac (token into ~/.aginx/agc/tokens.json).
- **Env propagation matters**: the ACP child (`aginx-carrier acp`) is
  spawned by the *gateway*, so `AGINXBRAIN_API_KEY` must be in the
  gateway's environment, not just the carrier daemon's. `/etc/aginx/env`
  (0600) + `set -a; . /etc/aginx/env; set +a` in aginx-services. First
  attempts failed with "LLM driver error: Auth error" exactly because of
  this (and once because `. env` without `set -a` doesn't export).
- Round-trips observed: first turn answered `ok`; `--session` resume turn
  correctly recalled the previous instruction (answered `ok` again).
  sessionId ed8005a4-…. clone-creator + me seeded; registrations
  auto-written with self-locating `[command] path = "/var/bin/aginx-carrier"`.

Session-end device state: AginxOS boot, slot a, adb aginxosredfin;
/var/bin/{aginx,aginx-carrier,agpkg} installed via agpkg; /etc/aginx/env
(0600, brain key); /var/home/.aginx (gateway id cf49973e, private access);
gateway + carrier running under setsid, logs /var/log/aginx{,-carrier}.log;
/etc/init.d/aginx-services + rcS hook pushed (autostart next boot);
m7-v7 racer still running; Wi-Fi on "Legrand AP" (192.168.0.166).

## M9: aginxbrowser runs on device — v8 (150.4) renders JS on musl (2026-08-29)

Acceptance: the full server-side browser engine, static musl, serving its
HTTP API on the phone. Observed:

- aginxbrowser repo did the port (their main @ 8a52027): deno_core 0.350 →
  0.411 (v8 137 → 150.4 — first with official
  `librusty_v8_simdutf_release_aarch64-unknown-linux-musl.a.gz` prebuilt),
  deno_error unified to 0.7. stealth/screenshot stayed opt-in, off.
- Build: `cargo zigbuild --release --target aarch64-unknown-linux-musl` →
  61 MB static stripped ELF. **Build-host gotcha**: `RUSTY_V8_ARCHIVE`
  applies to BOTH the target build and the host-side snapshot build script
  (build-dependencies deno_core), so pointing it at the musl .a breaks the
  host link ("archive member not a mach-o file"). Working recipe: serve a
  local mirror dir structured `<base>/v150.4.0/{librusty_v8_*.a.gz,
  src_binding_*.rs}` (bindings .rs files are downloaded too!) and set
  `RUSTY_V8_MIRROR=http://127.0.0.1:18742`. GitHub-release downloads of the
  37 MB archives stall on this network; gh-proxy.com mirror sustained
  ~800 KB/s.
- Installed via agpkg (sha256 7fe02343…d000). `--help` starts the server
  instead of printing help — there is no help text; default bind
  0.0.0.0:8089 (AGINXBROWSER_BIND overrides).
- On-device API results (via adb forward):
  - `/health`: ok, engine diting, capabilities screenshot:false
    stealth:false (as designed)
  - `/fetch https://example.com` (tier http): title + markdown body
  - `/fetch https://quotes.toscrape.com/js/ render_tier=obscura`: **v8
    executed page JS on musl** — quote list present (absent without JS)
  - `/search` (field is `q`, not `query`): bing+sogou aggregated, 69 hits
  - `/session/create` → s_1, `/session/s_1/eval document.title` →
    "Example Domain" (live multi-step session)
  - `/mcp` responds (streamable-HTTP handshake requires session id —
    endpoint alive)
- Boot wiring: aginxbrowser added to /etc/init.d/aginx-services (optional
  start, logs /var/log/aginxbrowser.log).

Session-end device state: AginxOS boot, slot a, adb aginxosredfin; aginx
gateway + carrier restarted after reboot (relay ESTABLISHED); aginxbrowser
(pid, port 8089) running; /var/bin now holds aginx, aginx-carrier, agpkg,
aginxbrowser; Wi-Fi "Legrand AP" 192.168.0.166; clock ntpd-synced after
boot (gateway's relay loop needs the clock BEFORE TLS validates).

## M10: first-boot provisioning pipeline — agdl + agpkg sync + ntpd in init (2026-08-30)

Acceptance (partial — everything except the full userdata-wipe restore,
which waits on the release pipelines; see end):

- `agdl` (new Rust crate, ureq+rustls, 2 MB static): TLS download on device
  verified — 6.9 MB from GitHub releases via gh-proxy.com. Streams to
  `.part` + rename, so a killed download never leaves a truncated target.
  This is now the phone's only working HTTPS fetcher.
- `agpkg sync [manifest]`: manifest lines `<name> <url> <sha256>`; per entry
  compares installed sha256, downloads on missing/stale, verifies, atomically
  installs (reuses the v0 install path, keeps `.prev`). Device tests:
  missing → installed; corrupted → restored to pinned sha (self-heal);
  clean → "up to date" no-op. Repo manifest `/etc/agpkg.manifest` ships with
  all entries commented until the musl release assets exist.
- `/etc/init.d/provision`: waits for net-bringup's `internet ok` in
  /run/boot.state (≤6 min), then `agpkg sync`; reports `pkg run|ok|fail`
  to the boot card. Ran clean on device (rc=0, state lines present).
- net-bringup now ntpd's (`ntpd -q -n -p ntp.aliyun.com`) right after the
  internet check, reporting `time run|ok|fail`. Verified across a reboot:
  boot.state shows `time ok 2026-08-29`; `date` agrees. This closes the
  "1970 clock breaks TLS" class for good.
- **Push-workflow trap**: `adb push` over an existing init.d script lands
  0644 — the second aginx-services push silently broke boot autostart
  (rcS got "Permission denied", zero services came up). build-rootfs.sh
  chmods correctly; on-device pushes of scripts must `chmod 755` after.
  (Also: `adb reboot` hangs with our initless rcS — use fastboot or
  sysrq-trigger. Two unplanned hard reboots this session drained the slot-a
  retry counter into "no valid slot to boot"; `fastboot set_active a`
  recovered, as before.)
- Full-boot chain after `fastboot set_active a` + reboot: touch/battery/
  modem/wlan/wifi/dhcp/internet/time/done all ok, then aginx+carrier+
  aginxbrowser up (relay ESTABLISHED, :8089 LISTEN) — after the chmod fix.

**Not done**: `userdata 清空 → 自动恢复到完整节点` needs the release
pipelines to publish musl assets (aginx, aginxbrowser on GitHub) and a
public source for aginx-carrier. Until then a wiped device reinstalls via
adb + `agpkg install` (the v0 path), which is exactly what we did for
M8/M9.

Session-end device state: AginxOS boot, slot a (retry counter refreshed),
adb aginxosredfin; /etc/init.d/{provision,net-bringup,aginx-services} and
/usr/bin/{agpkg,agdl} at their M10 versions (0755); all three services
running; Wi-Fi "Legrand AP" 192.168.0.166; clock ntpd-synced at boot.

## M11: aterm — on-device terminal (launcher + pty shell + touch keyboard) (2026-08-30)

New crate `crates/aterm` (378 KB static musl, deps: libc + vte). The panel
UI: bootcard hands the panel to aterm once boot finishes.

- DRM layer is a faithful Rust port of bootcard.c's proven msm_drm 4.19
  path (raw ioctls; zero count_fbs/encoders on the second GETRESOURCES,
  skip mode-less connectors, prefer DSI type 16, fall back to first
  compatible encoder when the previous master released the binding).
- Terminal: vte parser -> own cell grid (90x70 at glyph scale 2 on
  1080x2340), 400-line scrollback ring, SGR mapped onto the fixed phosphor
  palette (black bg, green normal, white = bright, inverse supported).
  busybox sh runs under openpty (setsid + TIOCSCTTY), verified live:
  `pidof sh` shows the pty child, typing echoes, commands run.
- Keyboard: on-screen ASCII (evdev /dev/input/event2, type-B protocol).
  SHF = one-shot shift, SYM = one-shot symbol page (30 shell punctuation
  chars), SPC/DEL/ENT/ESC. Tap log on device confirmed correct byte decode
  (letters, shifted digits). Drag on the terminal area scrolls scrollback.
  v1 has no TAB key.
- Launcher: CLONE/codex/grok/sh buttons (missing binaries shown dimmed,
  only SH installed so far); toolbar SH/BACK strip while an app runs;
  child exit -> back to launcher. Verified on device by hand.
- Handoff: rcS records bootcard's pid in /run/bootcard.pid (NOT /var/run —
  that dir doesn't exist on this rootfs; the first attempt wrote into the
  void and bootcard was never killed, aterm crash-looped on SETCRTC).
  /etc/init.d/aterm-handoff waits for `done` in /run/boot.state (5 min
  hard cap), kills bootcard, runs aterm under a respawn loop.
- **Buffer-latch trap**: the panel snapshots the fb at SETCRTC, and the
  modeset must latch the SAME dumb buffer the first frame was painted
  into. First version painted the back buffer but latched fb[0] — static
  launcher stayed black while interactive sessions eventually relatched
  (that difference is what made the manual ATERM_START=sh test pass while
  the boot path showed a dead screen). Fixed: initial_modeset latches the
  painted back buffer; PAGE_FLIP is refused on this driver (same as
  bootcard observed), so every present() re-SETCRTCs — event-driven, no
  visible lag typing.
- Host verification: `aterm --ppm out.ppm` renders launcher + terminal
  frames to PPM off-device (same pattern as bootcard --ppm).
- Manifest activated: aginx v0.3.2 and aginx-carrier v0.1.0 musl release
  assets are published on GitHub; /etc/agpkg.manifest entries are now
  live. `agpkg sync` on device downloaded both (sha256-verified, atomic
  install, .prev kept) and the stack restarted clean on the release
  binaries (relay ESTABLISHED, carrier daemon ready). aginxbrowser's musl
  asset is still pending — its manifest line stays commented.

### M11 polish (2026-08-30): keyboard/layout/performance round, verified on device

All results below observed on device this session.

- Glyph scale 5 (30x40px cells, 34 cols) is the accepted terminal size,
  with an 8 px gap between text rows (48 px stride — 40 px rows read as
  cramped); scale 9 (~20 cols) rejected as too big, scale 6 (~28 cols) a
  bit too few columns, scale 2/3 far too small.
- Input lag root-caused in two parts: (1) the renderer repainted the full
  canvas + re-SETCRTC per frame — fixed with per-row damage tracking
  (Term.row_dirty) so terminal() repaints only dirty rows, plus a
  persistent CPU canvas memcpy'd into the back buffer per present
  (~10 MB ≈ 1 ms); (2) keys fired on finger-up — moved all keys, launcher
  buttons and toolbar BACK to fire on Touch::Down (emitted at the
  tracking-id-down SYN). Echo fast-path: after a key write, poll the pty
  master 15 ms and parse the echo into the same frame = one present per
  keystroke. Result: typing fast, launcher opens on first tap.
- Keyboard layout borrowed from Termux (ExtraKeysConstants): slim
  extra-keys row above the keyboard (ESC TAB CTL arrows), hold-repeat on
  DEL/arrows at 400 ms delay / 60 ms interval. Letter rows staggered
  QWERTY-style (row offset = cell_w * row / 4), keycap labels at ~half
  cap height with side margins (28 px) so it reads as a real keyboard.
- Runaway-repeat bug: a >30 px drag suppressed the Tap event that cleared
  the held key, so DEL kept repeating and ate the whole line. Fixed:
  Touch::Up is now emitted on every finger-up (not only as Tap), and Drag
  cancels the held key.
- Header: launcher-style strip with only BACK at right (centered title
  was tried and rejected — it occluded content); terminal content starts
  below a 20 px gap.
- Launcher-remnant bug: the AGINXOS title (drawn at y=86, scale 5) left a
  squished 6 px sliver visible under the SH header separator (y=72),
  because row-damage rendering only repaints terminal rows from y=92.
  Fixed: full-canvas BG clear below the header on Launcher→Running
  transition. Verified clean on device.


### agdl stdout mode (2026-08-30)

`agdl <url>` with no output argument (or an explicit `-`) streams the
response body to stdout — plain `agdl <url>` is the "view a page" command
(status line moved to stderr so pipes stay clean). `-`/stdout skips the
`.part` file dance. Verified on device over Wi-Fi: `agdl https://example.com -` prints
the full document, file mode still downloads+renames atomically
(559 B example.com variant, no `.part` residue). On-device note: the adb
shell's PATH lacks /usr/bin — invoke as /usr/bin/agdl there.

### aterm: keyboard on demand (2026-08-30)

The keyboard no longer sits on screen permanently — it starts hidden
(launcher shows no keyboard at all now) and a tap in the terminal area
summons/dismisses it. Row count follows the visible area (~47 rows
hidden vs ~25 shown at scale 5): Term.resize_rows shifts top lines into
scrollback when shrinking (capped so the cursor row stays visible) and
the child gets TIOCSWINSZ/SIGWINCH. Verified on device: toggle works
both ways, typing after summon works, drag-scroll still works with the
keyboard hidden (drag region now spans the full terminal area).

## M12: codex official musl binary on device (2026-08-30)

- Root cause of "no space": the rootfs image (build-rootfs.sh SIZE=512m)
  is a 487 MB ext4 written onto sda19 (userdata, 114 GB) — the filesystem
  simply never grew to the partition. Fix on device: cross-built a static
  musl resize2fs (e2fsprogs 1.47.0, zig cc; had to force HAVE_LSEEK64 and
  skip the uuid test binaries), online-resized / to 107 GB. NOTE: any
  re-flash of userdata reverts to ~487 MB until resized again.
- codex-cli 0.151.0 (codex-aarch64-unknown-linux-musl.tar.gz from the
  openai/codex rust-v0.151.0 release, binary sha256 56f02601...6ce76,
  223 MB static) installed at /var/bin/codex (the launcher's expected
  path). `codex --version` runs clean on device. CODEX button now live.

### M12 verified (2026-08-30): codex -> brain.aginx.net round-trip works

- codex auth/config copied verbatim from the host ~/.codex (provider
  aginxbrain, base_url https://brain.aginx.net, wire_api=responses,
  model gpt-5.5, requires_openai_auth) into /var/home/.codex/ — key lives
  on device only, never in the repo.
- First `codex exec` hung in "Reconnecting... waiting for network":
  root cause was the missing TLS trust store (no /etc/ssl on the phone —
  codex reads system-native roots; our own agdl compiles webpki-roots in,
  which is why agdl worked). Fix: Mozilla cacert.pem installed at
  /etc/ssl/certs/ca-certificates.crt; build-rootfs.sh now bakes it into
  the image and SIZE grew 512m -> 2g so a re-flash fits codex.
- After the CA install: `codex exec --skip-git-repo-check` on device
  answered "pong" (9k tokens) through brain.aginx.net. Codex TUI from the
  launcher renders correctly in aterm. bubblewrap missing warning is
  benign (codex falls back to its bundled bwrap).

## M13: grok (xai-org/grok-build) source-built musl on device (2026-08-30)

- No official musl/arm64 release exists, so grok 1.0.12 (bc7f02eddd3d)
  was source-built for aarch64-unknown-linux-musl via cargo-zigbuild
  (toolchain pinned 1.94.0, ~41 min build). Build fixes that were needed:
  - sqlite-vec.c uses u_int8_t/u_int16_t/u_int64_t which musl only exposes
    via <sys/types.h>; fixed with CFLAGS macro self-typedefs
    (-Du_int8_t=uint8_t ...). `-include sys/types.h` breaks aws-lc-sys
    .S assembly, -D_DEFAULT_SOURCE alone is not enough.
  - jemalloc symbols are undefined under zig cc: build with
    --no-default-features --features sandbox-enforce.
  - CARGO_BUILD_JOBS=1 avoids spurious empty-stderr cc-rs failures.
  - rg/fd bundling pointed at local binaries via GROK_TOOLS_BUNDLE_RG_PATH
    / GROK_SHELL_BUNDLE_RG_PATH / GROK_TOOLS_BUNDLE_FD_PATH (the
    auto-download picks the gnu triple and stalls).
- Result: 172 MB static binary, installed at /var/bin/grok (the aterm
  launcher's expected path). `grok --version` runs clean.
- Brain wiring (same gateway as M12 codex): /var/home/.grok/config.toml
  sets [endpoints] xai_api_base_url + models_base_url to
  https://brain.aginx.net, [features] remote_fetch = false (the catalog
  fetch otherwise hits a hardcoded 5 s STARTUP_FETCH_TIMEOUT against
  brain), [models] default = gpt-5.5, and a [model."gpt-5.5"] override
  (api_backend = "responses", context_window 272000,
  inference_idle_timeout_secs = 600 — brain is slow, codex needed the
  same). Auth: /var/home/.grok/auth.json, scope "xai::api_key",
  auth_mode "api_key", key extracted on-device from the codex auth —
  key lives on device only.
- Verified round-trip: `grok -p "reply with the word ok"` on device
  returned "ok" via brain.aginx.net (ttft 789 ms, 11.2k input tokens,
  stop_reason "stop", clean headless exit). brain ignores streaming and
  answers a single JSON body even for accept: text/event-stream — grok
  tolerates it. NOTE: an earlier "hang" against brain was not grok at
  all — wlan0 had dropped to NO-CARRIER (the known Wi-Fi drop issue);
  after re-running net-bringup the same request completed in seconds.
- agdl gained curl-lite flags (-X/-H/-d @file, ureq 3 API: Agent with
  http_status_as_error(false)) so the device can probe HTTPS endpoints
  directly; used it to prove GET /api-key (200, SPA fallback HTML) and
  POST /responses (200 in ~6 s) from the phone.
- aterm adaptations for PC-designed TUIs (user-verified on device):
  glyph scale is now per-app — sh keeps scale 5 (34 cols), codex/grok
  run at scale 3 (~56 cols) so their 80-col layouts fit; scale resets
  to 5 when the child exits. Terminal cells no longer truncate Unicode
  codepoints to 7-bit font indices: box-drawing/block/shade/arrow and
  braille (spinner) glyphs render procedurally in the 5x8 bitmap format
  via glyph(). grok TUI confirmed usable on device after both fixes.
  CJK/wide chars still unsupported (needs double-width cells + a CJK
  font — future milestone).
- agpkg sync retries GitHub release URLs through gh-proxy.com when the
  direct connection is TLS-reset (observed on the device network;
  sha256 pinning keeps transport non-load-bearing). Manifest grew to
  five entries (aginx, aginx-carrier, aginxbrowser, codex, grok) and
  `agpkg sync` ran clean on device for all five — codex/grok are
  mirrored as raw musl binaries under yinnho/aginxos releases.

## M10 device acceptance: full wipe → unattended re-provision (2026-08-30)

Headless-path acceptance (the adb-fed variant; the scan/password wizard is
the UI half, still to build). Observed on the experiment unit:

- Backed up /var/home (1 GB) + /etc/wifi.conf over adb, then flashed a
  fresh rootfs.img (`fastboot flash userdata out/rootfs.img`) built from
  current master — the full factory reset path.
- One `no valid slot to boot` on the second reboot: the retry counter
  drained again (same as the 2026-08-27 incident); `fastboot set_active a`
  recovered it. Two reboots in quick succession after a userdata flash are
  enough to burn it.
- First boot on the wiped userdata: touch/battery/modem ok; aginx,
  aginx-carrier, aginxbrowser correctly "absent" (/var/bin is empty);
  net-bringup correctly `wifi fail no /etc/wifi.conf` + `done fail`.
- Pushed wifi.conf over adb and rebooted. Fully unattended boot then:
  wifi ok → dhcp ok → internet ok → ntpd time ok (2026-08-30) →
  provision `agpkg sync` downloaded and sha-verified ALL FIVE manifest
  entries into /var/bin (aginx 4.5M, aginx-carrier 27M, aginxbrowser 61M,
  codex 223M, grok 164M) → `pkg ok`. ~470 MB through the phone's Wi-Fi.
- Download-path observations:
  - Direct github.com release downloads throttle to ~40 KB/s on this
    network (aginxbrowser stalled completely once for 20+ min — no bytes
    to disk, connection ESTABLISHED). Killing the stalled agdl makes
    agpkg's built-in gh-proxy.com fallback take over; gh-proxy sustained
    ~1-4 MB/s and carried most of the traffic. The fallback is not
    optional on this network.
  - Next boot: aginx run / carrier run / aginxbrowser run, `done ok`.
- **aginxbrowser v0.2.5 upstream musl asset is broken**: downloads fine,
  sha-verifies, but segfaults instantly on device (empty log, no server,
  even `--version` segfaults). Local repo has the fix (b0315d7 "fix(ci):
  install zig from the official tarball in build-musl") AFTER the v0.2.5
  tag — the released asset was built by the broken CI zig. M9's verified
  on-device build was their main@8a52027 locally built (sha 7fe02343…),
  a different binary. Rebuilding locally per the M9 recipe (cargo zigbuild
  0.16.0, RUSTY_V8_MIRROR local mirror; mirror files via gh-proxy);
  result to be appended.
- aginxbrowser rebuild result: local build of their main@b0315d7 (cargo
  zigbuild, zig 0.16.0, RUSTY_V8_MIRROR pointing at a local mirror of the
  four v150.4.0 files — BOTH the aarch64-apple-darwin and musl .a.gz are
  needed, the host-side build-deps link v8 too) → 61 MB static, runs on
  device: /health 200 (engine diting), POST /fetch example.com returns
  title + markdown. Published as mirror release
  yinnho/aginxos aginxbrowser-v0.2.5 (same treatment as codex/grok);
  manifest switched to it, `agpkg sync` reports all five up to date.

## M10 UI half: Wi-Fi setup wizard on device (2026-08-30)

`crates/wifi-wizard` — TUI inside aterm's pty (nlscan -> numbered AP list,
strongest first, same-SSID dedup, signal bars; non-ASCII SSIDs listed but
marked unjoinable since the v1 keyboard is ASCII-only). Password prompt ->
writes /etc/wifi.conf (0600) -> runs net-bringup in the background and
mirrors its boot.state verdict lines (net-bringup redirects its own stdout
to /var/net.log, so the wizard polls the state file instead) -> offers a
reboot so services start. EOF on stdin quits the wizard (a rescan loop on
EOF would spin forever — caught in the first pty test).

- aterm wiring: launcher gained a 5th button (WIFI SETUP; button geometry
  re-derived for 5) and, at startup, if /etc/wifi.conf is missing the
  wizard replaces the launcher automatically (SYSTEM.md §9.2's first-boot
  path; headless adb-fed wifi.conf still works unchanged).
- Parsing gotchas fixed live: nlscan separates columns with runs of
  spaces (split(' ') yields empty fields) and the signal is TWO columns
  (-53.00 dBm) — the parser peels four whitespace tokens and treats the
  remainder as the SSID.
- Verified on device: scan list renders correct (6 BSS -> 3 deduped), and
  the full flow was then driven from the panel with the touch keyboard —
  wizard auto-started after wifi.conf was removed, network picked,
  password entered, and boot.state ran to `wifi ok Legrand AP / dhcp ok /
  internet ok / done ok` with wlan0 holding its lease. First-boot setup
  now works end-to-end without adb.

Session-end device state: AginxOS boot, slot a, adb aginxosredfin; all
five packages installed from the manifest; aginx + carrier + aginxbrowser
running; /var/home auth restored from the pre-wipe backup; out/rootfs.img
rebuilt with wifi-wizard + updated aterm/manifest/agpkg baked in.

## M15 power management on device (2026-08-31)

aterm gained: qpnp_pon key reading (/dev/input/event1, KEY_POWER=116; the
node also carries volume-down, ignored), screen blank via null-SETCRTC,
60 s idle auto-blank, short-press blank/wake toggle, hold >=1.2 s =
shutdown, launcher RESTART + POWER OFF buttons, and reboot2 extended
(`reboot2 poweroff` = RB_POWER_OFF; no arg = plain restart; other args
stay RESTART2 reasons).

- **Blank path probed, not assumed**: this kernel's sde connector has NO
  legacy DPMS property (OBJ_GETPROPERTIES on connector 29 lists 17 props —
  EDID/link-status/caps/roi/hdr/autorefresh/bl_scale/topology/LP — no
  DPMS; sysfs card0-DSI-1/dpms accepts writes but nothing happens; no
  /dev/fb*, msm_drm has no fbdev emulation). The working route is a null
  SETCRTC (fb_id=0, count_connectors=0, mode_valid=0): dmesg then shows
  `sec_ts suspend` + `dsi_backlight_early_dpms power_mode:5` and
  panel0-backlight/bl_power goes 4 (POWERDOWN). Wake = re-SETCRTC relatch
  (the same path as aterm's PAGE_FLIP-refused fallback). While blanked
  aterm must not present() — a relatch would re-enable the CRTC — so the
  render loop is gated on !blanked.
- **Injected-key verification** (toybox sendevent into event1, so every
  step below was observed without touching the device): 60 s idle ->
  blank (bl_power 0->4, screen held off, aterm alive); short press while
  blanked -> wake (dsi_display_set_mode relatch, bl_power 4->0); short
  press while lit -> blank (4); short press -> wake (0). KeyReader's
  event1 parsing proven by the toggle itself.
- **Shutdown verified**: held KEY_POWER (no release) -> aterm drew the
  farewell frame, spawned `reboot2 poweroff`, and the machine cut power
  (adb offline, device off).
- Battery current_now (qpnp-qgauge) is FROZEN at 7324 — useless as an
  observable for power experiments on this kernel.
- Not yet physically re-checked by hand: real power-key feel (same node
  as the injected events, low risk), touch wake while blanked (sec_ts
  suspends with the panel — may deliver nothing until the power key
  wakes it), launcher RESTART/POWER OFF buttons, charge/power-on
  behavior while off.

Session-end device state: POWERED OFF via the new M15 shutdown path.
Power back on with the physical power button (or plug USB). Slot a,
rootfs has new aterm + reboot2 via adb push (persisted, ext4); the
baked-in rootfs image is NOT yet rebuilt — next build-rootfs.sh run
should include them.

## M16 service layer + slot-successful marker on device (2026-08-31)

Supervision (`::respawn:/usr/bin/agsvc` in inittab, busybox init stays
PID 1) observed live after a full reboot:

- Units from /etc/agsvc.d (aginx, aginx-carrier, aginxbrowser; type
  simple, carrier gated by requires_weak aginx): all three `ready` via
  `agctl list`, children re-parented under agsvc (PPID = agsvc).
- `kill -9` a service -> respawned with growing backoff (spawns counter
  ticks). 5 kills inside 60 s -> unit parked `failed`, no more spawning;
  `agctl start` clears the breaker and returns it to ready.
- Readiness contracts verified with throwaway units: type notify (`sh -c
  'echo -n r >&3; sleep 60'`) goes Starting->ready on the fd-3 byte;
  exiting before notifying (`exit 3`) counts died-before-ready, not
  ready. `agctl stop/restart/reload` all behaved; reload picked up new
  and removed unit files live.
- `kill -9` agsvc itself -> children die with it (PDEATHSIG), init
  respawns agsvc within seconds, whole stack re-spawns from unit files.
  Verified twice (manual + post-reboot instance).
- App registry: rcS runs /etc/init.d/app-registry; /var/apps seeded with
  codex + grok (aclone correctly pruned — no /var/bin/aclone yet);
  launcher draws them from the scan (new aterm binary on device).

Slot-successful marker — the fastboot-loop fix. The original assumption
(libboot_control bootloader_control at misc+2048) was **wrong on this
device** and the probe chain that disproved it:

- misc (1 MiB, /dev/sda3): no TCAB magic anywhere. +2048 = "theme-dark"
  (recovery's vendor-space theme string), +0x8000 = misc_virtual_ab_message
  (v2, magic 0x56740AB0). devinfo/ssd/uefivarstore/logfs/spunvm/secdata/
  limits/storsec/toolsfv/klog/splash all scanned: no TCAB.
- One plain reboot re-dumped: ONLY klog changed (UEFI log tail append).
  So the boot-time slot write is not in any partition *payload*.
- bootctrl.lito.so (vendor_boot ramdisk, /system/lib64/hw) imports
  gpt_disk_*/gpt_utils_* — the slot store is the **GPT entry attribute
  u64** of every *_a/_b partition, per-LUN (/dev/sda sdb sdc sde sdf).
  Observed bits: 48-51 priority, 52-55 tries-remaining, 56 successful.
  **These UFS LUNs are 4K-logical-block** — GPT header at byte 0x1000,
  entries at 0x2000 on /dev/sda; 512B-offset reads see only MBR padding
  (that burned an hour; agboot-ok reads queue/logical_block_size now).
- State before marking: boot_a pri=15 tries=0 succ=0 — our unmarked
  boots since the last `fastboot set_active a` had drained the counter
  to the edge. boot_b still pri=2 tries=8: the next slot-a failure
  would fall through to stock Android on our ext4 userdata (first_stage
  would format it). Marked before that could happen.
- `agboot-ok` (new, GPT-based) set succ=1 tries=7 on all 29 *_a entries
  across 5 LUNs (primary+backup GPT, CRCs rewritten, fsync). Reboot:
  slot a booted and `agboot-ok status` shows tries STILL 7 succ STILL 1
  — ABL does not drain a successful slot. rcS re-marks after every
  `done ok` boot as belt-and-braces.

Session-end device state: RUNNING AginxOS, slot a marked successful,
agsvc stack supervised (aginx/carrier/browser ready), registry seeded
(codex+grok), new aterm/agboot-ok/rcS/inittab/provision pushed by hand
(persisted in the ext4 rootfs; baked image NOT rebuilt — fold into the
next build-rootfs.sh run together with M15).

## M17 keyboard: event split + on-device input-path fixes (2026-08-31)

Input path split (SYSTEM.md §12.6): keyboard is now a key table
(kb.rs `KeyDef`/`Act`, EXTRA_KEYS 7 + SPECIALS 5 as const tables; letter
pages stay char grids). Hit tests return `InputEvent` — `Key(KeyEvent)`
for Esc/Tab/Enter/Backspace/arrows/Ctrl+letter, `Text(String)` for
composed text — and byte encoding happens once at the terminal layer
(`input::encode`), which reads the child's DECCKM state: `Term` tracks
`app_cursor` from CSI ?1 h/l (vte 0.13 collects the '?' into
`intermediates`, so private modes gate on `intermediates == [b'?']`);
arrows encode SS3 (ESC O A) when set, CSI (ESC [ A) otherwise; RIS
resets it. Hold-repeat (DEL + arrows, 400 ms) and the M18 voice
injection point (ATERM_INJECT=1 → aterm polls /run/aterm.inject each
loop and injects the file's content verbatim as TextInputEvent) both go
through the same `inject()` in main.rs.

Verified on device (adb-pushed binary, ATERM_START=/bin/sh test loop;
synthetic taps via sendevent on /dev/input/event2, text via
/run/aterm.inject):

- Letter tap compose: tap 'h' + inject "ello\r" → sh ran
  `touch /var/k2-hello` (file exists). Keys fire on finger-DOWN; inject
  path end-to-end incl. `\r` → command execution.
- Arrow + DEL line edit: inject "touch /var/arw-XX", tap LEFT
  (ESC [ D), tap DEL (0x7f), inject "\r" → `/var/arw-X` exists,
  `/var/arw-XX` never created.
- CTL+c: one-shot CTL latch + 'c' composes KeyEvent::Ctrl('c') → 0x03;
  a foreground `/bin/busybox sleep 30` (exec'd, own pgrp, tpgid set)
  died and the next injected command executed.
- ESC: tap ESC → 0x1b written to pty; a canonical-mode `dd bs=1`
  read it back (`od` shows 1b). ESC alone doesn't complete a canonical
  read — raw-mode consumers (codex/readline) get it immediately.

Two bugs found on device, both fixed:

1. **TouchReader stale start_y** — synthetic frames (tracking-id before
   x/y, e.g. sendevent order) anchored `start_y` from the *previous*
   touch's `raw_y`, so any new touch >30 px away instantly read as a
   drag: Tap became Up, keyboard summon silently failed. Real firmware
   reports position before tracking-id, which masked it. Fix: track
   `y_in_frame` (y seen this frame) — anchor from raw_y at tracking-id
   when the position already arrived, else mark `fresh` and anchor at
   the first y of the touch. Both orders now produce clean
   down=false-drag taps (observed in the lift trace).
2. **SIGINT discarded: SIG_IGN inheritance** — CTL+c bytes reached the
   ldisc (kill_pgrp fired) but every pty child ignored SIGINT. Root
   cause via /proc/*/status SigIgn masks: rcS's busybox sh ignores
   SIGHUP+SIGINT (0x1006), adbd ignores both too (0x...06 + rt-sig 38),
   and SIG_IGN survives exec — so the whole aterm→sh→jobs chain was
   immune to ^C (init itself only ignores SIGPIPE, so this is ancestry,
   not kernel). Even `kill -INT` from adb couldn't kill an
   adb-spawned sleep. Fix: the pty child now resets HUP/INT/QUIT/TERM/
   TSTP/TTIN/TTOU/PIPE to SIG_DFL and clears the signal mask before
   execv. After the fix the sh child's SigIgn lacks INT, and CTL+c
   kills a foreground job (test above).

Testing anomalies (not bugs, cost real time):

- busybox sh runs `sleep 30` as a NOFORK applet *inside* the shell
  process — invisible to `ps` by name. Interrupt tests must exec a
  real binary (`/bin/busybox sleep 30`) to be observable.
- First inject after an aterm restart is sometimes lost (master write
  racing child setup) — warm the instance with a throwaway `#\r`.
- Keyboard starts hidden on fresh instances; a tap above the panel
  toggles visibility. Test sequences must establish kb state first
  (our CTL "failures" twice were just the CTL tap toggling kb on).

Session-end device state: RUNNING AginxOS, aterm restored to the
standard aterm-handoff respawn loop (launcher mode, /var/aterm.log),
test artifacts removed. Same rootfs caveat as M15/M16: the new aterm is
adb-pushed (persisted on ext4) but the baked rootfs image is not yet
rebuilt — fold M15+M16+M17 into the next build-rootfs.sh run.

## rootfs re-bake + full boot chain on device (2026-08-31)

Task #72: fold M15+M16+M17 into a baked rootfs image and prove the whole
boot chain with no adb-pushed crutches. `scripts/build-rootfs.sh` (with
build-phone.sh `all` now including wifi-wizard) produced out/rootfs.img
(124 MB); flashed to userdata after a full state backup
(.local/backup-pre-bake-0831: wifi.conf, /var/home/.aginx/.codex/.grok —
restored post-flash).

**Full chain observed after one reboot** (boot.state):

    touch ok / battery ok 100% / modem ok / wlan ok wlan0
    wifi ok 666 / dhcp ok 172.20.10.3 / internet ok www.baidu.com 696199B
    time ok 2026-08-31 / done ok

aterm, agsvc, adbd all up from the baked image. Provision then
agpkg-synced the must-exist tier over the air: aginx-carrier (28.7 MB),
aginxbrowser (64 MB), codex (233 MB) — downloaded, sha256-verified,
installed to /var/bin with no host involvement. carrier came up ready
with the iLink WeChat channel online.

The AP was an iPhone hotspot ("666", psk device-only), which cost three
wifi-join fixes — all device-observed:

1. **iOS masks the beacon SSID** (length kept, bytes zeroed: `00 00 00`);
   wildcard scans never unmask it. nlscan now hex-escapes non-printable
   SSID bytes (the old `?` substitution destroyed the evidence); wifi-join
   sends a directed probe (SSID attr in TRIGGER_SCAN) and accepts an
   all-zero SSID of matching length as a hidden-AP match.
2. **WPA3-transition 5 GHz BSS associates but never handshakes**: with
   SAE (00:0f:ac:08) in the AKM list, Apple accepts assoc and then never
   sends EAPOL M1 (rx counters stayed 0 for 20 s; kmsg showed auth/assoc
   complete, SME akm 9 = eCSR_AUTH_TYPE_RSN_PSK, correct). Same SSID in
   "Maximize Compatibility" mode (2.4 GHz, PSK-only) handshakes fine.
   wifi-join now prefers a PSK-only BSS over any SAE-advertising one
   when both carry the SSID. SAE/PMF support stays future work.
3. **M2 rejected on plain WPA2 when our RSNE carried MFPC**: setting
   PMF-capable bits (an experiment against #2) made Apple MIC-accept
   nothing — M2 sent, no M3, no M1 retransmission. Plain WPA2 APs want
   rsn caps 0x0000 exactly. Reverted; M1→M4 completed and
   "passphrase correct".

**agsvc absent-recheck bug (M16, first real exercise)**: with /var/bin
empty at agsvc start, all three units went Absent and stayed there for
25 min after the binaries landed. `try_spawn()` only acts on Backoff,
but the tick loop's Absent recheck called it with the unit still in
Absent — a no-op forever. Fixed with an Absent→Backoff pre-pass in
tick(); after the binary swap aginx-carrier and aginxbrowser were ready
2 s later (observed in kmsg + agctl). M16 acceptance never caught this
because binaries existed before agsvc started.

Remaining (network-bound, not system bugs): aginx (gateway) and grok
never landed — first a TLS reset at 02:29, later the hotspot's DNS
(172.20.10.1) and then the whole data path died (iPhone hotspot
auto-off). Provision correctly recorded `pkg fail`; next networked boot
re-syncs, and the now-working 30 s recheck starts the gateway unit
without a reboot.

Device session end state: RUNNING AginxOS from the baked rootfs,
carrier + browser ready, aginx absent pending network. wifi.conf holds
the iPhone hotspot (ssid 666, psk device-only). Note for the next
build: the wifi-join/nlscan/agsvc fixes are in the source tree; the
flashed image predates them (current /bin binaries are adb-pushed
replacements on the rw ext4).

## M18 audio bring-up: 说 proven baked, 听 blocked at the sensor-DSP boundary (2026-08-31)

Task #69. Card: sm7250-noextcodec-snd-card (card 0, 53 pcm + 8 compr
devices). All work via freestanding ioctl tools (snd-cap/snd-play/
snd-mixer + new i2c-reg), no alsa-lib, musl-static.

**uapi ioctl numbers were the first "silent mic"** — snd-pcm-uapi.h
gave PREPARE HWSYNC's slot (0x22→kernel treats it as HWSYNC on a SETUP
stream → -EBADFD), READI/WRITEI compat-range nrs and a wrong READI
direction. Numbers now match uapi sound/asound.h exactly, frozen with
_Static_asserts on both struct sizes and ioctl encodings. Two more
stream-keeping fixes the hard way: read the period size the kernel
wrote back into hw_params (a fixed 1024 against the q6 FE's 2000-frame
period overruns in seconds) and stop_threshold = 1<<62 (rate*4
self-stops capture on overrun). Hostless FEs need geometry taken from
HW_REFINE's own bounds (they cap period ≤1024 / buffer ≤4096).

**Module order proven at symbol level, baked in audio-bringup**: q6_dlkm
EXPORTS msm_aud_evt_* and digital_cdc_rsc_mgr_hw_vote_* → swr_ctrl_dlkm
loads after q6 (it was failing in every baked boot, dragging the four
macro modules with it). Slimbus chain slimbus → of_slimbus (imports
slim_register_board_info) → msm_sps → slim_msm_ngd (imports
of_register_slim_devices) → bluetooth_power. Boots 7/8: `load done,
0 failed`.

**Boot 6 "audio fail no-card" was not audio**: the card and all 53 pcm
devices sat registered in /sys/class/sound while /dev/snd held one
stale `timer` node. This kernel has NO devtmpfs (`mount -t devtmpfs` →
ENODEV — rcS's mount line is dead code, same family as the missing
debugfs). Android gets nodes from ueventd; we get them from `mdev -s`
after the modules load — same trick touch-bringup already used. Baked
into the card-check loop; boots 7/8 report `audio ok`.

**Two aDSP behaviors that fake a dead mic path** (both proven twice,
boots 6 manual + 7 script-only):

1. Backend `Channels` ctls reset to 0 on a fresh card. Sessions open
   healthy, READI returns data — all zeros. With
   `PRI_TDM_TX_0/QUIN_TDM_RX_0 Channels` = 2 the same play+capture
   carries the loop (880 Hz FFT peak 6.4e6 vs 0.0 rms).
2. First session cycle after the mixer writes is a cold cycle: run 1
   all zeros, identical run 2 carries the loop. The aDSP wires the TX
   loopback graph one session late. audio-bringup now burns it with a
   vol-0 1 s play + 1 s capture; boot 8's FIRST real session after
   boot carried the tone (rms 174.7, 880 Hz peak 6.38e6).

**说 (playback)**: 3 s 48 kHz stereo 880 Hz tone through MM1 →
QUIN_TDM_RX_0 (AMP PCM Gain 17 / Digital PCM Volume 817, Main AMP
Enable) plays rc=0 with no XRUN on the fully baked chain (boot 8).
Routing verified in-band: the tone appears in a simultaneous capture.
Acoustic audibility human-confirmed (user ear, 2026-08-31): the 880 Hz
beep at vol 60 is clearly audible from the speaker. A 15 s Chinese
TTS speech sample (48 kHz stereo, vol 80) then played rc=0 and was
heard and understood (user ear, same day) — 说 is acoustically closed
end to end: baked boot → MM1 → QUIN_TDM_RX_0 → dual CS35L41 →
audible, intelligible speech.

**The captured "mic audio" is a digital aDSP echo, not a microphone**:
FFT of captures during playback shows 96-99 % of energy at the played
880 Hz ± sidebands plus 1760/2640 harmonics, zero broadband floor;
captured amplitude scales linearly with playback volume (~7 %, peak
685 @ vol 60, 59 @ vol 5); after playback stops the capture is
bit-silence. The loop mixer ctl 'PRI_TDM_TX_0 Audio Mixer MultiMedia1'
is also REQUIRED for session survival without vendor ACDB: with it off,
READI fails EIO in ~10 s and kmsg shows `event_handler: reclaimed all
bufs` (aDSP async teardown of the codec-TX COPP). Route mixer ctls read
back "1,0" after writing "1 1" — q6's put only consumes value[0].

**听 (capture) root cause sits past the codec, at the sensor-DSP/SLPI
boundary**. The rt5514 codec is alive and configured — new i2c-reg tool
(bus 0 addr 0x57, regs accessed as `reg | 0x18000000`, 32-bit BE):
VENDOR_ID2 0x10ec5514, DOWNFILTER AD_AD_MUTE bits clear, DIG_SOURCE
AD0+AD1 DMIC select, CLK_CTRL1 AD0/AD1 enables + DMIC clock, mic LDO
DAPM event fires per capture — but its driver forces
DSP_FUNC=WOV_I2S_SENSOR (rt5514 hw_params), i.e. the I2S output carries
the sensor DSP's stream, not raw ADC. 'DSP Booted' (rt5514-qmi) was 0;
boot 6 brought the QMI handshake ALIVE: `remote rtk_spi server online,
connecting` + `Send request success` — but the SPI-side DSP firmware
never streams. Alternatives without the loop all fail on-device:
MM1/MM2 no-loop EIO, hostless pcm53c ADSP_EFAILED, LSM pcm56c/57c need
the LSM protocol (EIO), ADC pcm58c is 8 kHz mono only. Next lever:
boot the SLPI-side sensor stack (rtk_spi firmware path) or obtain ACDB
calibration.

Device session end state: RUNNING AginxOS with the baked audio chain
(boots 7/8 `audio ok`, warm-up in place); patched vendor_boot image
still flashed (M18 experiment state). wifi join rc=2 both boots —
hotspot "666" absent (iPhone auto-off), expected. i2c-reg/snd-mixer
added to build-rootfs.sh for the next re-bake (#72).

## Rootfs re-bake #2: M18 audio chain + wifi-join/agsvc fixes in the image (2026-08-31)

Task #72 follow-up bake. `build-phone.sh musl` + `build-rootfs.sh` (now
also baking snd-mixer/i2c-reg and chmodding audio-bringup) →
out/rootfs.img, 129 MB content / 2 GB fs, flashed to userdata after the
state backup (.local/backup-pre-bake-0831b: wifi.conf + /var/home/
.aginx/.codex/.grok, 992 MB).

**Boot from the image alone, zero adb-pushed crutches** (boot.state):

    touch ok / battery ok 100% / modem ok
    audio ok  0 [sm7250noextcode]: sm7250-noextcodec-snd-card
    wlan ok wlan0 / wifi fail no /etc/wifi.conf   ← expected, by design

Audio holds on the fresh image: the FIRST play+capture after boot
carries the loop (880 Hz FFT peak 5.8e6, rms 175) — mdev coldplug,
backend Channels ctls and the silent aDSP warm-up all correct in the
baked script.

wifi.conf restored from backup → net-bringup joined the "666" hotspot
(172.20.10.3), ntpd set the clock, boot.state flipped to `done ok`.
Provision then re-synced the wiped /var/bin over the air: direct
GitHub TLS was reset (standing network condition), the gh-proxy
fallback delivered — aginx, aginx-carrier, aginxbrowser (64 MB),
codex (233 MB) all sha256-verified; `codex --version` runs (0.151.0).

**agsvc absent-recheck fix verified from the baked image**: aginx-
carrier and aginxbrowser were running as children of agsvc (~4 min
after their binaries landed, no reboot, no manual spawn). One nuance
observed: the `aginx` unit had FAILED during the pre-install window
("No agents configured" + carrier path missing) and the recheck does
not resurrect a failed unit — only Absent→spawn; after carrier
existed, `agctl restart aginx` brought it to ready. A unit that fails
while its dependency is still downloading stays failed until nudged.

grok (largest asset) outlived this entry twice: the reboot killed its
first download, and the retry's direct GitHub GET STALLED (no error,
0 bytes for minutes — agdl has no read timeout, so the gh-proxy
fallback never triggers). Killing the stalled agdl unblocked the
fallback, which pulled ~100 MB/min via gh-proxy; grok 1.0.12 installed
and provision reached `pkg ok`. agdl needs a connect+read timeout to
make the mirror fallback self-healing — added the same day
(timeout_connect 30 s / timeout_recv_body 60 s) and verified on
device: a held-open response that sends headers but no body now
fails at exactly 60 s with "timeout: receive body"; a slow-but-
flowing trickle (8.2 MB in 200 s direct) is correctly NOT killed. End state re-checked: carrier +
browser + aginx all ready, five /var/bin binaries live.

Device session end state: RUNNING the fully baked image, carrier +
browser + aginx ready, audio chain live, patched vendor_boot still
flashed (standing M18+ experiment state).

## M18 听 closed: 'DSP ADC' ctl boots the rt5514 DSP; mic live end to end (2026-08-31)

Continuation of #69 on the baked image. The blocker diagnosis from the
earlier session was rebuilt from source (redbull lineage-22.1) and two
wrong turns corrected:

1. **'DSP Booted' is not an rt5514 control at all.** The string exists
   only in `/vendor_a/etc/mixer_paths_noextcodec_snd.xml` (and the crus
   copy), inside the cs35l41 path
   `cs35l41-load-protection-firmware-start` — it is the speaker amp's
   firmware-load marker. No module binary contains it. The prior
   "'DSP Booted' = 0 ⇒ mic DSP never booted" inference is void.
2. rt5514-spi.c is only the AP-side **buffer reader** (copy_work_0..3;
   ADC ring = RT5514_BUFFER_ADC_BASE/LIMIT/WP, validated against the
   0x4fe00000 DRAM window). The DSP boot lives in rt5514.c and is
   triggered by an ALSA ctl: **`DSP ADC`=1** → rt5514_dsp_enable →
   i2c patch + `request_firmware(rt5514p_dsp_fw1..4.bin)` (files in
   /vendor/firmware; v_p part, fw_addrs 0x4fe00000/0x4ff00000/
   0x4fe98000/0x4fea8000) pushed over SPI. Observed on write:
   `DSP Firmware Version: 2.1.20.0`.
3. Before that ctl write, pcm58c ("ADC Capture" FE = rt5514-dsp-fe-dai3,
   8 kHz mono) hw_params'd fine but READI EIO'd, and dmesg showed the
   exact failure: `adc is streaming` → `rt5514_schedule_copy: Fail for
   address read` (SPI buffer-descriptor reads outside 0x4fe00000). After
   the write the FE opens and streams rc=0 — but its buffer stays
   all-zeros even with the mic acoustically live (beep + speech tests).
   The DSP ring buffer is NOT a usable mic source in this config; MM1 is.
4. **DMIC mux enum is `{"DMIC1","DMIC2"}` — index 0 is the live handset
   mic.** The earlier recipe (and mixer_paths XML string "DMIC1" ≠ 1)
   had `Stereo1 DMIC Mux=1` = DMIC2 = dead. 3-window matrix with the
   phone's own speaker as source: DSP-off+DMIC1 0.0 rms, DSP-off+DMIC2
   0.0 rms, **DSP-on+DMIC1 rms 154 / 880 Hz mag 164.6** — the rt5514
   DSP supplies the DMIC clock; codec-side routing alone powers nothing.
5. **Acoustic (non-loop) proof**: with the phone completely silent, a
   1 kHz tone from the Mac speaker across the room lands at rms 11.4 /
   1 kHz mag 17.4 in MM1 — air-conducted, no loopback involvement. A
   Mac TTS speech sample then produced a proper speech envelope
   (rms 285.7, peak ±2934, 0.5 s windows 12→516→160→546→485…); the
   recording was played back on the Mac for the user. The earlier
   "user speaks" windows stayed silent — speech cues never coincided
   with the capture window (one run also had the DSP left off after a
   control window); the Mac-source tests made the verification
   self-contained.

**听 recipe now baked in audio-bringup** (replacing the wrong mux):
`PRI_TDM_TX_0 Channels=1` (vendor handset-mic), `DSP ADC=1`, sleep,
then `Stereo1 DMIC Mux=0` + `Sto1 ADC MIXL DMIC Switch=1` +
`ADC1 Capture Volume=60 60` (vendor 1st-mic-only level) — order matters,
the fw load rewrites codec regs so the mic route goes after it. Warm-up
MM1 capture now mono to match the proven session shape.

**Cold-boot verification, zero manual steps** (script pushed to
/etc/init.d on device, image not yet re-baked): fresh boot shows
`DSP Firmware Version: 2.1.20.0` at t+53 s from the script's own ctl
write; boot.state full chain ok (wifi auto-joined Legrand AP → dhcp →
internet → pkg → done ok); the FIRST capture session on that boot
carried speech with no warm-up crutch beyond the baked one
(rms 130.0, peak ±1088, speech bursts in every 0.5 s window). 听 is
acoustically closed end to end: DMIC1 → rt5514(DSP) → PRI_TDM_TX_0 →
q6 MM1 → /dev/snd/pcmC0D0c, 48 kHz mono.

Device session end state: RUNNING baked image + the updated
audio-bringup pushed over adb (device copy newer than the image — fold
into the next re-bake), patched vendor_boot still flashed (standing
experiment state).

## M19 camera: full driver stack loaded, nodes up (2026-08-31)

Task #71 start. The whole Qualcomm camera driver stack (IFE route, no
ICP) now loads on device — 31 modules, proven by insmod rounds with
per-symbol dmesg evidence:

    camcc-lito, cam_debug_util, cam_tasklet_util, cam_res_mgr,
    cam_smmu_api, cam_mem_mgr, cam_utils, cam_req_mgr, cam_irq_controller,
    cam_isp_packet_parser, cam_sensor_vsync_pb, cam_cpas, cam_cdm,
    cam-sync, cam_cci, cam_csiphy, cam-sensor-io, cam_sensor_util,
    cam_gyro_core, cam_ife_csid, cam_ife_csid17x, cam_ife_csid_lite17x,
    cam_vfe, cam_ife_hw_mgr, cam_isp_hw_mgr, cam_sensor_vsync_dev,
    cam_req_mgr_late, fw-update, cam_sensor, cam-context, cam_isp

Node result: /dev/media0-3, /dev/video0-4, /dev/v4l-subdev1-5 after
mdev (media2/3 + video3/4 + subdevs appear with cam-context/cam_isp).
CSIPHY0-3 probes link regulators (dmesg "Linked as a consumer to
regulator.64/.7"). DT: cci0 → cam-sensor@0/@1 + eeprom@0/@1 +
actuator@0 + ois@0 (rear main+ultrawide), cci1 → cam-sensor@2 +
eeprom@2 (front).

**Two traps that cost the first hour, both recorded for good:**

1. **Mixed dash/underscore filenames.** cam-sync.ko, cam-sensor-io.ko,
   cam-context.ko use dashes; every other cam module uses underscores.
   `insmod cam_sync.ko` fails with "No such file" — which a 2>/dev/null
   eats, and a status-check bug (tr -d instead of tr '-') then read as
   "loaded". Result: cam_sync appeared loaded while never being
   inserted, and cam_isp/cam_context cascaded "Unknown symbol
   cam_sync_*". Diagnosis tool that cracked it: busybox insmod prints
   the per-module "Unknown symbol X (err -2)" lines to dmesg — read
   THOSE, not the insmod exit text.
2. **fw-update.ko is outside the cam_* namespace** and exports
   checkOISFWUpdate/getFWVersion (Google OIS helpers) that cam_sensor
   imports. Without it cam_sensor fails "unknown symbol" forever.

kallsyms gotcha for future symbol work: CONFIG_KALLSYMS_ALL is unset on
this kernel — /proc/kallsyms lists TEXT symbols only, so diffing a
module's UND list against it produces phantom "missing" data symbols
(v4l2_subdev_fops, __tracepoint_*).

Proven recipe baked as boot/rootfs/etc/init.d/camera-bringup (not yet
in rcS — standalone until first light; idempotent, re-run green).
eeprom/actuator/ois/flash/ICP/JPEG/LRME/FD/custom modules stay out of
the minimal set until a frame lands. Next: identify the media topology
(subdev1-5 roles), then the cam-shot userspace (cam_req_mgr packets →
IFE RDI → raw frame).

Device session end state: RUNNING baked image + camera-bringup pushed
over adb (newer than the image), 31-module camera stack live, patched
vendor_boot still flashed (standing experiment state).

## M19 camera: sensor subdevs up — three ordering traps found and fixed (2026-08-31, second session)

Wrote boot/rootfs/src/media-topo.c (zig-static, /bin/media-topo): media-
controller ENUM_ENTITIES/ENUM_LINKS dump. media2 = cam_req_mgr (video3 +
subdevs + 2x cam-cci-driver), media3 = cam_sync (video4). media0/1 =
vim2m/vicodec test drivers, irrelevant. The Qualcomm entity types decode
via uapi cam_req_mgr.h CAM_*_DEVICE_TYPE (cpas=+7, csiphy=+8,
actuator=+9, cci=+10, eeprom=+12, ois=+13 over MEDIA_ENT_F_OLD_BASE).

After the first 31-module load, sensors were MISSING: all three
cam-sensor@N platform devices unbound, zero sensor subdevs. Three
independent causes, each observed:

1. **cam_req_mgr_late seals the subdev registry.** Its module init calls
   cam_dev_mgr_create_subdev_nodes() → v4l2_device_register_subdev_nodes,
   after which cam_register_subdev() rejects everything:
   `CAM_ERR: CAM-CRM: cam_register_subdev: 675 dynamic node is not
   allowed, name: cam-isp` (observed; cam_sensor's probes hit the same
   wall). cam_req_mgr_late must be the LAST camera module, after
   sensor/isp/context/eeprom/actuator/ois have all probed. With it last,
   cam-isp registers (v4l-subdev14, type CAM_IFE_DEVICE_TYPE).
2. **fw_devlink defers camera probes silently.** Every camera DT node
   (sensor@0/1/2, eeprom@0/1/2, actuator@0, ois@0) is a device-link
   consumer of i2c 1-0075 = slg51000, the camera PMIC (dmesg "Linked as
   a consumer to 1-0075"). Without its driver, every camera probe
   returns -EPROBE_DEFER — which prints NOTHING (no CAM_ERR, no kernel
   probe-failed line, driver dir simply empty) and this kernel has no
   debugfs (CONFIG_DEBUG_FS off) to list deferred-probes. Symptom is
   "driver loaded, devices match, nothing binds, zero log lines".
   slg51000-regulator.ko is the fix and belongs in the chain.
3. **GPIO race on PM8150L gpio9 (global 1109).** Loading
   slg51000-regulator late (after the rest of the stack) failed its probe
   with `slg51000-regulator 1-0075: GPIO(1109) request failed(-16)` —
   dlg,enable-gpios = <pm8150l_gpio 10>, <pm8150l_gpio 9> and gpio9 was
   already gpio_requested by someone (DT-wide sweep found no other
   reference: pm8150l pinctrl states only claim gpio3 (irq_pin_top),
   gpio5 (key_vol_up), gpio8 (camera_rear_vcm_en — that one is the
   bound /soc/gpio-regulator@0), gpio10 (en_rwcam + reset_pin_bottom,
   shared), gpio11 (reset_pin_top), gpio12 (eldo13_pin); camera nodes
   reference pm8150l gpio2). Loading slg51000-regulator FIRST — before
   every other camera module — wins the race; at boot 14:37 it probed
   clean ("No IRQ configured" only, informational) and 1-0075 bound.

Also this boot: rcS now runs /etc/init.d/camera-bringup (explicit call
added — rcS invokes bring-ups by name; a script sitting in /etc/init.d
that rcS doesn't name never runs. That cost one reboot to notice: pushed
script + reboot produced a boot with zero camera nodes and a camera.log
whose only entry was the previous session's manual run, pre-NTP clock
stamp 14:18 vs real boot 14:30). eeprom/actuator/ois modules added to
the chain (their devices were also frozen on 1-0075).

Observed result, boot 2026-08-31 14:37 UTC: camera-bringup via rcS,
0 failed loads, **14 v4l subdevs**: cam-cpas, 4x cam-csiphy-driver,
3x cam-eeprom, cam-actuator-driver, cam-ois, **3x cam-sensor-driver**
(media2 entities 16/17/18, type CAM_SENSOR_DEVICE_TYPE), cam-isp; all
three cam-sensor@N platform devices bound to driver "qcom,camera";
1-0075 bound to slg51000-regulator. media2 entity list complete.

Device identity still unknown at this stage (sensor power-up/chip-ID
read is a userspace packet op — cam-shot's first job). Next: cam-shot
v0 = CREATE_SESSION on video3 + CAM_SENSOR_PROBE_CMD on each sensor
subdev to read chip IDs over CCI; then the IFE RDI capture path.

## M19 camera: all three sensors identified — chip-ID read from userspace (2026-08-31, third session)

cam-shot v0 (boot/rootfs/src/cam-shot.c, musl-static, /bin/cam-shot on
device) walks /dev/mediaN entities for type 0x10001 subdevs whose
major:minor exactly matches a /dev/v4l-subdevN sysfs dev node, opens
/dev/video3 (cam_req_mgr), and issues CAM_SENSOR_PROBE_CMD
(0x10A) packets built entirely in userspace: 3 cam_mem_mgr allocations
(ALLOC_BUF 0x112 with flags KMD_ACCESS|CMD_BUF_TYPE — size field must
be sizeof(cam_mem_mgr_alloc_cmd)=104, NOT 24; video3 validates
k_ioctl->size against the payload struct), mmap the returned fds, and
lay out a cam_packet with num_cmd_buf=2: desc0 = i2c_info{slave,
freq=FAST, cmd=4} + probe{WORD/WORD, reg 0x0016, expected 0xFFFF,
mask 0, camera_id=slot}; desc1 = power blob.

Discovery trick: expected 0xFFFF + mask 0 makes cam_sensor_id_by_mask
return the full 16-bit id and the compare always fail, so kmsg prints
`CAM_WARN ... cam_sensor_match_id: 767 read id: 0xNNN expected id
0xffff:` with the REAL chip id, then powers down cleanly — sensor
stays unprobed and the attempt is repeatable. NACK (wrong address)
prints rc=-22 / read id 0x0 instead.

Observed result: all three sensors answered at 8-bit slave 0x34
(7-bit 0x1A), id reg 0x0016 WORD:
- slot 0 rear-main /dev/v4l-subdev11: **IMX363 (0x363)** — but ONLY
  after the power-up sequence grew SENSOR_CUSTOM_REG1+2 (DT rails
  cam_v_custom1 1.8V, cam_v_custom2 1.1V — the latter is the sensor's
  DVDD from the slg51000 camera PMIC; without them the module never
  powers and every address NACKs).
- slot 1 rear-uw /dev/v4l-subdev12: **IMX481 (0x481)** — VIO/VANA/VDIG
  sequence suffices.
- slot 2 front /dev/v4l-subdev13: **IMX355 (0x355)** — VIO +
  CUSTOM_GPIO1 (PM8150L gpio2) + RESET + MCLK.

Full CCI read cycle (power-up → chip-id read → power-down) driven
purely by our userspace packet; no kernel mods. Benign kmsg noise:
slg51000 "No IRQ configured", CCI probe Device Type 0/1, "No clk data
for ife_dsp_clk", repeated cam_req_mgr_close WARNs from fd closes.
Next: real probe with matching expected id (0x363/0x481/0x355, mask 0)
→ CAM_SENSOR_INIT, then IFE RDI capture (session/link/config → frame).

## M19 camera: rear-main slot 0 solved — IMX363 answers at 0x20; real probe success all three slots (2026-09-01)

Continuation of the 2026-08-31 session. Slot 0 (rear-main) would read
its chip id exactly once and then NACK forever, eventually wedging CCI
(-110, FIFO buf_lvl 0x0). Chain of investigation, all observed:

1. **Rear module rail map (DT phandles resolved on device).**
   sensor@0 and eeprom@0 share one rail list; actuator@0 only cam_vaf:
   - cam_vio → slg51000 ldo7 (1.8V)   [sysfs regulator.80]
   - cam_vana → slg51000 ldo3         [regulator.76]
   - cam_vdig → slg51000 ldo1         [regulator.74]
   - cam_v_custom1 → slg51000 ldo4    [regulator.77]
   - cam_v_custom2 → slg51000 ldo6    [regulator.79]
   - cam_vaf → /soc/gpio-regulator@0 "camera_ldo", fixed 2.85V,
     pm8150l gpio8 (camera_rear_vcm_en) [regulator.9]
   - cam_clk → camss GDSC
   slg51000 sits at i2c@98c000 slave 0x75.

2. **Kernel executor has no rail fallback.** cam_sensor_core_power_up
   (cam_sensor_util.c:1898+) enables ONLY rails that appear in the
   power-settings array. The vendor Chromatix power-up array decoded
   from com.qti.sensormodule.metric_imx363_lito2.bin (factory image,
   the only imx363 module bin — /lib64/camera/ has exactly one)
   deliberately contains no VIO and no MCLK step; driving that exact
   sequence from our userspace NACKs. With no VIO step the sensor's
   I2C/DOVDD (ldo7) is simply never powered.

3. **Working slot-0 sequence** (added VIO first, MCLK before XCLR):
   up: VIO(1) VANA(1) VAF(0) VDIG(1) custom1(1) custom2(1) MCLK(1)
   RESET=1(5); down: MCLK(1) RESET=0(1) custom2 custom1 VDIG VAF VANA
   VIO. Power-up succeeds kernel-side either way — match_id still ran
   and NACKed at 0x34, i.e. a NACK does NOT mean power-up failed.

4. **Full-bus sweep (new `cam-shot --sweep 0`)** walks every even
   8-bit address 0x02..0xFE on the slot's CCI master, one full power
   cycle per address, classifying each from kmsg (ACK prints nonzero
   `read id`). Result on cci0/master0 (rear module bus: sensor@0 +
   eeprom@0 + actuator@0 + ois@0): **exactly one address answers —
   0x20, id 0x0363.** No eeprom/actuator/ois ack anywhere (they stay
   silent even with our full rail set; unresolved, noted).
   IMX3xx latches one of two slave addresses from INCK/XCLR power-on
   timing; with MCLK running before XCLR release our timing latches
   0x20. The single historical 0x34 success was the other latch.
   (Cam-shot kmsg classifier must filter to `CAM-` lines: an adbd
   watchdog line contains the word "timeout" and false-triggers.)

5. **Real probe result (observed):** pinning slot 0 → addr 0x20,
   `--real 0` returns rc=0 on try 1; three rounds × three slots =
   **9/9 first-try successes** (`Probe success,slot:N` in kmsg, 14
   total this boot). No CCI wedging since probing the right address;
   the -110 storm was a symptom of hammering 0x34, not a bus fault.

Device state: patched vendor_boot still flashed, camera stack loads
via rcS (camera-bringup), /bin/cam-shot = sweep-capable build.
Remaining for M19: IFE RDI capture — cam_req_mgr CREATE_SESSION,
link csiphy→csid→vfe, CAM_CONFIG_DEV, SCHED_REQ, plus sensor init
register tables (regSetting array in the metric_imx363 blob,
169040 bytes @0x01bcb0, holds the streamon register lists).

## M19 camera: FIRST FRAME — IFE/RDI pipeline proven via CSID TPG, no sensor data needed (2026-09-01)

Result first: `/bin/cam-shot --stream --tpg --wait 3000 --out
/tmp/tpg.raw` on a fresh boot returns rc=0 with a full 2,525,600 B
frame (1640x1232 RAW10, stride 2050) DMA-written by the IFE RDI path,
fence signaled, no IRQ storm. The userspace-driven IFE route is proven
end to end: CREATE_SESSION -> ACQUIRE (sensor+csiphy+isp) -> LINK ->
sensor INIT/CONFIG -> ISP INIT/CONFIG/UPDATE (fence) -> SCHED_REQ ->
START -> SYNC_WAIT signaled. Everything below is what stood between
the old "sync timeout" runs and this frame.

1. **--tpg mode** (new cam-shot flag): in_port->res_type =
   CAM_ISP_IFE_IN_RES_TPG (0x4000) at IFE ACQUIRE_HW. The kernel then
   skips csi2_rx_cfg.phy_sel, arms the CSID's internal test generator
   (cam_ife_csid_config_tpg: VC 0xA / DT 0x2B fixed, width<<16|height
   from in_port, split color bar), and RDI0's CID is (0xA, 0x2B).
   Sensor/csiphy are never streamed. First TPG attempt on the old
   8-hour boot timed out with the familiar storm signature (RX
   0x1100033 = ECC+UNBOUNDED+DL0/DL1 SOT, RDI0 0x1000, evt "idx 2
   err 5 phy 2"). Decode of that signature (cam_ife_csid_core.h):
   RDI0 0x1000 is CSID_PATH_INFO_INPUT_SOF — NOT an error: RDI0 was
   already receiving frame starts. The RX bits are expected noise:
   the RX block is configured and its IRQs enabled unconditionally,
   even in TPG mode, against a stale phy_sel with a powered-but-idle
   sensor on it (the kernel powers the sensor up inside ACQUIRE_DEV).

2. **Fresh-boot trap: sensor ACQUIRE_DEV EINVAL.** On a clean boot
   --tpg died at `sensor ACQUIRE_DEV: Invalid argument`. Kernel:
   cam_sensor_core.c CAM_ACQUIRE_DEV rejects while is_probe_succeed
   == 0 — the old boot had earlier sensor-mode runs that had probed;
   a fresh boot has not. Fix: --tpg still runs the real I2C probe
   (unchanged code); only regs-after-probe stay mode-dependent.

3. **The actual deadlock — per-frame NOP dropped by state, Skip
   Frame forever.** With probe+acquire fixed, streaming still timed
   out but the log now said, at every SOF:
   `Skip Frame: req: 1 not ready on link 0x3c0304 for pd: 2 dev:
   cam-sensor open_req count: 1`. Kernel chain: the NOP opcode
   handler (cam_sensor_i2c_pkt_parse) silently drops our req-1 nop
   while sensor_state is INIT/ACQUIRE ("Rxed NOP packets without
   linking"), so cam_sensor_update_req_mgr never registers req 1 for
   cam-sensor; the req mgr then refuses to apply req 1 to ANY device
   on the link — including the ISP request that carries our fence.
   Sensor state only reaches CAM_SENSOR_CONFIG when a CONFIG packet
   with real i2c settings is applied. Fix: --tpg now runs the same
   global-INIT + mode-CONFIG packets as sensor mode (harmless,
   already-exercised writes); only the STREAMON packet, csiphy
   config/start and sensor START stay skipped. Next run: fence
   signaled, buffer nonzero, frame dumped. This very likely explains
   the sensor-mode timeouts too (there the state was CONFIG, but the
   sensor never emitted — the missing register tables remain the
   sensor-mode blocker, now the ONLY one left).

4. **Frame content (observed, decoded on host):** static split color
   bar exactly as the kernel's TPG config programs it — pixel levels
   clean 10-bit 1020/0 (white/black, no noise), rows 0–615 begin with
   410 white pixels, rows 616–1231 begin with 410 black (halves
   inverted), a 1-pixel alternating band between, row N identical
   across the frame (static). 1,010,240 nonzero bytes = 820 nonzero
   pixels/row exactly. csid-lite IRQ line: 12 interrupts total for
   the whole run (the old storm was 1.9M+ per boot).

5. **IRQ baseline reset:** the 1,924,669 accumulated csid-lite count
   was boot-lifetime accumulation of teardown storms on the old
   8-hour boot, not a live fault — fresh boot shows 0 before, 12
   after the TPG capture.

Device state: our rootfs on patched vendor_boot (unchanged this
session), camera stack via camera-bringup rcS, /bin/cam-shot =
--tpg-capable build, debug_mdl=0, csiphy_dump=0 (fresh-boot
defaults, verified). Remaining for M19 real capture: the sensor
register tables (rear imx363 metric bin holds them — decode the
Parameter Parser V2 blob — or capture stock-Android I2C traffic).

## M19 camera: REAL SENSOR FRAME — the blocker was lane_cfg identity, not signal integrity (2026-09-01, fifth session)

Continued from the TPG-first-frame session. This session: decoded the
vendor sensor register tables out of the metric bin, streamed the real
rear IMX363 through IFE/RDI, chased header corruption for a day, and
found the actual root cause: **cam_isp_in_port_info.lane_cfg = 0 is NOT
identity**. Canonical identity is 0x3210. With it the link is clean and
the first real frame landed in the buffer.

1. **Vendor register tables decoded from the Parameter Parser V2 bin**
   (/tmp/extract_regs.py, verified register-for-register vs mainline
   imx355.c for the front). Rear imx363 (metric_imx363_lito2): initSettings
   = 29 single-byte writes; 12 mode tables (regSetting nodes), of which
   mode #544 = 2016x1136 @1836 Mbps/lane (our default) and mode #2610 =
   2016x1136 @1128 Mbps/lane (FLL 2488, LLP 4176, OP_MUL 188, pck ≈312
   MHz). bin TOC also exposes laneAssign (=0 for both rear and front —
   that field feeds cam_csiphy_info, NOT the CSID port; don't confuse
   the two). Also: TOC[23] is a full 84-write 4032x3024 @1368 Mbps mode.

2. **Real streaming: sensor transmits, headers garble.** First real-mode
   runs (mode #544 @1836): RX IRQ histogram ~95x 0x0d040ff (bits: SOT+EOT
   all lanes, WARNING_ECC bit14, UNBOUNDED_FRAME bit24, TG_FIFO_OVERFLOW
   bit26, RST_DONE bit27 — %x strips leading zeros, 0x0d040ff prints as
   0xd040ff) + a few ERROR_ECC events. Rate ladder: 1128 (mode #2610,
   --slowrear) same dominant event, 10 ERROR_ECC+10 STREAM_UNDERFLOW kmsg
   prints; 564 (OP_MUL 188->94, --rear564) 18x 0x1000ff ERROR_ECC —
   WORSE. U-shaped error curve, vendor-validated 1128 the cleanest of
   the three, but none clean.

3. **DT/VC sweep via packet-capture registers: all negative.** Armed
   CSID capture with (VC0, 2B/2A/2C/2D/12), (VC1, 2B/2C/2A/12), VC2/VC3
   — no long packet ever matched. Short captures: armed VC0 latched
   VC:0 DT:0 LC:0 (FS decodes); armed VC1 latched VC:1 DT:1 LC:16721 (a
   frame-end short packet, DT=0x01 correct, LC nonsense) — FE arrives but
   corrupted. Broadcast short packets (FS/FE) replicate on all 4 lanes so
   they survive any lane garbling; long-packet headers are byte-striped
   1 byte/lane so a wrong lane mapping corrupts every one of them. That
   asymmetry (FS clean, long headers never decode) was the signature of a
   lane-mapping problem all along — earlier "lane permutation ruled out
   because FS decodes" reasoning was backwards.

4. **Register-state parity with the vendor achieved — not the cause.**
   Exact diff of our INIT vs bin initSettings: ours = vendor's 29 writes
   + one prepended {0x0112, 0x0a} (vendor never writes 0x0112). Dropped
   it (--keep0112 restores); also fixed the stale IFE timing model
   (SENSOR_DIMENSION blob was still mode-#544's vbi=1652/pck=208M under
   #2610; now 2488/312M). Neither change moved the histogram. Sensor
   state was byte-identical to vendor and the link still garbled.

5. **Kernel has no D-PHY rate tuning.** cam_csiphy_cphy_data_rate_config
   (the per-rate analog table) runs only under csiphy_3phase; the D-PHY
   path programs lane enables + settle counts only. data_rate in the
   csiphy blob is inert for us — no RX equalization knob exists from
   userspace.

6. **lane_cfg root cause.** cam_ife_csid_core.c writes cfg0 =
   (lane_num-1) | lane_cfg<<4 | ... unmasked. Mainline camss-csid-gen2
   documents the register: RX_CFG0 [7:4] DL0_INPUT_SEL, [11:8] DL1,
   [15:12] DL2, [19:16] DL3 — each logical lane selects its physical
   source. lane_cfg=0 therefore means ALL FOUR logical lanes read
   physical D0 — headers stripe garbage. Canonical identity (uapi "4
   bits per lane" encoding): lane_cfg = 0x3210.

7. **Full sweep that found it** (--lanecfg, one run each,
   --stream --rear --slowrear --rawvendor): singles 0->mode A (garbled),
   1->mode B (501x ERROR_ECC, one RDI EOF), 2/3/6/7 dead (RX silence),
   4/8 no-op, 12 (bits6+7) adds ERROR_CRC+UNDERFLOW; all 24 permutations:
   only 0x3210 clean; 0x132/0x321 garbled-alive; everything else dead.

8. **The frame** (0x3210, reproduced 3x incl. no-flag default run):
   RX 370-550x 0x04000ff (WARNING_ECC only, corrected; no ERROR_ECC, no
   UNBOUNDED/TG_OVF, no CRC failures), LONG_PKT_CAPTURED VC:0 DT:0x2B
   WC:2520 ECC:0x1b (2520 B = exactly 2016 px RAW10), RDI0 SOF+EOF per
   frame (5-6 pairs), fence SYNC_WAIT result=0 (signaled),
   buffer(frame) nonzero 2860467-2860495/2862720, mod-5 histogram
   820/819/819/819/812 per 4KB (RAW10 packing as expected). Decoded on
   host to PNG (/tmp/m19-first-frame.{raw,png}, frame md5
   bab36c2c8edb84b9f3f280f1149db01): a real photograph — dark room,
   two ceiling lights, door frame. Sensor gain/exposure defaults are
   dark; a brighter frame needs exposure control (0x0202 coarse).

   cam-shot default now g_lanecfg=0x3210; --lanecfg still overrides.

Device state at session end: MATRIX_CONF_A cleared (0x00),
/lib/modules.aginx/cam_ife_csid.ko restored to stock build (md5
3f9bbc83b09942baeb582d80c0e3b2ef, verified on device); loaded modules
still carry this boot's debug_mdl=0xffffffff/csiphy_dump=1 params (gone
at next boot); /tmp binaries+logs vanish at reboot. Remaining for M19
closure: exposure/gain for a usable image, JPEG/encode path, front UW
(slot 1) same treatment, multi-request queueing (single request worked
here — "Apply failed in Substate[SOF]" did not block once data flowed).

## M19 camera: capture persists across reboot; exposure/gain + on-device PNG; kmsg follower bounded (2026-09-01, sixth session)

1. **Exposure/gain implemented in cam-shot** (imx355-family formulas,
   mainline imx355.c; the imx363's own updateGainSettings bin container
   resists decoding — its slot pairing breaks on non-regSetting-shaped
   entries). `--cit N` coarse integration lines (0x0202/0x0203, max
   FLL-10), `--gain X` analog multiplier 1..16 (0x0204/0x0205,
   val=1024-1024/m clamped 960), `--dgain X` digital (0x3070=1 global
   select + 0x020e = m*256). Observed: mode #2610 defaults are CIT 2474
   (FLL 2488, near max — more TIME is unavailable) and gain 1x;
   `--gain 8` (reg 0x380) visibly brightens the dark-room frame. Dark
   scenes need gain, not exposure.
2. **On-device PNG encode**: `--png` writes /tmp/frame.png — gray8 from
   RAW10 (MSB byte per 4+1 group), filter-0 rows, stored-DEFLATE IDAT,
   software crc32/adler32. Verified on host (2016x1136 grayscale PNG
   opens; pixel content matches the raw). Frame artifacts land in
   /tmp (tmpfs) by design; pull before reboot.
3. **Persistence**: /bin/cam-shot on device = the lane_cfg-0x3210 +
   gain + PNG build (md5 41bcda8b6a75f75a4fb6ebad1ff7379e, 755). Recipe
   synced: camera-bringup updated from the device's newer copy (md5
   f157e2a9e1c92fa7f430d4dc55489150 both sides), build-rootfs.sh gained
   the cam-shot zig-cc build line + camera-bringup/cell-bringup in the
   chmod-755 list (they were 644 in the baked image — the latents that
   made the baked camera path depend on a hand-chmod'd copy).
4. **Fresh-boot verification**: rebooted via /bin/reboot2. rcS ran the
   full chain unprompted — boot.state reached "camera ok media+video
   nodes" and "done ok". FIRST cam-shot invocation after the cold rcS
   load faulted mid-stream (CAM-SMMU iommu fault handler isp ctx 0,
   CAMNOC SLAVE_IRQ err_code=1 address decode @0xdfebb400/0xdfebb300,
   then CDM stream-off failed 32 — full teardown, no frame). Second and
   third invocations: clean. Third run output: fence SYNC_WAIT
   rc=0 result=0, buffer nonzero 2860534/2862720 (99.9%), mod-5
   histogram 820/819/819/819/817, /tmp/frame.png 2291550 B written
   on-device. Frame pulled and confirmed (dark room, --gain 8) →
   /Users/sophiehe/Documents/aginxos-frames/m19-freshboot-frame.png.
   So: capture works from a cold boot with only rcS-loaded modules and
   the persisted /bin binary. The first-run IOMMU fault is reproducible
   knowledge, not noise: retry once after any cold module load.
5. **kmsg follower was unbounded and filled the rootfs**: rcS's
   `(cat /dev/kmsg > /var/kmsg-follow.log)&` had grown to 387 MB on the
   2 GB ext4 — adb push of cam-shot failed ENOSPC at 18 MB free.
   Fixed in rcS (recipe + device, md5 50b02ae9689ff3933edbd1b5b38cb56e
   both sides): loop rotates to kmsg-follow.old with `head -c 32M`
   capping each file (cat dies on SIGPIPE) — the pstore-substitute
   record is now bounded at ~64 MB. Truncated the 387 MB log by hand;
   / is back to 403 MB free (80%). Bounded loop verified running on
   device (one cat|head pair, pid 2194).

Device state at session end: booted AginxOS rootfs, stock vendor_boot,
camera stack loaded by rcS, /bin/cam-shot + fixed rcS +
camera-bringup persisted on /etc-/bin (ext4). Full rootfs re-bake
(#3) NOT done — deferred deliberately (flashes userdata, wipes the
agent install); the recipe is now correct for whenever it happens.

## M19b: front imx355 + ultra-wide imx481 — all three sensors frame (2026-09-01, seventh session)

1. **Front imx355 (slot 2) framed on the first try** after the lane_cfg
   fix — every front attempt before it was the same 0-vs-identity bug
   (the front was cam-shot's default slot all along). Invocation:
   `/tmp/cam-shot --stream --railhelper --gain 8 --png`. Vendor-bin
   mode 1640x925 4-lane 360 Mbps/lane verbatim; fence signaled, buffer
   nonzero 1892325/1896250 (99.8%), PNG on-device; reproduced
   (1892402). Frame: selfie view of the dark room,
   aginxos-frames/m19-front.png. --railhelper (rear INIT held in the
   same session to keep the SLG ldo1..6 + camera_ldo analog rails up)
   is REQUIRED for the front — its DT node only carries cam_vio.
   NB: --gain/--dgain folding was rear-only this run; the front frame
   is at vendor-default gain (folded for slot 1 below, front still to
   do if a brighter selfie is ever needed).
2. **UW imx481 (slot 1) framed on the first try too.** Vendor bin
   (com.qti.sensormodule.metric_imx481_lito2.bin) decoded with the
   same Parameter-Parser-V2 path: init = 209-write initSettings, modes
   = 9 regSettings. Mode table (i = regSetting#1301): 2328x1310
   binned, fll 1888, llp 5120, 4 lanes (0x0114=3). PLL rule
   lane_Mbps = INCK/0x030d x (0x030e<<8|0x030f) = 24/15x439 = 702.4
   Mbps/lane -> pck 281 MHz -> 29.1 fps; cross-checked against mode0
   full-res (24/4x218 = 1308 = the 523.2 MHz pck x 10/4 that the bin's
   own resolutionData declares — rule holds). Mode[4] (2016x1136)
   computes to ~120 fps with its 24/3x230 PLL — the vendor fast mode,
   not used. Slot wiring: --uw, dims/stride 2328x1310/2910, csiphy
   dr 702.4M (settle 24), cfg buffer grown (209-write INIT = 1680 B
   > the old 1536 B cmd buf; cfg_regs[128] -> [192] for the 138-write
   mode). Probe 0x481 @0x34/0x0016 OK with --railhelper. Output:
   fence signaled, nonzero 3808766/3812100 (99.9%), reproduced
   (3808795). Frame: wide view of the same dark room,
   aginxos-frames/m19-uw.png.
3. **Sensor census complete**: all three Pixel 5 cameras produce real
   frames through one tool, one pipeline, zero vendor userspace:
   rear imx363 2016x1136 / UW imx481 2328x1310 / front imx355
   1640x925, each with its own vendor-bin register tables baked into
   cam-shot. /bin/cam-shot updated to this build (md5
   597f47d97539360d25d2fb5ad5c0c23d).

## Rootfs re-bake #3: M19 fully folded — all three cameras from the baked image (2026-09-01)

1. Recipe state baked: cam-shot with the lane_cfg-0x3210 default +
   gain/PNG + UW tables, camera-bringup 755 (the 644 latent fixed),
   bounded kmsg follower in rcS. Image built and flashed
   (fastboot flash userdata, serial 13201FDD4001N8 confirmed;
   46 s write).
2. Fresh-bake boot: touch/camera/battery/modem/audio/wlan all ok;
   "wifi fail no /etc/wifi.conf" as always on a fresh bake.
3. **Restore flow hit one trap worth recording**: the pre-bake backup
   tar was taken from /var (bin+home) and MISSED /etc/wifi.conf —
   the live conf (Legrand AP, the AP the device had been on all day)
   died with the flash. Recovered from .local/backup-pre-bake-0831/
   (ssid=Legrand AP — the 0831b copy is the "666" hotspot, which is
   off: join rc=2). Lesson: pre-flash backup must cover /etc too.
   wifi.conf restored + net-bringup -> 192.168.0.166; internet ok;
   time ok; boot.state reached "done ok". /var/bin (5 packages) +
   /var/home (1 GB) restored from the 0901 tar; 79% used.
4. **Camera acceptance from the baked image, this boot**: UW first
   call (no IOMMU fault this time — the cold-load fault is
   intermittent, not deterministic): fence signaled, nonzero
   3809092/3812100. Rear: 2860449/2862720. Front: 1892026/1896250.
   /bin/cam-shot = baked build md5 412c3757e3b57cafa5fcd0c14b389796.

Device state at session end: fresh-bake #3 rootfs running, wifi on
Legrand AP, agent packages + home restored, all three cameras
working from the baked image. Backups: .local/backup-pre-bake-0901/
(tar: var bin+home; wifi.conf came from 0831, NOT the fresh conf).

## M19c: JPEG compression + color path — baseline encoder, all three sensors verified (2026-09-01, eighth session)

1. `boot/rootfs/src/jpegenc.h`: baseline sequential JPEG, no libraries.
   gray8 (1 comp) and YCbCr 4:2:0 (rgb24 converts internally), quality
   via IJG rule, runtime-built Huffman (merge tree + canonical codes,
   Annex-K quant tables in natural order, DQT emits zigzag). Two decoder
   compat rules found the hard way and now commented in the header:
   - DHT class byte must be `Tc<<4 | Th` — swapped nibbles define the
     luma AC table as "DC table 1" and every decoder dies at SOS.
   - The Huffman code must stay INCOMPLETE (a complete tree assigns the
     all-1s code, which libjpeg rejects as "Bogus Huffman table") — one
     freq-1 phantom symbol (index 256) joins the merge tree but is
     excluded from hbits/hval.
2. Host validation: sips (Apple ImageIO) + PIL/libjpeg cross-decode.
   Synthetic RGGB quadrants decode to the exact source colors (worst
   channel diff 3 of 255); 33x17 odd dims (MCU padding) and all-black/
   all-white DC extremes decode.
3. Device (raw2jpg, musl static, md5 d25b1f30263f2f7c4803ba43a188e98c):
   all three sensors, pulled and sips-verified 2016x1136 / 2328x1310 /
   1640x925 (front: odd height 925 exercises MCU row padding):

   | sensor | raw | gray q85 | color q85 |
   |---|---|---|---|
   | rear imx363 | 2,862,720 B | 453,722 B, 0.101 s | 194,474 B, 0.180 s |
   | uw imx481 | 3,812,100 B | 104,649 B, 0.126 s | 77,703 B, 0.277 s |
   | front imx355 | 1,896,250 B | 81,173 B, 0.085 s | 63,296 B, 0.141 s |

   rear color q-sweep: q50 74,153 / q70 101,301 / q85 166,852 /
   q95 514,714 B. Encode ~0.1–0.3 s/frame on the big core — no integer
   AAN DCT needed.
4. rggb CFA phase visually verified correct for imx363 AND imx481 (same
   indoor scene, matching warm hues; UW wider FOV consistent). imx355
   NOT hue-verified: phone lay flat, lens against the desk, frame near
   black — pipeline + odd-height path verified, color judgement
   impossible as observed.
5. `cam-shot --jpeg [q]` native (md5 5752fbf026742f829c6f67dcd2baa434
   at test): color default, `--jpeg-gray` mode (`--jpeg-gray`/`--jpeg-
   color` also enable, q defaults 85), `--jpeg-out <path>`, default
   output = raw dump path .raw→.jpg. Observed: color 193,364 B /
   0.214 s; gray 450,819 B / 0.125 s. raw2jpg also built into the
   recipe for already-captured dumps.
6. Front first capture reproduced the known intermittent cold-load
   IOMMU fault (CAMNOC address-decode @0xdfebb900-class); one retry
   frames. Same as M19b — not new.

Device state at session end: baked rootfs #3 still running (no flash
this session); test cam-shot + raw2jpg under /tmp (tmpfs, gone at
reboot). Repo jpegenc.h + raw2jpg.c + cam-shot --jpeg land post-
session; re-bake folds M19c.


## M19c burst: multi-request capture fixed — RDI write-master loop_size, not a race (2026-09-01, ninth session)

`--frames 2+` previously produced frame 1 fine and frame 2 zero +
fence timeout, with SMMU PFs at round_up(pix[0] end), CAMNOC address
decode errors, and `RDI Error: STATUS_1=0x4`. Two candidate causes:

1. **Race/bubble (disproven)**: rolling submission requeues req i+1 at
   fence i, but fence signal is EOF-ish and userspace wakeup + 3
   ioctls + CRM apply take ~17 ms vs a 10–18 ms EOF→SOF gap.
   Discriminating experiment (pq1.log): pre-queue ALL requests before
   START (`--stream --rear --slowrear --rawvendor --frames 2`) —
   failed identically. Not a race; deterministic programming error.
2. **RDI write-master multi-frame mode (confirmed)**: kernel-source
   decode of cam_vfe_bus_ver2.c update_wm — for RDI WMs (client idx
   < 3) `loop_size = irq_subsample_period + 1` image_addr writes are
   emitted per request, and the WM hardware auto-advances the write
   address by `frame_inc` (= stride × slice_height = whole frame)
   within the cycle. Our IFE INIT HFR blob set
   `hfr.port[0].subsample_period = 1` → loop_size 2 → kernel wrote
   image_addr twice (base, base+size) → frame 2 physically landed at
   base+frame_inc, exactly past pix[0]'s mapping.

Register decode receipts (cam_vfe170.h / cam_vfe_bus_ver2.c,
LineageOS redbull): top `reg_update_cmd = 0x4AC`, RDI0 update data 2
(the trailing CDM pair in every request is correct); WM client 0:
image_addr 0x2214, stride 0x2228, frame_inc 0x2258, irq_subsample_
period 0x2248. kmd CDM scratch dump (added `dump_kmd`) showed req 1
with 6 pairs (double 0x2214 write: base + base+0x2bae80) before the
fix, 5 pairs after.

**Fix**: `hfr.port[0].subsample_period = 0` (INIT HFR blob, with
`io.subsample_period` left at 1). Pre-queue all N before START is now
the default (rolling kept behind `--roll` for A/B — it still hits the
EOF→SOF race and is not recommended).

Observed (pq2.log / pq4.log, binary md5 dad9d25e006e73864aa68a1228e246bb
— repo source rebuilds bit-identical):
- `--frames 2`: both fences signaled 66 ms apart; frames 2,860,441 /
  2,860,478 of 2,862,720 B nonzero; healthy mod-5 RAW10 histogram.
- `--frames 4 --jpeg 85`: all four fences signaled (~67 ms spacing,
  slowrear rate), four full frames, four JPEGs ~84 KB @ 0.166 s, RC=0.
  One benign CAM_WARN bubble line (detected=0) — no RDI error, no
  faults.

Device state at session end: baked rootfs #3 still running; test
cam-shot under /tmp (tmpfs). Burst fix lands to
boot/rootfs/src/cam-shot.c; re-bake folds it.

## Rootfs re-bake #4: M19b + M19c fully folded — burst + JPEG from the baked image (2026-09-01, ninth session cont'd)

Full re-bake after the M19c JPEG (5429240) and burst fix (70a3635)
commits: `build-phone.sh musl` + `build-rootfs.sh` → out/rootfs.img
(132 MB sparse), flashed userdata (fastboot serial 13201FDD4001N8
confirmed, single device).

userdata flash + state restore, lessons recorded:
1. Device-only /etc state = wifi.conf only (.rcs-ran is the first-boot
   marker — leave absent so rcS runs the first-boot path; resolv.conf
   regenerates). Recipe diff was otherwise empty.
2. /var holds 1.45 GB of provision-installed software (/var/home
   986 MB + /var/bin 480 MB) that fits the 2 GB fs but NOT together
   with its own tar: landing var.tar on /var then untarring hits
   ENOSPC at ~400 MB into extraction. Stream instead —
   `cat var.tar | adb shell "tar -xf - -C /"` — restored byte-exact
   (du 1009808/491600 KB, 417 MB free).
3. Restoring wifi.conf before first boot means provision starts its
   full 1.45 GB Wi-Fi download immediately; kill agpkg/agdl if you are
   about to restore the backup anyway (they compete for /var/tmp).
4. Mid-flash-session "spontaneous reboots" (~every 2 min, kmsg cut
   mid-google_charger line) were NOT crashes: wifi-wizard asks on the
   panel whether to restart the network, and answering y reboots the
   PHONE. google_charger exonerated; adb drops during pushes were the
   reboot. UX note: network-restart should re-run net-bringup, not
   reboot the device.

Acceptance on baked image #4 (clean boot: boot.state modem/audio/wlan
ok; agsvc runs aginx + aginx-carrier + aginxbrowser from restored
/var/bin; provision sees installed state, no re-download):
`/bin/cam-shot --stream --rear --slowrear --rawvendor --frames 4
--jpeg 85` — all four requests pre-queued before START, four fences
signaled ~67 ms apart, four full frames (2,860,6xx/2,862,720 B
nonzero each), four 2016x1136 color q85 JPEGs ~85 KB, RC=0. No RDI
errors, no faults.

Device state at session end: baked rootfs #4 running, /var restored,
Wi-Fi on the Legrand AP.

## M19c ring mode: sustained real-time burst — UPDATE packet pool cap + raw-spill design (2026-09-01, ninth session cont'd)

Long bursts (--frames > 16) run ring mode: MAXF(16) slots pre-queued,
each slot recycled for request f+16 at its fence (retire + fresh
CAM_SYNC_CREATE, SCHED_REQ + UPDATE + NOP on the same buffer/kmd
packet). Two limits and one pacing fact discovered:

1. **Kernel IFE UPDATE packet pool caps ~19 packets** — pre-queueing
   150 died at "isp UPDATE packet 20: Out of memory" (CAM_IOCTL
   failure). The pool is per-context and pre-allocated; pixel buffers
   themselves alloc fine for 150 (only the kmd UPDATE packets don't).
   Hence ring recycling is the only path past ~19 requests.
2. **In-loop JPEG encode stalls the ring**: 0.17 s/frame encode vs the
   67 ms frame period drains the 16-deep window after ~1 s and the
   effective capture rate drops to ~3 fps (150 frames over 51 s —
   observed before the fix). Fix: the wait loop only SPILLS each frame
   to a numbered raw on tmpfs (~3 ms write); a post-encode pass
   (inspect + JPEG + unlink) runs after the burst.
3. With spill, 150 frames take 149 x 66.8 ms = 9.95 s — true real-time
   15 fps (t2033.640 -> t2043.592), all fences signaled, 150 JPEGs,
   RC=0.

Exposure sweep for the dark-indoors complaint (rear imx363, mode
#2610 defaults CIT 2474 = near FLL max, gain 1x): mean YAVG via ffmpeg
signalstats — default 19/255; --gain 4 -> 22; --gain 8 -> 30; --gain
16 -> 44; --gain 16 --dgain 2 -> 68 (usable indoor night image, noise
acceptable). Gain regs 0x0204/0x0205 + dgain 0x020e confirmed working
on imx363 (not just imx355).

Demo artifacts on the Mac: ~/Desktop/burst10s.gif / burst10s.mp4
(150 x 2016x1136 color q85, 15 fps real-time).

Device state: baked #4 running; test binary /tmp/cam-shot-ring3
(md5 = repo build). Ring mode lands to boot/rootfs/src/cam-shot.c;
next bake folds it.

## M19c white balance: gray-world auto WB kills the Matrix green (ninth session cont'd)

User report: "颜色偏绿,好像黑客帝国的那种绿" (image leans green, like the
Matrix). Measured on x10-75.jpg (PIL channel means): R=51.4 G=81.2 B=54.0
→ G/R=1.58, G/B=1.51. Root cause: the RGGB debayer had NO white-balance
gains at all, and G has 2x the samples.

Fix in cam-shot.c (dump_jpeg color path):
- wb_measure(): per-site means straight off the Bayer pattern (R at
  even/even, B at odd/odd, G on the cross), gains normalize all three to
  the BRIGHTEST site mean — every gain >= 1, so brightness is preserved
  and the --gain/--dgain exposure tuning is never undone. Gains capped
  at 4x; means < 1 (black frame) leave wb = 1,1,1.
- cs_debayer() applies the gains post-interpolation, clamps at 255.
- Default is AUTO (per frame); --wb r,g,b manual, --wb off.

Observed on device (rear imx363, --gain 16 --dgain 2):
- wb: auto r=1.55 g=1.00 b=1.47 (matches the manual PIL measurement
  1.58/1.51; site means vs debayered+JPEG means explain the small gap).
- JPEG channel means after: R=74.7 G=79.8 B=75.9 → G/R=1.07, G/B=1.05.
  Residual ~6% = JPEG 4:2:0 chroma subsampling softening R/B; visually
  neutral (white paper neutral, warm desk lamp stays warm — correct).
- 150-frame burst re-run with WB: 10.02 s (t2992.864 -> t3002.885),
  real-time held; per-frame gains adapt as the scene changes
  (r 1.29→1.55 across the burst). Encode cost unchanged (0.19-0.20 s).

Demo artifacts refreshed: ~/Desktop/burst10s.gif / burst10s.mp4 now
color-balanced (150 frames, 2016x1136, 15 fps real-time).

Note for the next bake: the WB binary is /tmp/cam-shot-wb on the device
(test path /data/local/tmp/cam-shot-wb); baked #4 rootfs still carries
the pre-WB cam-shot — fold with the ring-mode change.

Commit: 6557bab (cam-shot.c; jpegenc.h was already tracked by 2f48c3f).

## M19d: IFE PIX path unlocked from userspace — CAMIF program via CDM BL, RAW_DUMP frames on device (tenth session)

Goal: hardware-ISP YUV (route 3). Status after this session: the PIX input
side is fully alive and RAW_DUMP (unprocessed Bayer through the PIX path)
lands real frames from both TPG and the rear sensor. NV12 is blocked at the
color-module stage — the demosaic/CCM/CSC register programs are CAMX-
proprietary and no public kernel programs them (verified across redbull
techpack vfe_top/vfe_bus/vfe17x: no module drivers exist; `module_ctrl.color
.enable = 0x48` is dead code in-kernel).

Root cause of the silent PIX death (carried over): the techpack kernel's
camif start writes only core_cfg/epoch/RUP/STATS-cgc. CAMX normally programs
the rest via CDM IQ commands. We replicated that from userspace:

Channel: cmd_buf desc with `meta_data = CAM_ISP_PACKET_META_COMMON (3)` is
appended verbatim as a CDM BL by cam_isp_add_command_buffers
(`Meta_Common num_ent=2 handle=.. len=128 offset=168` in kmsg proves it
executed). BL payload must be memcpy'd into the mapped cmd buffer — a stack
payload reads as zeros and raises CDM "Invalid command IRQ" (observed).
Encodings: ChangeBase word0=(8<<24)|base24 (IFE1 base 0xB6000); RegRandom
word0=(4<<24)|npairs, then (offset,value) pairs.

The payload that woke the CAMIF up (INIT packet, ordered):
- 0x2C/0x30/0x34/0x38/0x3C = 0xFFFFFFFF — LENS/STATS/COLOR/ZOOM/bus cgc_ovd
  (camif start only un-gates STATS). Not sufficient alone (tested: still
  silent).
- 0x088 = 0xA00 — io_format, PLAIN16(5)<<9 camif raw pack.
- 0x484 = (h-1)<<16|(w-1) — pixels_per_line/lines_per_frame. THE key write:
  with it the CAMIF starts counting lines (first EOF appeared).
- 0x488 = w-1, 0x48C = h-1 — first/last pixel/line.
- 0x494 = 0x1F1F, 0x498/0x49C = 0xFFFFFFFF — subsample period/patterns.
- 0x46C = 3 — camif_input = CAMIF_MIPI_INPUT (enum msm_vfe_camif_input,
  msmb_isp.h). Inherited VFE4.7 semantic; harmless per RAW_DUMP success.
- 0x478 = 0x4 then 0x1 — camif_cmd clear+set input enable
  (msm_vfe47_update_camif_state ENABLE). Nobody in the techpack writes
  0x478 at all. THIS is what turns on SOF/EPOCH/EOF IRQs.
- 0x048 = 0xFFFFFFFF — COLOR group master enable (vfe170 top
  module_ctrl.color.enable; dead code in-kernel). Did NOT unlock NV12 by
  itself; kept as best-known state.

Reference for all of the above: msm_vfe47.c (wahoo lineage-17.0
camera_v2/isp) — same register layout for the CAMIF block on Titan 170.

Observed on device (debug_mdl=8, restored to 0 after):
- TPG+PIX NV12 after payload v2 (geometry): first CAMIF activity ever —
  one EOF at teardown. After v3 (+input select/enable): 147 SOF + 147
  EPOCH + 147 EOF over the run window, clean 30 fps cadence, zero CSID/
  VFE errors, but fence still -110: frame dies inside the color pipe.
- TPG+PIX RAW_DUMP after v3: `SYNC_WAIT rc=0 result=0 (signaled)`,
  /tmp/tpg_pixraw3.raw = 3034000 B (1640x925x2), content 0x03FC color
  bars. First PIX-path frame on device.
- Real rear sensor (slot 0 imx363 2016x1136, --gain 16 --dgain 2):
  fence signaled in 119 ms; /tmp/real_pixraw.raw = 4580352 B = 4032x1136
  PLAIN16_10, live Bayer values (~0x3C-0x47 at those gains, per-pixel
  noise). PIX RAW_DUMP works with the real sensor.
- CSID PXL-path PPP IRQ silence during PIX streaming is expected: its
  IRQ mask (init_config_pxl_path) arms only RST_DONE/FIFO_OVERFLOW/
  CCIF_VIOLATION.

What remains for NV12 (next session): the color-pipe module programs
(demosaic dimensions/config, CSC mode, CCM/gamma). Sources: camx userspace
(not public), or RE of redfin's vendor camera blobs (chromatix bins /
camera HAL .so). No kernel-side shortcut exists in any public techpack.
Device state: baked #4, stock vendor_boot, debug_mdl=0, binary
/data/local/tmp/cam-shot-pix (md5 4dbdada2b4de8456e96ddfb264c02928).
Flags: --pix (NV12), --pix-raw (RAW_DUMP), --ife-base.

Addendum (same session): one more blind experiment, recorded so it is not
retried — the 4.7 YUV-path module program (mainline camss-vfe-4-7.c:
LENS_EN 0x40 bits demux|chroma_upsample, ZOOM_EN 0x4C bits scale|crop|
realign, demux cfg/gains/patterns 0x560-0x578, scale 0x91C-0x964 1:1 with
halved chroma, crop 0x974-0x980, clamp 0x984/0x988) appended to the CDM
payload: NV12 still silent (147 SOF/EOF, fence -110). Titan 170 did NOT
inherit the 4.7 IQ-module offsets, or the Bayer color pipe needs its own
config regardless. Payload reverted to the committed 16-pair version
(binary md5 4dbdada2b4de8456e96ddfb264c02928 = repo = device).

## M14 — A/B self-update: slot mechanics fully mapped, agupd autonomous E2E (2026-09-02)

Task #56. Everything below observed on device this session. The GPT-attr
slot model from M16 turned out to be the AOSP layout; the real one is
Qualcomm's uefi.lnx.3.0 layout (cross-checked against QRD-Development/abl
uefi.lnx.3.0.r12-rel PartitionTableUpdate.c; redfin ABL r3-0.6 matches it):

    bits 48-49  priority (max 3)      bit 54  successful
    bit  50     ACTIVE (selector gate) bit 55  unbootable
    bits 51-53  tries (max 7)

GetActiveSlot only considers entries with ACTIVE set — priority alone
selects nothing (why M16's old tool never switched anything). The old
mark "worked" by bit overlap: 7<<52 also sets bit 54 (succ).

Observed mechanics, all live:
- **Userspace attrs-only switching works, both directions.** agboot-ok
  set-active X (attrs per ABL SetActiveSlot, no GUID swap, no LUN flip)
  → reboot → boots X with full bringup. a→b and b→a both done. The r12
  SwitchPtnSlots GUID swap + UFS bBootLunEn flip are NOT gated on this
  older ABL; the earlier "LUN flip required" conclusion was wrong. The
  boot LUN stays 1 (xbl_a chain) for both slots; stock xbl_a/xbl_b are
  functionally identical. MarkPtnActive flips ACTIVE on every LUN's
  entries each boot (xbl_a/xbl_b included).
- **Tries drain**: every unmarked boot decrements the ACTIVE boot
  entry (live: 7→3→2→1 across failed/hung cycles; 3→2, 7→6 on normal
  unmarked boots).
- **rcS mark** (agboot-ok v2): after `done ok`, boot entry of running
  slot gets succ|tries=7|ACTIVE; other entries ACTIVE only. Observed
  landing on both slots. Note: rcS marks even on `done fail` — the
  wait loop times out at 300 s and runs the marker regardless (M16
  legacy; revisit: only hard no-boots roll back as-is).
- **Automatic rollback (tries exhausted)**: staged b with tries=0 →
  reboot → ABL FindBootableSlot: not bootable → HandleActiveSlotUnbootable
  marked b unbootable (boot_b raw 0x0082 — byte-identical to the stock
  factory parked state), re-activated a via SetActiveSlot, cold-rebooted
  by itself; device came back on a, rcS re-marked. Zero host involvement
  after the reboot command.
- **agupd E2E, fully autonomous**: one `agupd apply` (manifest with
  boot+vendor_boot+dtbo+vbmeta+vbmeta_system, all sha256-verified, all
  written to the inactive slot) → agboot-ok set-active → reboot2 →
  boots the new slot → rcS marks. No fastboot anywhere in the loop.

Failure-class map (each provoked and observed):
- corrupt kernel, intact header, mirrored vbmeta → device-unlocked so
  AVB tolerates the hash mismatch (orange), boots the bad kernel, which
  HANGS: dark screen, no USB, no watchdog rescue. Forced power-cycles
  drain one try each; rollback only if something keeps rebooting it.
  GAP for follow-up: arm a hardware watchdog in rcS/agsvc.
- zeroed boot header → GetAVBVersion misreads header_version=0 → VB1
  path fails → `IsUnlocked && error` → ABL drops to fastboot. No
  rollback, but host-recoverable.
- tries=0 → instant auto-rollback (the good path above).

Session end state: slot a active + marked (pri3/ACTIVE/tries7/succ1);
b = healthy byte-mirror of a's chain (pri2/succ1); boot_b GUID-swap
parity even (fastboot switches + ABL rollback cancel out). rcS boot
state this last boot: done fail (wifi join rc=4, known AP flake #76) —
unrelated to M14; marker still fired after the 300 s timeout.
Deployed: /usr/bin/agupd (md5 0b1daf0795e751d6b8eba595093bb43f),
/usr/bin/agboot-ok (md5 8ac7d78e77cf491e401d94f6c8d1d6e8) — fold into
next rootfs re-bake along with the /etc/aginx-version stamp.

## 2026-09-02 — rootfs re-bake #5: agupd + agboot-ok v2 folded in, version stamp, provision shakeout

`scripts/build-rootfs.sh` unchanged from 0f4b1ea (recipe already had the
agupd/agboot-ok copy lines + `git log -1` → `/etc/aginx-version`); full
musl rebuild then bake → flash userdata → first-boot verify.

- **Bake**: version stamp `aginxos c95e7c1 2026-09-02` (HEAD is the
  local docs commit; image code content = pushed 0f4b1ea). `/etc/wifi.conf`
  (Legrand AP, mode 600) baked in for the first time instead of pushed
  post-flash. md5 on device after boot: agupd 0b1daf0795e751d6b8eba595093bb43f,
  agboot-ok 8ac7d78e77cf491e401d94f6c8d1d6e8, wifi.conf 6405ac38494d315c77a058d912e3ac91 — all match the deployed set.
- **First boot**: auto-joined Legrand AP from the baked wifi.conf, DHCP
  192.168.0.166, internet ok (baidu), ntp ok, `done ok` at ~t+45 s.
  Slot a was already marked (attrs survive a userdata flash — they live
  in the GPT, not userdata); boot_a pri3/ACTIVE/tries7/succ1 confirmed
  via `agupd status` from the baked image, which prints
  `slot _a version aginxos c95e7c1 2026-09-02` + the full slot table.
- **provision first-boot**: aginx, aginx-carrier, aginxbrowser
  downloaded + sha256-verified + installed over the network (the
  provision path itself proven end-to-end). codex (223 MB) truncated at
  163 MB: the AP dropped the wlan0 association under sustained transfer
  (NO-CARRIER, then DNS "Try again" for everything — AP as resolver died
  with the link). In-place recovery that worked (no reboot): `ip addr
  flush dev wlan0` + `/bin/wifi-join` + `udhcpc -n` → same lease
  re-obtained. Retried codex: AP dropped the link again ~2 min in.
  Sidestep: host-side download + `adb push` (USB is immune) +
  `agpkg install` sha256-verified — codex 56f02601… and grok dac1ccb20…
  (from out/m13) installed. All 5 manifest packages present in /var/bin,
  app-registry refreshed.
- **New #76 evidence (twice in one session)**: the Legrand AP kills
  long wlan transfers; after a drop there is no auto-rejoin — wifi-join
  + udhcpc by hand restores the link in ~10 s. A background rejoin/
  watch loop in net-bringup or agsvc would make provision self-healing.
  Also observed: agdl does not resume its `.part` on retry (restarts
  from 0), and agpkg leaves orphaned `$DL/<name>.part` after failure
  (`rm -f $f` misses it).
- **Pre-existing gap (not caused by this flash)**: `/etc/aginx/env`
  (relay creds, aginx unit's env_file) is absent — and was absent
  pre-flash too (pre-flash /etc listing has no aginx/). aginx /
  aginx-carrier units fail-fast: running the binary by hand prints
  "未检测到已配置的 Agent" and exits. Restoring the relay identity is a
  separate task (secret lives outside the repo).

Session end state: AginxOS rootfs #5 on slot a (active + marked, succ1);
slot b = healthy byte-mirror (pri2/succ1); vendor_boot = ROOTFS=1 build
(the standing operating state; stock copies untouched in boot/). All 5
manifest packages installed; aginxbrowser ready, codex/grok registered.

## 2026-09-02 — M20a/M20b: relay restored + network self-heal (net-watch)

**M20a — relay long-connection restored.** Root cause of the dead
aginx/aginx-carrier units was missing identity, not code: `/home/.aginx`
(config.toml/binding.json/agents/carrier state) died with an earlier
userdata re-bake and `/etc/aginx/env` was already an empty dir in the
0831 backup — the env was lost even earlier. Restored `/home/.aginx`
(4.7 MB, 50 files) from `.local/backup-pre-bake-0901/prebake0901.tar`;
a minimal `/etc/aginx/env` containing only `HOME=/home` proved
sufficient (relay endpoint + identity live in .aginx). Result: both
units ready under agsvc, relay ESTABLISHED 192.168.0.166 →
106.75.32.216:8443. aginx reconnects by itself (10 s retry loop) once
the network returns — observed live. env is now baked in the recipe
(`boot/rootfs/etc/aginx/env`); .aginx stays per-device data (restore
path = pre-bake backups until the #86 backup channel exists).

**M20b — net-watch self-heal, proven by deliberate link kill.**
New: `/usr/bin/net-rejoin` (the one in-place recovery primitive: flush
+ wifi-join + udhcpc, 2 attempts, lease-gated), `/usr/bin/net-watch`
(agsvc unit, PID under agsvc; waits for net-bringup's boot verdict,
then probes the default gateway every 15 s — 3 consecutive fails →
net-rejoin; WAN-side loss logged only), and wifi-wizard no longer
offers a whole-device reboot after configuring — it nudges
`agctl restart aginx aginx-carrier` in place instead. Test:
`ip link set wlan0 down` → fail 1/3, 2/3, 3/3 logged (~45 s) →
automatic rejoin (scan picked BSS 44:d1:fa:de:16:b9 this time) →
lease ok, default route back, ping ok — zero human input, total
outage ~75 s. The relay TCP connection survived the heal (same IP);
`agctl restart aginx` on a ready unit verified as the wizard path.
Found and fixed during the test: the fail counter was reset every loop
(one variable double-used) — never tripped; now separate idle flag.
Two cosmetic follow-ups: busybox `ip` does not accept `-4` (silent
empty output — use plain `ip addr`), and a stray "Segmentation fault"
line appears after wifi-join's "keys installed" during rejoin — join
still returns 0 and the link comes up; suspect a busybox applet or
wifi-join teardown; harmless to the outcome, left as a watch item.

## 2026-09-02 — M20c: watchdog — softdog proven, agsvc armed, wedge E2E reset

**Why the M14 bad-kernel hang sat dark.** dmesg at every boot:
`msm_watchdog 17c10000.qcom,wdt: wdog absent resource not present` —
the APPS-side bark/bite IRQ resources are absent from this DT, so the
hardware watchdog never engages and nothing rescues a dead kernel. That
class stays unrescuable on this platform; the defense is agupd's
verify-before-write (and M21 signing). But `/dev/watchdog` still exists
(10:130, devtmpfs): it is the **softdog fallback inside watchdog_v2**
(identity "Software Watchdog", options 0x8180, module [permanent],
used-by msm_poweroff) — a kernel hrtimer + emergency_restart, NOT a
PMIC dog. It has NO watchdog-core sysfs attributes
(/sys/class/watchdog/watchdog0/ is an empty dir) — it is a hand-rolled
qcom misc device, so ioctl numbers must be exact: WDIOC_SETTIMEOUT is
**_IOWR (0xC0045706)**; a _IOW guess (0x40045706) returns ENOTTY
(measured; zig's uapi header has it right).

**Live-fire proof.** `wdt probe` (new /bin/wdt, boot/rootfs/src/wdt.c):
identity/options read fine, timeout set 128 OK, pet 5x. `wdt starve 15`:
countdown stopped at ~8 s remaining → box hard-reset itself, adb
re-attached, log survived — softdog resets an unpetted but kernel-alive
system. `/dev/watchdog` never closed (nowayout) and the reset still
landed.

**Production arming: agsvc is the petter.** crates/agsvc main loop now
opens /dev/watchdog lazily, SETTIMEOUT 180 s, KEEPALIVE every 15 s (12x
margin against the 200 ms poll loop). Contract: supervisor alive ⇒ box
stays up; supervisor wedged (STOP, deadlock, starved loop) ⇒ softdog
reset ⇒ ABL tries/rollback semantics from M14 take over.

**E2E wedge test — PASS.** `kill -STOP $(pidof agsvc)` at 01:37:04
(process alive so init does not respawn — exactly the wedge class).
Pulse: still up at t0+150 s, then `up 1 min` on the next poll. Kernel
restart computed from `01:41:49 − /proc/uptime 101.8 s` = **01:40:07 =
t0+183 s**, dead center in the predicted [t0+165, t0+195] window
(180 s timeout + ≤15 s since last pet). Post-reset self-heal chain all
green without human input: agsvc up 32.7 s, wdt armed 32.9 s
(`agsvc: wdt armed, timeout=180s, petting every 15s` in kmsg), net-watch
rejoined WLAN, ntpd re-synced (device GMT == host wall clock to the
second). Power sampler (M23a) restarted post-test; idle draw ~79-91 mA
@ ~4.46 V while "Charging" status.

## 2026-09-02 — M21: update signing — ed25519 gate in agupd, E2E on device

Unsigned/tampered manifests are now dead at the door. New host tool
`agsign` (keygen/sign/verify; private key in `.local/keys/agupd.key`,
0600, gitignored — public key compiled into agupd as
`AGUPD_PUBKEY_B64`). Signature scheme: detached — base64(64-byte
ed25519) in `<manifest>.sig` next to the manifest (local sibling or
URL+".sig"), verified over the RAW manifest bytes (no JSON
canonicalization anywhere), BEFORE parsing or downloading anything.

Device E2E (`agupd apply --no-reboot`, new binary swapped into
/usr/bin): (1) no .sig → rc=1, nothing attempted; (2) tampered body
("test-1"→"test-EVIL") with the stale sig → rc=1; (3) properly signed
manifest → gate passed ("running aginxos c95e7c1 → applying test-1 to
slot _b" — that print sits after fetch+verify+parse in cmd_apply),
then died at the image-download stage as designed (bogus example.com
URL) — no partition touched. Gate order confirmed in source: verify →
parse → print → stage → verify sha256 → write. Host self-test of
agsign sign/verify/tamper passed first (Verification equation was not
satisfied on tamper). Binary cost: agupd 281 KB → 482 KB (ed25519-
dalek, static musl). Rootfs bake #6 will fold it in; until then the
on-device binary is hand-swapped (state noted above).

## 2026-09-02 — M22: rootfs 自更新 — staged swap E2E 全链路 PASS（三次失败取证）

rootfs 换血成功：设备从 c95e7c1（bake #5）原地换成 590edc5（bake #6，1 GiB），
wifi/relay/.aginx/日志全数回归，slot _b 标记 successful。协议链每一段都有
设备回执：

**agupd apply（pre_staged 路径）。** boot+vendor_boot 写 _b（sha256 校验）→
state tar 4.9 MB 落 64 GiB → 对 userdata 块设备 pread 原位哈希 1 GiB body
（manifest 签名锚定）→ swap 头最后落 8 GiB（commit 点）→ set-active _b。
1 GiB 镜像体由 host 侧 `cat img | adb shell dd seek=2097153` 直灌块设备——
Live fs 只有 926 MiB 空闲，稀疏镜像 adb push 会膨胀，直灌是唯一路径，且
agupd 的块上哈希就是传输完整性证明。

**trampoline swap（kmsg 回执）。** t+29.4 `rootfs-swap: pending (new
1073741824 B, old fs 2040373248 B) — hashing staged body` → t+51.4 `body
sha256 ok — backing up current fs` → t+73.5 `new rootfs written, marker
cleared — mounting`。哈希 1 GiB ≈22 s，备份+拷入 ≈22 s。开机后 `/etc/
aginx-version` = 590edc5，provision 重下 required 档（aginx/carrier/
codex；browser/grok 选装档未装），agctl 全 ready，internet ok。

**失败 1 — ROOTFS=1 少了 USBADB=1：slot _b 七连灭回滚。** 首次打包只给了
ROOTFS=1，产出的 vendor_boot 缺 modules.usb/toybox/props/usb-adb/hold 五件
（对比正在运行的 vb_a 解包差异定位）。_b 起不来（无 gadget、无 sh、无属性
区），ABL 耗尽 7 tries 回滚 _a，swap 头和 state marker 原封未动——协议的
crash-safe 设计经受住了这次真崩溃：旧 fs 无损。pack 脚本现在直接拒绝
ROOTFS=1 而无 USBADB=1。教训：**打包 flag 集合以解包对比为准，不看脚本
echo**（echo 全是 note 级，静默降级无报错）。

**失败 2 — busybox ash 八进制陷阱：state-restore 三次静默不执行。** swap
成功后 /home/.aginx、/var/power 未回归，marker 还在。root cause：marker 的
16 位零填充长度 `0000000004896256` 进 `$(( ))` 被 busybox ash 按八进制解析
（数字含 9）→ "arithmetic syntax error" → 整个脚本中止，且发生在任何输出
之前。欺骗性在于：**手动 sh -x 全是通过 adb shell 的 toybox sh 跑的
（十进制容忍），所以每次手动都"正常"**——用 /bin/busybox sh 复跑当场复现。
修复：先剥前导零再算术。副产品：state-restore 全程自捕获到
/var/state-restore.log（ash 自身的报错行正是破案证据），kmsg 输出改为
先测可写的通道。修复后启动路径回执：t+32.6 `marker found (4901376 B tar)
— restoring` → `ok` → `marker cleared`，状态全回。

**附带修复。** mke2fs 从不 truncate 已存在的输出文件——SIZE=1g 重烤后 stat
仍是 2 GiB（尾部是旧镜像字节）；build-rootfs.sh 现在先 rm。agupd
commit_rootfs_swap 的 blkdev 原以 O_WRONLY 打开，pwrite 能过而 pread 直接
EBADF——pre_staged 校验路径改 read+write（设备 /usr/bin/agupd 已更新为
修复版）。

后续待办（记入任务）：生产 https rootfs 更新需要 agdl 直灌偏移（镜像大小
接近 live fs 上限后无法走 staging）；super 挂的是写死的 _a 子分区（本次
无影响，slot _b 运行时值得复查）；选装档（aginxbrowser/grok）未随 swap
回归，需要"+"选装 UI 落地后按档重装。

## 2026-09-02 — M23b: suspend 阻塞链逐层取证（s2idle/deep + wakeup 源）

**基础设施先立住。** /sys/power/state=`freeze mem`；mem_sleep=`s2idle
[deep]`（deep 默认）。唤醒源清单（sysfs wakeup enabled）：gpio-keys、
qpnp_pon（电源键，PMIC 常供电域）、sec_touchscreen（触摸）；RTC 有
 surprises——rtc0 无 sysfs wakealarm 属性，但 **RTC_WKALM_SET/RD ioctl
可用**（/tmp/rtcal 工具，zig 静态），闹钟 IRQ #203 `pm8xxx_rtc_alarm`
走 pmic_arb，PMIC 侧，wake-capable。CONFIG_PM_WAKELOCKS=y（无活跃锁）。

**阻塞链逐层剥开（每层都有 kmsg/日志回执）：**

1. **表层：** 裸 `echo mem` 报 "Some devices failed to suspend, or early
   wake event detected"——无信息量。
2. **网络层：** /sys/class/wakeup 计数器 diff 指向 qcom_rx_wakelock/
   IPA_WS/wlan vdev_*（活流量事件）。agctl stop 三件套 + 20 次重试环后真凶
   显形：`msm-dwc3 a600000.ssusb: Abort PM suspend!! (USB is outside LPM)`
   → `Abort: Callback failed on a600000.ssusb ... returned -16`。
   **dwc3 在 gadget 绑定且 host 活跃时拒绝系统挂起——这正是 Android
   "插线即醒"的设计**，不是 bug。
3. **解绑层（v4，失败但证据金贵）：** `echo "" >
   /config/usb_gadget/g1/UDC`（configfs 挂在 **/config**，不是
   /sys/kernel/config——后者是空的）瞬间撕掉 USB，**且写进程永久挂死**
   （gadget teardown 死锁）：脚本日志止于 unbind 前一行；heartbeat 连续
   走了 2h20m（wall 与 uptime 同速——系统全程醒着，并非睡死）；kmsg 里
   v4 时段零 suspend 记录（连 entry 都没发起）。只能硬重启救回。教训：
   **UDC 文件写入在这内核上不可用作运行时操作**。
4. **理论修正：** 用户态 `sleep N` 不是唤醒源——CLOCK_MONOTONIC 在
   suspend 中停走（rtcwake 全用 RTC 正因如此）。任何脚本化挂起必须先武装
   RTC 闹钟。
5. **意外发现（独立成立）：** configfs gadget **过不了 VBUS 拔插循环**——
   拔线即断 adb，插回不重连（pullup/session 状态需要软件重建，Android 由
   USB HAL 状态机做）。恢复=重启。产品含义：线缆热插拔需要一个
   vbus-watch 自愈守护（后续任务）。

v5 探针（断线门控 + RTC 武装 + 不碰 gadget + 120s 拔线窗）首次运行因拔线
晚于窗口而干净放弃（零改动路径验证 ✓）。v5b：10 分钟窗 + 结尾自动
reboot2 干净重启，测真实场景（电池供电 s2idle 75s + RTC 自唤醒）——结果
见下。

**v5b s2idle E2E — 全链闭环（14:00 左右实测）。** 拔线 13:58:03 →
脚本检测断线（status=Discharging）→ agctl 静默三件套 → RTC 武装 +75s →
`ENTERED try=2`（try=1 瞬退一次——挂起瞬间被残留事件踢醒，重试环必要）→
kmsg 铁证：

```
PM: suspend entry (s2idle)
pm_system_irq_wakeup: 203 triggered pm8xxx_rtc_alarm
Resume caused by IRQ 203, pm8xxx_rtc_alarm
PM: suspend exit 05:59:17     ← 恰好 75s
```

→ 唤醒后恢复服务、sync、`reboot2` 干净自重启 → 插回 USB 后 adb 直接回来
（用户全程只做拔线/插回两个动作）。75s s2idle 耗电 < 1mAh（charge_counter
分辨率以下）。**结论：电池供电 s2idle 入睡 + RTC 自主唤醒 + 干净退出 +
自动恢复，全部成立。** deep 模式 + 15 分钟浸泡测功耗随后（结果续记）。

**deep +900 实测（15:0x 一轮）：入睡 ✓、RTC 唤醒 ✓、但只睡 20.04s。**
kmsg：`PM: suspend entry (deep)` → `pm_system_irq_wakeup: 203 triggered
pm8xxx_rtc_alarm` → `Resume caused by IRQ 203`。printk 时间戳是
CLOCK_MONOTONIC（不含挂起时间），不能用来判时长；判别器是采样器：
`/proc/uptime` 在本内核含挂起时间（boottime），真挂起会冻结采样器的
`sleep 10`，醒来后下一个样本的 uptime 出现 >10s 的跳变——实测 220→250
（+10 sleep + **20s 挂起**）✓ 与 wall 差值 06:39:53.637→06:40:13.681
（20.04s）互证。**由此校准：跨挂起的 wall 差值是真流逝**，回溯确认
s2idle 的 +75s 是真睡满（且闹钟精确）；charge_counter 零增量诚实一致。

**deep 20 秒早醒之谜（未解）**：闹钟按 RTC 自身刻度设 +900（刻度自洽：
醒态 +60s 实测准点触发、`/proc/interrupts` 计数吻合；RTC 恒定偏置
-53y、走时醒态精确），但 deep 下 +20.04s 就由 IRQ 203 唤醒——比刻度早
~877s。怀疑内核挂起路径重排闹钟/PMIC 域切换副作用。下一轮 deep+120
判别"20s 是否与闹钟时长无关"。另：RTC 偏置 -53y 与系统时间不一致本身
就让 rtc suspend/resume 簿记失真——正解可能是启动时把 RTC 设为真时间
（ntpd 后 settimeofday→RTC），列待办。

**采样器 v2**：原 pwr-sample.sh 每次启动截断同名日志（deep 那轮的样本
只是恰好没人覆盖才幸存）——改为 `idle-MMDD-HHMM.log` 唯一名。

**deep 20s 早醒判别（deep+120 一轮）：系统性，与闹钟时长无关。**
同样 try=2 入睡、同样 IRQ 203 `pm8xxx_rtc_alarm` 唤醒，wall 差
07:39:22.836→07:39:42.643 = **+19.81s**（上一轮 +900 请求是 +20.04s）。
两次闹钟刻度差 780s，实睡差 0.23s——deep 下唤醒时刻不跟随闹钟，恒定
~20s。疑点方向：内核 alarmtimer 在挂起入口重排 RTC 闹钟（alarmtimer
子系统会为内核侧 alarm 重编程 rtc0，覆盖我们 rtcal 设的值），或 PMIC
power-collapse 下闹钟比较行为变化。判别实验（待办）：deep 完全不设闹钟
看是否仍在 ~20s 醒（若是→RTC IRQ 线自身在 power-collapse ~20s 产生
事件；若否→alarmtimer 重排是根因）。s2idle 不受影响（+75.0s 精确），
故长浸泡功耗测量走 s2idle。

**s2idle +900 首轮（15:47 拔线）：只睡 409.5s，alarmtimer 现身。**
try-2 打出 `Abort: Callback failed on alarmtimer ... returned -16`
（alarmtimer 子系统在挂起路径会重排 rtc0 闹钟——deep 20s 之谜的机制
就在这里），try-3 入睡 07:47:06.089→07:53:55.611 = +409.5s 后被一个
**非 IRQ 定时器**唤醒（无 pm_system_irq_wakeup/resume-cause 行）——
s2idle/freeze 下 CLOCK_REALTIME 类定时器照常运行，长浸泡会被任意挂起
不可停的定时器切短。我们的 RTC 闹钟（+900）没到点（刻度差 490s），
不是它唤醒的。功耗数字未落地：charge Δ=0（409s@~8mA≈0.9mAh 在
1mAh 分辨率下），采样器挂起中冻结、醒来后 reboot2 又跑赢下一个 10s
采样。发现 `/sys/kernel/wakeup_reasons/{last_resume_reason,
last_suspend_time}` 存在——v6 探针醒来即抓，点名唤醒者。

**qg-fifo-done 静音尝试（败）+ v7 多循环绕行（成）。** 静音路线全部堵死：
qg 平台设备 `c440000.qcom,spmi:...@2:qpnp,qg` 无 `power/` 目录；
`/sys/kernel/irq/313/wakeup` 是 0444 只读（SELinux Disabled、root、sysfs rw
下 chmod 644 成功但 store 全语法 `0/off/disabled/enabled/1` 一律 rc=1 静默
失败——此内核把 irq wakeup 属性编成只读）。改绕：v7 每循环 RTC+120s 睡、
醒后立即再睡，15 循环（2026-09-02 16:35 拔线 ~26.5min 窗口）。结果：
**15/15 循环全部入睡+唤醒**；12 次被 IRQ 203 RTC 精确唤醒（arm 差 121s =
120 睡 + ~1s 醒转），qg-fifo-done 只切短 c4/c8/c12 各 ~47s——arm 时间戳
1761417→1761828→1762238，**tick 节拍实测 410/410s**，与此前两轮 409.5s
切割互证。累计真睡 ≈ 12×120 + 3×47 ≈ **1575s（窗口占比 ~98.7%）**。
脚本 bug：`last_suspend_time` 是两行（abort 行+duration 行），`awk '{print
$2}'` 取空 → susp 全记 0s；时长由 arm 差恢复，无需重跑（下版取 `$1` 第二行）。

**M23a 首个功耗界（s2idle）**：整窗 charge_counter 4187→4187 mAh **Δ=0**
（1595s，FIFO 报告粒度 ~410s + 1mAh 分辨率，两端边界噪声 ~±0.9mAh）
→ 窗口平均电流 **< ~2.3mA**，扣除醒转间隙（~20s×−60mA）后 **s2idle
平均 < ~4mA ≈ <18mW @4.4V**（下界不可分辨）。醒转间隙实测：采样器唯一
幸存帧 −59.8mA@4400mV（服务全停、灭屏醒态，含 modem/wifi 待机）；
脚本醒后即读 −198mA 为 resume 瞬态（与 Δ=0 不相容，弃）。电压
4418.8→4399.6mV 是拔线后弛豫，非耗量证据。**结论记为区间/上限，非点值**；
拿点值的路径：≥3h 连续浸泡（Δ~10mAh 量级）——可被动挂机过夜跑
~100 循环版 v7。

**rtcal 转正 + RTC 写时间被策略门挡（2026-09-02）。** /tmp 的 zig 一次性
rtcal 转正为 `boot/rootfs/src/rtcal.c`（zig cc musl static，进 rootfs 配方
→ /bin/rtcal），子命令：无参=dump（since_epoch+alarm）、`set <epoch>`=
布防（沿旧语义，探针不换脚本）、`arm <+delta|epoch>`、`sync`=系统时间
写入 RTC（先解除残留闹钟防时跳后立即触发）。设备实测：`arm +15` 醒态
准点触发（IRQ 203 计数 0→1，触发后 enabled 自动归 0），dump 正常；
设备 /var/tmp/rtcal 已换 C 版（zig 版备份为 -zig.bak，源已失传）。
**`sync` 被拒：RTC_SET_TIME → EACCES** —— pm8xxx_rtc 驱动的 allow_set_time
策略门（DT 未声明 `qcom,allow-set-time`，set_time 按设计返回 -EACCES；
读时间/闹钟布防不受限，与我们的醒/唤醒路径无冲突）。net-bringup 的
ntpd 后 `rtcal sync` 钩子已埋（best-effort），在 DTB 补丁打开开关前
静默失败。**待办：如需真时间进 RTC，给 vendor_boot/boot DTB 的 rtc
节点加 qcom,allow-set-time 再刷**——收益仅限开机早期 wall 正确（ntpd
本就纠正）与刻度整洁，不在关键路径上，暂不做。−53y 偏置下闹钟自洽
刻度已验证精确（v7 十二循环 121s 全准点）。

**选装"+"（M23 tiering）：agpkg 分层 + aterm 选装页。** manifest 行加
第 4 栏 `[core|opt]`（缺省=core，旧行为兼容）：`sync` 只自愈 core；新增
`available`（列未装 opt）与 `opt-in <name>`（下载安装 + 播种
/var/apps/<id>/app.toml；已知应用优先复用 /etc/apps.d 种子保留原
scale）。解析全 sed 组（busybox awk 在本机段错误，案底见 net-bringup）。
aterm 启动器 builtins 头部加 "+" 砖 → `Mode::Picker`（SELECT PKGS 列表，
点击行同步跑 opt-in，先画 INSTALLING 帧再阻塞；装毕双列表刷新、注册表
即现启动器；BACK 返回）。**CLI E2E 全绿（2026-09-02）**：adb reverse
隧道 + 本机 http.server 喂 corehello(3 栏)/opthello(4 栏 opt) 测试包——
`available` 只列 opt ✓、`sync` 只装 core ✓、`opt-in` 下载+sha 校验+安装+
播种 ✓、`available` 清空 ✓、二进制运行 ✓。测试包已清理。**grok 转
opt tier**（manifest 第 4 栏 + 注释，/var/bin/grok 已移除备份
/var/tmp/grok.bak）作为真实选装候选：`agpkg available` 现列 grok，
经 "+" 装回走真 GitHub 路径。**UI 触点未上机**（无触摸注入）：aterm
新二进制已部署并由 respawn 拉起（pid 4527，无崩溃循环），"+" 砖与
选装页的目击验证待用户手指。

**"+" 触点上机（2026-09-02，用户手指）**：启动器 "+" 砖 → SELECT PKGS
页渲染正常（列表/hit 区）→ 点击 GROK 行 → INSTALLING 帧出现、下载真实
发生（grok.part 增长）。**大包慢源实测**：grok 172MB，GitHub 直连
27KB/s → ETA 27 分钟——v0 同步阻塞安装 = 屏幕 27 分钟冻死无响应、
无取消。止损：kill agdl/agpkg 重试链（agpkg 的 gh-proxy 回退会再拉一
次，须连杀）→ aterm 弹回选装页（FAILED 路径，pid 稳定未崩）；grok
从 /var/tmp/grok.bak 秒装回（sha 与 manifest 同，/var/apps/grok/
app.toml 未动，启动器砖恢复）。**跟进两项（用户指示晚点处理）**：
① aginx.net 软件镜像（#87，服务器侧，客户端零改动——治"慢"）；
② aterm 异步安装+进度+取消（治"冻"）。carrier 非启动器应用属设计
（agsvc 服务，pid 在跑），用户疑问已释。

**键盘 10 列网格化（2026-09-02，用户目击通过）**。用户报"10 个格没对
齐、没铺满一行"并定调用百分比思维。核实（以行可用宽 span=屏宽 94.8%
为基准）：数字/字母四行是固定格宽 cell_w=90px（6×8 字体网格量化 +
迁就最宽阶梯行 10.75 格），10 格只铺 87.9%~94.4%，右缘阶梯参差
928/950/973/995px；而箭头行(7键)与特技行(5键)是 span/N 均分满宽
(~99.8%)——两套布局数学并存。间隔也是三个常数（横向 8/6/8px、纵向
8/4px）。**修复**：四行全部每行 10 键 → 正 10 列网格，格宽=span/10，
列对列全对齐（1/Q/A/Z 同竖线），右缘统一 ~100%；阶梯 row_off 整个删除
（物理键盘的遗产，触屏不需要）；间隔收敛为单常数 gap=span/128
（≈0.8%，1080 屏=8px）横纵全键盘等宽；行高 118px 不变（封顶 KB_ROW_H，
终端显示区一点没动）；hit-test 与渲染同一套网格数学。设备：新二进制
md5 5524d84a…推 /usr/bin/aterm（旧版备份 /var/tmp/aterm.bak），
handoff 拉起新 pid 11776，**用户看过确认"现在键盘挺好的"**。

**M24 — `ag` 单入口路由器 + 元数据协议 v1 + 重烤 #7 收口（2026-09-02/03，
全程设备实测）。** `crates/ag`（libc+serde_json，无 tokio/clap）→
/usr/bin/ag（musl static 427KB）：execve 派发退出码与信号直通；AG_CMD_PATH
默认 /var/bin:/usr/bin；最长前缀（argv[1..k] 连 '-'，k=n..1，stat 命中即
停，余参原样）；精确命中只 stat 不读元数据（快路径）；`--help/-h` 任意位置
拦截（解析+打印+exit 0，目标绝不执行）；`ag:args=` 含 `<x>` 裸调用拒
exit 2；未知 → 前缀列表+did-you-mean（编辑距离≤2）exit 127；`ag commands
[--all|--json|--check]`（--json 出 D1 信封；--check 五类 lint：碰撞/缺
summary/坏布尔/ag:exec 目标缺失/编译件缺 .agmd，组未注册仅警告）。
首批 17 个 sh shim（cam/snd/net/sys/pkg/svc/agent/cron）+ /etc/ag/
groups.desc；build-rootfs.sh 挂 `ag commands --check` 门禁（对构建树跑
host 版 ag，17/17 过才 mke2fs）。host 16 个子进程测试（哨兵文件/信号
直通/信封往返）全绿。**设备验收（换机前 adb 推二进制 590edc5 上）**：
菜单分组渲染 ✓；`ag sys reboot --help` 拦截且 uptime 6:39→6:40 不变
（reboot2 未执行——Omarchy 事故类）✓；`ag snd-cap` 裸调 exit 2 ✓；
`ag cam-sho` → 127+前缀匹配 ✓；`ag svc list` 真穿透 ✓；`ag svc status
bogus-unit` → ERR exit 1 直通 ✓；信封合法 ✓；路由器 CPU 开销 x50 实测
~0.5ms/次（0.04s vs 0.03s）。**重烤 #7（1GiB，stamp "aginxos 8c7c00d
2026-09-02"，sha 2289d728…e34ba9）走 M22 换机路径**：manifest+ed25519
签名（agsign）；boot=stock（e2ce2f17…）+ vendor_boot 重打包
HOLD=1 USBADB=1 ROOTFS=1（unpack-diff vs 运行槽文件清单一致，sha
d80b8098…）；rootfs body `dd bs=4096 seek=2097153`（=8GiB+4096）流灌
47.7s@21MB/s，回读 sha 相符后 `agupd apply`：boot_a/vb_a 写入+校验、
state tar 5169152B 落 64GiB、pre_staged body 复验、swap header 最后
落（len 1073741824，old fs 1020702720）、agboot-ok 置 _a active、自
reboot2。**换机后 clean 验收全绿（烤进 /usr/bin 的 ag，非 adb 推）**：
开 15×5s 回网，/etc/aginx-version=8c7c00d；菜单/17 OK check/拦截
（uptime 3min→3min）/裸调 exit 2/127+建议/信封/穿透 exit 1 直通全过；
rtcal 在位（换机前 check 曾正确抓到 590edc5 烤里缺 rtcal 的真漂移，
#7 解决）；x50 路由 0.04s wall（≈0.8ms/次，热缓存下与直执不可分辨）。
**插曲与诚实记录**：① 换机后 Legrand AP 半死（assoc+M4 成功但网关
不可达、dhcp leasefail、静态 IP 也不通）——provision 6 分钟门到点静默
退出，未装 /var/bin；按"大包走 adb push"回执恢复：主机 gh-proxy.com
下载 manifest 钉住的 v0.1.0 资产（GitHub 直连 10 分钟超时），aginx+
aginx-carrier sha256/md5 双验后推 /var/bin，agctl start 双单元 ready；
aginxbrowser/codex 留给有网下次启动自愈。② **carrier v0.1.0 无
agent/cron 子命令**（只有 start/web/acp/info/probe/notify/qr-login/
ticket）——agent/cron CLI 面在源码 v0.2.0（8e1b2d2 已打 musl 资产）；
`ag agent list --json` 目前穿透为 clap 错误 exit 2（路由本身行为正确）；
**manifest 升 v0.2.0 归 M25**（carrier retrofit 里程碑，顺手做，避免
push 高版本被 provision 按 v0.1.0 sha 打回）。③ adb shell（/system/bin
mksh）PATH 无 /usr/bin——系统 PATH（PID1 environ
/sbin:/bin:/usr/sbin:/usr/bin:/system/bin）正确，纯 adb 测试要手动
export。④ 烤进文件 owner=501（mke2fs -d 从 macOS 树保 uid；agupd 等
先例相同，root 执行不受影响，mode 755 已核）。⑤ 烤 #7 里的 ag-sys-rtc
还是旧 args（裸调会被误拒）——树里 c58264f 已修，进 #8。⑥ 墙钟
~7.5h 偏（AP 死→ntpd 没跑；pm8xxx set_time 政策门见上条）。**设备终
态**：slot _a active 跑 8c7c00d，aginx/aginx-carrier/net-watch ready，
ABL 7 次未标记自回滚兜底在岗，旧 rootfs 备份在 32GiB。

**M25 — HOME 统一 /home + agio 信封 + carrier retrofit（2026-09-03，设备
实测收口）。** **HOME**：aterm(main.rs setenv) 与 agsvc(spawn 默认) 原指
/var/home 而 carrier 单位走 /etc/aginx/env 的 /home——终端里的 agent 与
daemon 对 ~ 不一致；且设备上 /var/home 从未存在（无孤儿数据要迁），
/home/.aginx 才是真数据（state tar 覆盖 /home，旧路径反而不在备份里）。
两处改 /home 后推新二进制：agsvc 经 inittab respawn 轮换（kill 后 8s 内
回岗，四单元 ready），**/proc/<carrier>/environ 实测 HOME=/home** ✓；
aterm kill 后 handoff 循环 2s respawn（新 pid，md5 验），二进制字符串
/var/home 0 处、/home 在 ✓——aterm 里 shell 的运行时 $HOME 观察需屏幕
开终端（留用户手指，同"+"砖先例）。ATERM 运行时观察待手指。**agio**
（crates/agio，仅 serde_json）：ok/ok_meta/fail/fail_hint + exit 助手，
error.type 封闭集（usage→exit2 余 1），stdout 永远可解析、stderr 归人；
`ag commands --json` 改经 agio，形状与 M24 手写版逐字节同构（16/16 测试
不变）。**carrier v0.2.1**（aginx-carrier 仓库 9a2b10b，release 已切）：
`agent list --json` 与 `cron list` 从 JSON Lines 归一为 D1 信封——设备经
烤入路由器实测 `ag agent list --json` →
{"data":[],"meta":{"count":0,"local":0,"remote":0},"ok":true} rc=0、
`ag cron list` → {"data":[],"meta":{"count":0},"ok":true} rc=0（空列表
提示走 stderr）✓；无存量机器消费方（carrier-panel 只吃人读 info，实测
info 面不受影响；aclone 未落地）故直切不加兼容层。**manifest v0.1.0→
v0.2.1**（设备 /etc/agpkg.manifest + 仓库配方同步）：v0.1.0 无 agent/cron
子命令（M24 收口时发现，clap 报错穿透即其证），不升则下次 provision 把
手机上的信封版静默降回；现推送二进制=release 资产（sha bbac1017…），
provision 对它是 no-op。**adb 壳假象记录**：adb mksh 无 HOME，从 adb 跑
`aginx-carrier info` 显示"数据目录 /.aginx/carrier"——仅当前 CLI 进程
的显示，daemon（/proc 实测 HOME=/home）不受影响。aterm/agsvc 为推送到
运行 fs 的覆盖（/usr 持久，重启不丢）；**烤 #8（M26 时点）折入**：
aterm/agsvc/ag(agio)/manifest v0.2.1/ag-sys-rtc 头。设备终态：8c7c00d
烤 + 推送覆盖，aginx/aginx-carrier/net-watch ready。

**M26 — agpkg Rust 重写：签名 manifest + 四件套（2026-09-03，设备实测
收口）。** **签名链**：agsign 长出 lib（AGUPD_PUBKEY_B64 挪入，agupd 内联
副本删除——一钥匙一链，更新与包同源）；/etc/agpkg.manifest 现带 detached
ed25519 .sig（build-rootfs.sh 内容校验式自动重签，git 不保 mtime 故不用
时间戳判陈旧）。设备实测：.sig 挪走后 `agpkg sync` 拒（rc=1，auth/
manifest_unsigned）✓；恢复后 sync 过签名门、aginx/aginx-carrier 报 up
to date 并**落 stamps**（legacy 疗愈路径：无 stamp 时按二进制自身 sha 对
manifest，M26 前安装即此愈合）✓；aginxbrowser/codex 缺失走下载（AP 仍
拥塞，FAILED kept previous，rc=1——自愈语义正确）。**四件套**（手工 tar：
bin/demo + pkg.toml[name/#[service] cmd/type/autostart] + SKILL.md，ustar
格式）：`agpkg install demo <tar> <sha>` → /var/bin/demo(mode 755)、
/var/lib/agpkg/skills/demo/SKILL.md、units/demo.toml（[unit]name+[service]
原样序列化）、stamps/demo=tar sha，**agctl reload 即时生效——`agctl
list` 无重启出现 demo 行**（M16 覆盖通道首次有写入者）✓。`ag pkg list`
人读三行（demo 带 skill,unit 旗标）、`--json` D1 信封一条
{"ok":true,"data":[…],"meta":{"count":3}} ✓——demo 的 stamp=tar sha 而
binary sha 不同，正是 tar 包的 stamp 语义。**篡改拒绝**：假 sha 安装
rc=1 + hint ✓。**回滚**：装 v2（SKILL 变 v2、.prev=v1 二进制）→
`agpkg rollback demo` 二进制回 v1、stamp 作废（下次 sync 重愈）、skill
保持 v2（回滚仅二进制，v0 语义不变）✓。**mode 修正**：首版 fs::write
落 666（设备 umask 0）——skills/units/stamps 改显式 644（skill 文档是
agent 消费面，不该全局可写），重推重装实测 644 ✓。`ag commands --check`
17 OK（ag-pkg shim 头更新后）。sh agpkg（179 行）退役删除。**首启
provision 绿未单独验**（需重启；签名 manifest+新 agpkg 已在位，下次开机
provision 即走签名门——留 bake #8 重启观察）。设备终态：8c7c00d 烤 +
/usr/bin/agpkg(Rust)、/etc/agpkg.manifest{,.sig}、demo 四件套（一次性
验收件）推送在位。

**M26 续 — 烤 #8 换机 ×3：/var/lib 存活闭环 + 两处换机洞（2026-09-03，
设备实测）。** 烤 #8（SIZE=1g，rootfs sha a5fc358c…，流式 dd 到 swap body
+ 读回 sha 精确匹配；boot.img 取自 boot_b 槽读回=镜像、vendor_boot 复用
#7 的 d80b80…——分区整读含填充勿比对）。**第一次换机 /var/lib 未存活**：
/var/lib/agpkg 全无。三重根因实证：(1) 64GiB 残留 tar（marker 只清头、包
体还在）TOC 只有 etc/home/root/var/log/var/power——无 var/lib，apply 时
跑的是旧 fs 的旧 agupd（8c7c00d 烤，旧 scope）；(2) 烤 #8 镜像内的 agupd
md5=62a39294 与新源码构建一致——新二进制在镜像里，只是没在 apply 前到旧
fs 上；(3) 烤 #8 树本无 /var/lib 目录。**trampoline 时序**：换机开机 rcS
state-restore 在 up≈69s 才跑（1GiB 拷贝吃掉前一分钟），解包+marker 清零
已由 trampoline 完成——/var/state-restore.log 只剩一行 start（kmsg 路径
吞掉其余），marker 区读回 0xFF=已消费、无悬挂 commit 头。**第二次换机
state tar 截断事故**：/root/bin 预置 330MB 二进制后 apply，tar 建在
staging_root（/tmp=tmpfs）上死于 235491328 字节（内存压力杀 tar），agupd
只验 0<len≤512MiB → 部分包照收，swap 开机解出截断 codex（134529536B、
mtime 1970-01-21=未校时钟期写入）。apply 尾部自带 `reboot2 reboot`
（--no-reboot 可关）——事后 adb 读数全落在竞态窗口里才显得灵异。
**修复 1（agupd 硬化，已实测）**：state tar 改落 /var/tmp（真 fs）+ tar
退出码非零即 die（"would ship a torn state"）。实测：tar 因烤 #8 无
/var/power 退 1 → 拒绝 ✓（顺带暴露：旧 fs 的 /var/power 是 M23a 采样器
运行期建的，烤树从未有过）；补 mkdir 后 apply 干净（4,869,632B）。烤树补
var/power + var/lib/agpkg/{skills,units,stamps}。**第三次换机（--no-reboot
受控）成**：预检 tar TOC rc=0、105 条、var/lib/agpkg/stamps×4+skills+
units 俱在包内；reboot2 后 up=151s 时 **stamps 四枚全在**（mtime 由 tar
保留）——**/var/lib 随 state tar 存活闭环 ✓**。**修复 2（agpkg sync 自愈
洞，已实测）**：stamps 存活而 /var/bin 被换机清空 → sync 只比 stamp 报
"up to date"，一件都不重装（三换后 /var/bin 空、provision pkg fail）。
up-to-date 判据补 binary 存在性（单测：stamp+无 binary→走下载路径、
stamp 保留）；推送后 sync 实测四件全重装（aginx 走 gh-proxy 重试落
1f8134d0…）。**aginx-carrier v0.2.1 资产漂移**：GitHub release 重切于
2026-09-02T16:41Z，资产 sha=c0d490d3…（API digest 佐证），manifest 旧 pin
bbac1017=本地构建 → sync sha mismatch。manifest 重 pin c0d490d3 + 重签。
**结构性发现：state tar 只带 wifi.conf+/etc/aginx，不带
/etc/agpkg.manifest——换机后 manifest 回退烤内版本**（第三次换机又翻回
bbac pin 即此）；烤 #9 折入新 manifest。（更正 2026-09-03：本条原记
"provision pkg fail 后仍写 done ok（失败不重试）"——归因错了。`done ok`
是 net-bringup 的收尾行，不是 provision 写的；provision 无记忆、每靴
重跑 sync，真正的洞是"一次性步骤没有标记纪律"（M27 立）+ 单元在空
/var/bin 窗口重启耗尽后没人扶（M27 的 resync 后 restart 补）。）AP DNS 阵发失联
（github/gh-proxy 双双 Try again，后自愈）；codex 233MB 在换机后完整重下
成功过一次。/root/bin 垫片用完即删。设备终态：烤 #8 fs（第三次换机后）+
推送覆盖 agupd(9434a35d)/agpkg(d7d57e9d)/manifest(c0d490d3 pin)+sig；
stamps×4；/var/bin 四件经修复后 sync 重装。

### M27 — provision 三层 + ag-done 标记纪律（2026-09-03）

**件**：`crates/agdone`（Rust，agio 信封）→ /usr/bin/agdone + sh shim
/usr/bin/ag-done（ag:group=sys）；标记态 /var/lib/ag/done/<name>（内容
= mark 时刻 epoch 秒，仅人读，存在即真值）。语义：check rc 0=已标记 /
3=未标记（查询合法答案，非失败）/ 1=io；坏标记（路径被目录占位）当
未标记；名字限 [A-Za-z0-9._-]（遍历类 usage rc 2）；--json 出 ok:true
信封但 rc 仍 3（脚本按 rc 分支、JSON 按 data 分支）。ensure 刻意区别于
步骤惯用法——ensure 先盖戳再干活，步骤失败标记已在，恰是要堵的洞；
步骤的正确姿势 `agdone check x || { step && agdone mark x; }`。

**实测（adb 直跑 + ag done 路由）**：未标记 rc3 → mark rc0 → check rc0
→ JSON 信封 {"ok":true,"data":{"marked":true},"meta":{"at":…}} →
list → reset 幂等 rc0；目录占位标记 rc3 ✓；`mark ../escape` rc2 ✓；
`ag commands --check` **18 OK**（ag-done 入册）。

**provision v2 三层**（/etc/init.d/provision 重写，md5 推送验证）：
- **seed**：mkdir /var/lib/ag/done + agpkg 三目录（烤树也补了——busybox
  tar 缺成员退 1 的防线，M26）。
- **resync**：agpkg sync 每靴跑，输出改落 /var/tmp/agpkg-sync.log
  （boot.state 只记 pkg ok/fail 一行）；app-registry 无条件随后（原来
  只在 sync 成功才跑）；**本靴装过的包解析 `^agpkg: installed (<name>)`
  后 `agctl restart <name>`**——针对 2026-09-02 观察（空 /var/bin 窗口
  aginx 5 次重启耗尽 → failed，人工 restart 才活）。
- **finalize**：agdone-gated 一次性步骤位就绪，首个租户 M28（python+pip）。

**双启验收**：首启 boot.state `pkg run → pkg ok → done ok`（done ok 是
net-bringup 的行——上面那条更正的活证据）；sync log 四件全 "up to
date"（M26 的 stamp+binary 双闸在位，零重下）；/var/lib/ag/done 由 seed
本靴建（mtime 18:29）；agctl 四单元 ready 无人工干预；agent me +
clone-creator running。**第二启 77s 到位（与首启同速）**、四件仍全 up
to date、单元全 ready、标记区空——幂等快道 ✓。设备终态：烤 #8 fs +
推送 agdone/ag-done shim/provision v2；slot _b；无悬挂 swap 头。

### M28 — python3 core tier（musl CPython 四件套树包 + finalize 首租户）（2026-09-03）

**件**：agpkg 长出**树包**（四件套扩展）：tar 内 `files/` 整树流式落
`/var/lib/agpkg/pkgfiles/<name>/`（保留 tar mode + 树内软链；拒绝绝对
链目标；单成员仍限 256MiB；整树不进内存——CPython 树 45MB 起步），
pkg.toml `exec = "bin/python3"` 声明脸位 → `/var/bin/python3` 是指向树内
的**相对软链**（CPython 走 /proc/self/exe 定位 stdlib，无需 PYTHONHOME；
软链锚点是链接所在目录——首版按链接全路径算 `..` 数错了一层，host
装真工件时抓到）。exec 与平铺 bin/<name> 二选一，都有即拒；错误码
pkg_face_twice/pkg_exec/pkg_unsafe_link。sync 的 up-to-date 闸（M26）
对树包自然成立：树被抹 → 脸悬空 → exists()=false → 重下。

**工件**：astral python-build-standalone `cpython-3.12.14+20260901`
aarch64-unknown-linux-musl install_only_**stripped**（上游 sha256
a0ad6f01…），裁 86M→45M：去 tkinter/tcl/tk 链（~9.7M）、include/、
share/、libpython3.12.so.1.0（exe 静态链 libpython，lib-dynload NEEDED
只有 libc.so——22M 白送）。**关键坑：PBS musl 构建是动态链
`/lib/ld-musl-aarch64.so.1`**（内核 ELF 解释器路径，写死）——本机全家
静态、无 musl 加载器。解法：树内带 Alpine 3.20 musl 1.2.5 的
ld-musl-aarch64.so.1（723KB，musl 无符号版本化，1.2.5 跑 Clang 22 新构
无缺符号——实测），finalize 链到 /lib。镜像 yinnho/aginxos
python3-v3.12.14（GitHub API digest=sha256:23923ffe… 与本地一致）；
manifest 加 core 行 + 重签。

**provision finalize 首租户**（agdone 纪律实战）：python3 在 → 链
loader → `python3 -c 'import sys,ssl,sqlite3,socket,json'` + `pip
--version` 双验 → 过才 `ag done mark python-finalize`；sync 失败/离线
→ python3 缺 → 不落标，下靴重试。_system SKILL（"保证 python3，不保
证 node"）每靴覆写种子。

**实测**：设备 sync 经 gh-proxy 兜底装成（直连 github TLS 又掉，
retry 兜住——M10 的韧性路径）；手动链 loader 后 `python3 -V` =
3.12.14、ssl/sqlite3/socket/json 全过、pip 26.2.1 可用、
`pip install six` 装成 import 1.17.0（musllinux wheel 链路通）。
**删标 + 删 /lib loader + 重启**：finalize 经 provision 真跑——
boot.state 出 `py ok 3.12.14 (main, Sep 1 2026…`、loader 重链、标重写、
python3 可用 ✓。带标靴 finalize 静默跳过（纪律目的）；81s 靴无回归；
`ag pkg list` 出 python3（skill 标）；`ag commands --check` 18 OK。
设备终态：烤 #8 fs + 推送 agpkg(90f3f5d8)/provision/manifest(c0d490d3
+python3 23923ffe)+sig；pkgfiles/python3 54.9M；标 python-finalize；
slot _b。离线靴：python3 运行路径无网依赖（标在则 finalize 都不跑）。

### M29 — 测试纪律成形（check.sh + accept 套件）（2026-09-03）

骨架收口件：host 门禁 + 设备验收单从"每次手敲"变成可重跑的脚本。
不改 boot 行为，全部是 dev 侧工具；设备观察 = 套件本身在机跑绿。

**host 侧 `scripts/check.sh`**：
- cargo test 主机集 = ag/agio/agpkg/agdone/agdl/agsign/aterm/
  wifi-wizard/aginxos-probe/aginxos-agent（10 个，40 用例绿）。Linux 上
  跑 --workspace；macOS 上 agupd/agsvc/aginxos-init 编不过（prctl/
  SO_PEERCRED/ioctl c_ulong Linux-only libc 面，agupd 自 M14 如此），
  显式列出主机集。
- `ag commands --check` 对 scratch shim 树：cp boot/rootfs/usr/bin +
  chmod 755（git 存 644，路由器只认可执行）+ 15 个 ag:exec 目标打桩
  （C 工具只在烤盘里编译，--check 只查存在性）→ 18 commands OK，
  与机上 `ag commands --check` 同数。
- `check.sh lint` 跳过 cargo 半场（改 shim 后秒级反馈）。

**testkit crate（fixture 统一）**：tmp(tag)/write_exec/env_lock 三件；
约定两级——优先注入路径（agpkg::Paths）或子进程 `.env()`（ag 路由
测试，天然并行），只有代码按设计读全局 env（agdone::dir）才用
env_lock 串行。ag/agdone/agpkg 四处测试已迁。

**设备 accept 套件 `scripts/accept/`**：lib.sh 全 adb 调用钉实验机串
号 aginxosredfin（日常机永不沾）；drv 用尾行 `__RC=$?` 抓设备退出码
（adb 历史上不透传），滤 bionic linker 噪声行；expect_py 用 host
python3 验 D1 信封（取最后一条可解析 JSON 行——carrier 面允许人读
提示行在前）。六套件实测全绿：smoke 11、m24 14、m25 6、m26 8、
m27 13、m28 11 = 63 断言。m24 头条照旧是 Omarchy 事故类：
`ag sys reboot --help` 拦截 + uptime 单调证明 reboot2 没跑。

**套件自举期抓到的真缺陷**（dev 侧，非设备回归）：
1. lib.sh drv 的输出管道在 set -o pipefail 下，设备命令零 stdout 时
   末级 grep -v 空输出退 1 → set -e 静默杀脚本（首跑 smoke 前 11 行
   正常是因为 libc 噪声行恰好撑着管道非空）。`|| true` 修复。
2. 测试断言错两处（设备行为本来对）：agdone --json 的 rc=3 只在
   未标记时（我在已标记态断言 rc3）；SKILL.md 只有四件套 tar 才有
   （裸二进制包没有——aginxbrowser 断言改 python3）；cron list 无
   --json 旗标（信封是默认输出形状）。

设备终态：无变化（套件只读 + accept-m27 草稿标已自清）；烤 #8 fs、
slot _b、python-finalize 带标不变。

### M30 — 化身注册 + agent install --file + dup 上机（首个四件套包）（2026-09-03）

三件：dup CLI 搬进 aginx-carrier workspace（M30a，carrier 仓提交
9f05566）、`agent install --file <tar>` + `--dry-run` 权限预览（M30b，
carrier 仓提交 fd8a712）、dup 作为第一个四件套 agpkg 包上机 + 全链
设备验收（M30c，本条）。

**构建**：cargo zigbuild aarch64-unknown-linux-musl 双产物——
aginx-carrier 27.6MB、dup 4.6MB，均 static stripped。四件套 tar
（bin/dup + pkg.toml + SKILL.md）sha256
652f9d58…c32a82；**首包被 agpkg 拒**：flat bin/dup 成员 + pkg.toml
`exec` 同时存在 = pkg_face_twice（exec 只用于 files/ 树 symlink 面，
python3 形态）——去 exec 行后装成。macOS 打 tar 必 COPYFILE_DISABLE=1
（AppleDouble ._ 成员；carrier 侧 tar_source 也已卫生跳过）。

**上机实测**（均 adb 推 + md5 验证 + chmod 755）：
- `agpkg install dup /data/local/tmp/dup-4pc.tar <sha>` rc 0 →
  /var/bin/dup 755、`dup 0.2.0`、SKILL.md 落 /var/lib/agpkg/skills/dup/、
  无 agsvc 单元（pkg.toml 无 [service]，预期）。
- dup 离线本地环全通：dummy env（OPENCARRIER_URL/KEY）`dup init`
  落 .dup/state.json → 加文件 `dup commit` 出短 id、`dup log` 带
  消息、`dup status` 干净。网络面（pull/push）未测——duphub auth 等
  M36 sidecar。
- 新 aginx-carrier（HOME=/home 面）`ag agent install accept-m30
  --file clone.tar --dry-run` → 预检通过 + flow/shell 权限预览 +
  未安装（workspace 确实没落）；真装 → 已安装 + workspace 全树
  （AGENT.json/agent.toml/flows/history/knowledge/logs/profile.md/
  sessions）；坏 tar（flow 缺 description）--dry-run rc 1 报
  「预检未通过」；remove 干净。
- ag-agent shim 元数据升 v1 全景（--file/--dry-run/remote）；
  `ag agent --help` 拦截出 usage、`ag commands --check` 18 OK。

**真缺陷（M30c 实测抓到）**：adb shell 的 HOME=/（adbd 原生，不读
/etc/aginx/env——那只喂 agsvc 单元）→ CLI 面 `ag agent install` 把
注册表/workspace 劈到 **/.aginx** 开了第二真源（daemon 在
/home/.aginx）。已清：HOME=/ 卸载 + rm -rf /.aginx，重装落
/home/.aginx/carrier/workspaces/。**修复**：accept lib drv() 钉
HOME=/home（aterm 面本就设 /home 不受影响）；裸 adb 手工操作须自觉
export HOME=/home。

**验收**：新增 scripts/accept/m30-agent.sh 30 断言全绿（dup 产物 4 +
离线环 9 + install --file 12 + 清理 5）；六存量套件回归无恙（63 断
言）；host `check.sh lint` 18 OK。套件自举期抓到 dev 侧错一处：
expect_py 参数序（name, expr）我写反——bash -x 才看见 expr 被展开成
名字。

设备终态：dup 四件套装成（/var/bin/dup，v0 手装、不在 manifest）；
/var/bin/aginx-carrier = 本地 v0.2.0-dev 构建（27.6MB，daemon pid 1254
仍执旧 inode 至下次重启；manifest 仍钉 v0.2.1 release——下次
provision sync 或换机自愈会拉回，**manifest 行 + release 资产待
「推送」后补**）；/usr/bin/ag-agent shim 已更新（烤盘同款，重烤折叠）。
注册表终态 = clone-creator + me（accept-m30 已卸），/.aginx 已清。

## 2026-09-03 — M31: D3 批1 — browser/web 工具外置成 agb 包 + runtime 桥（实测）

### M31c — agb 四件套上机 + ag-browser/ag-web 面 + 桥全回路（2026-09-03）

Host 侧（M31a/M31b，见 aginx-carrier 仓 363c24f / de89073）：agb crate
（browser/search/fetch 实现 verbatim 搬家，单真源）；runtime 三模块
（browser.rs/web_search.rs/web_fetch.rs + toolset.rs）删，换 agb_bridge
（12 个 ToolDefinition 逐字节保留，execute spawn `agb tool <name>`
stdin-JSON/stdout-D1-信封）；tool_search 退役（宪法性替代 `ag commands`）。
carrier workspace 全量 cargo test 1451 绿 + clippy -D warnings 0 + 两条
工具面金样本重录后绿。

**上机产物**：
- `agpkg install agb <tar> <sha256>`（HOME=/home）rc 0 → /var/bin/agb
  755（3,337,216 B musl static，md5 与 host 构建一致）+
  /var/lib/agpkg/skills/agb/SKILL.md。`agb 0.2.0`；机读面
  `agb tool browser_close` → `{"data":"…stateless…","ok":true}` rc 0；
  带 api_key 的 URL → taint 信封 `"ok":false` rc 1（干净信封非崩溃）；
  未知工具名 → `"ok":false` rc 1。
- 新 aginx-carrier 27,531,832 B 入 /var/bin（staging+mv，重启后 daemon
  PATH=/sbin:/bin:/usr/sbin:/usr/bin:/var/bin 含 /var/bin——桥 spawn
  `agb` 可解析）。
- /usr/bin/ag-browser + ag-web shim（group=web，烤盘同款）；设备
  /etc/ag/groups.desc 追加 `web=` 行。`ag commands --check` 20 OK
  （18+2）；菜单出 web 组两命令；`ag browser --help` 拦截目标未执行
  （无信封/Title 痕迹）。
- 真网络面：`ag web fetch https://example.com` HTTP 200 + EXTCONTENT
  包裹；`ag browser navigate https://example.com` 引擎回
  Title: Example Domain；`ag web search rust` 聚合真结果
  （rust-lang.org / arxiv）。

**桥全回路（M31b 的 bring-up 证据）**：直驱 ACP 桥（stdin ndjson：
initialize → session/new → session/prompt）对 clone `me` 提令
「调 web_fetch 抓 example.com，贴结果首行，输出 BRIDGE_OK」→ 13 s 内
session/update 回全文 + resp `stopReason: end_turn`，回复含真实正文
首行（# Example Domain）+ BRIDGE_OK。session jsonl 里 tool_use
web_fetch{url,method:GET} → tool_result HTTP 200 + EXTCONTENT——即
LLM → agb_bridge → spawn agb → 信封解包 → 结果回填全链在设备上走通
（runtime 内已无 web_fetch 实现，唯一执行体是 agb CLI）。同轮观察：
hub fallback flow 的 `tools:` 白名单不含 browser_*（agent 如实拒调
browser_close、不编造结果）——flow 冻结语义符合设计，非桥缺陷。

**顺手修的设备问题**：/etc/aginx/env 只剩 HOME=/home——AGINXBRAIN_API_KEY
在历次重烤后丢了（rootfs 烤盘 /etc 不含 key），agent LLM 轮缺 key。已从
Mac ~/.aginx/carrier/.env 取回补进 /etc/aginx/env（0600）+ 设备
/home/.aginx/carrier/.env（0600，dotenv 面：adb 起的 acp/agb 进程靠它）；
AGINXBROWSER_URL=http://127.0.0.1:8089 同入 .env（agb search 需要它，
browser_* 默认值即可）。重启后 daemon environ 验证含 key。

**网络面观察（Legrand AP，非 M31 缺陷但影响验收稳定性）**：
- 直连 HTTPS 偶发 `error sending request`（同命令 3 试 2 成）；设备无
  v6 默认路由但 DNS 回 AAAA（example.com 双栈）；agb 每调用新进程无
  DNS 缓存，抖动放大。套件对策：网络面断言带 drv_net 重试（3 次）。
- 引擎聚合搜索曾整排 engine "transient error"（冷 DNS），同 host python
  v4 直连却通；数分钟后自愈出真结果——按瞬态记录，未改引擎。
- net-watch 在本轮 boot 起 segmentation fault 循环（/var/log/agsvc/
  net-watch.log 连续 Segmentation fault，agctl 仍显示 ready pid 在）；
  relay 日志伴 DNS "Try again" 错误。**M20b 自愈链实际失效中，待查**
  （非 M31 引入：M30c 会话同图未记，需单独立案）。
- agb fetch 输出接 `head` 会 broken-pipe panic（Rust 默认忽略 SIGPIPE，
  writeln! 报错即 panic）——CLI 人面小疵，桥路径不受影响（stdout 全量
  消费）；carrier 侧待修。

**验收**：新增 scripts/accept/m31-agb.sh 30 断言全绿（产物 4 + 信封面 8 +
路由面 12 + 组表/门禁 4 + 清理 2，网络项 drv_net 重试）；七存量套件回归
无恙（smoke 11 + m24 14 + m25 6 + m26 8 + m27 13 + m28 11 + m30 30）。

设备终态：AginxOS boot、slot a、adb aginxosredfin；agb 四件套装成
（v0.2.0 手装，manifest 行 + release 资产待「推送」后补）；/var/bin/
aginx-carrier = M31 本地构建（27,531,832 B）；/usr/bin/ag-browser、
ag-web 已推（重烤 #9 折叠）；/etc/aginx/env + /home/.aginx/carrier/.env
含 brain key 与 AGINXBROWSER_URL（均 0600）；net-watch 崩溃循环待立案。

## 2026-09-03 — net-watch 自愈失效根因：busybox awk 段错误 → 每 45 s 重连健康网络（已修）

M31c 收尾立案（#112）当日结案。表象：本轮 boot 起 /var/log/agsvc/
net-watch.log 连续 699+ 行 Segmentation fault；同时段直连 HTTPS 偶发
`error sending request`（同命令 3 试 2 成）、relay 日志 DNS "Try again"、
aginxbrowser 聚合搜索整排 engine transient error——网络"抖动"被当成
Legrand AP 的锅写进了 M31 观察记录。

**根因（逐层剥）**：`busybox awk` 在本机**无条件段错误**（rc=139，连
`awk 'BEGIN{print 1}'` 都死）——案底早在 net-bringup（bringup 三脚本
都有 sed/set-- 注释规避），但 M20b 的 net-watch/net-rejoin 漏带进 awk：

- net-watch:52 `gw=$(ip route | awk …)` → awk 死、gw 恒空 →
  `[ -n "$gw" ]` 恒假 → fail 计数每轮 +1 → **每 ~45 s 对健康网络
  net-rejoin 一次**（wlan0 闪断 = 全部"偶发"网络失败的源头）；
  agsvc 单元日志里的 Segmentation fault 行 = ash 报子进程 SIGSEGV
  （子进程自身输出被重定向吞掉，报文落在单元 stderr）。
- net-rejoin:39 lease ok 消息里的 awk 同死（纯外观：日志显示
  `lease ok ()` 空 IP）。
- M20b 验收当年只测了"人为断链 → 自愈"路径，gw=none 让 fail 计数
  照样走满——**断链用例恰好掩盖了健康态误判**；健康态 45 s 一炸的
  症状一直到 M31c 长验收会话才显形。

**修复**：两处 awk → sed（烤盘 + 设备同步推，md5 一致）：
- `gw=$(ip route 2>/dev/null | sed -n 's/^default via \([0-9.][0-9.]*\) .*/\1/p' | head -n 1)`
- lease IP 同款 sed 解析（`^ *inet \([0-9.]*\)/`）。

**实测**：sed 版在设备上解析 gw=192.168.0.1 / ip=192.168.0.166；
`agctl restart net-watch` 后 55 s+ 无 probe fail 日志（健康分支静默
设计）、单元日志停止增长（710 行封口）、前台直跑新版无 Segmentation
fault。烤盘已修（重烤 #9 折叠）；**教训入册：本机 busybox 一律禁 awk，
解析用 sed 或 set --，新脚本评审过一眼**。

顺带修正 M31 记录的归因：当日"Legrand AP 直连偶发失败"至少大部分是
net-watch 自炸，非 AP（AP 掐长传输的旧案仍成立——grok 172 MB 收据
在前）。m31-agb.sh 的 drv_net 重试保留：多一层网络韧性无害。

补记（同日）：agb SIGPIPE 修复上机——carrier 71bb9b6 在 main 恢复
SIGPIPE 默认处置；musl 重建（3,327,680 B）推 /var/bin/agb（md5 一致），
设备实测 `agb fetch … | head` 干净退出无 panic；out/agb 四件套 tar 重出
（sha256 c960cab3…，待「推送」时用）。信封面/版本回归正常。

## 2026-09-03 — M32: D3 批2 — 文件面工具外置成 agf 包 + runtime 桥（实测）

### M32c — agf 四件套上机 + ag-file 面 + 桥全回路（2026-09-03）

Host 侧（M32a/M32b，见 aginx-carrier 仓 e32bbe8 / 52acb1e）：agf crate
（file_read/file_write/file_list/file_convert 从 filesystem.rs、
image_analyze + 魔数/尺寸 helpers 从 media.rs verbatim 搬家，单真源）；
runtime filesystem.rs 删，换 agf_bridge（5 个 ToolDefinition 逐字节保留，
execute spawn `agf tool <name>` stdin-JSON/stdout-D1-信封）；桥经保留键
`_ctx` 注入执行身份与预解析绝对路径——用户数据路由/沙箱/taint 权限留在
kernel 侧（§9 随末模块退役）；截断留在 tool_meta，信封行为零漂移。
carrier workspace 全量 cargo test 绿（runtime 547，agf_bridge 5/5，
agf 19/19）+ clippy -D warnings 0。

**四件套安装事故（收据，已修）**：第一次 `agpkg install agf` 用的 tar 是
`tar -czf` 出的**gzip 压缩档**——gzip 头把 257 偏移的 ustar 魔数盖掉，
is_tar 嗅探失败 → 走 v0 平面二进制路径 → **安装报成功**但 /var/bin/agf
是原始 gzip 字节（magic 1F8B）、skills/agf/ 缺失。agb/dup 当年装得成是
因为它们的 tar 是未压缩的。修复两步：
- 重打未压缩 ustar（`tar --format=ustar -cf`，sha256 eed372ac…）→
  重装即"installed agf bundle"，/var/bin/agf = ELF（1,171,304 B musl
  static，md5 1317cb57… 与 host 构建一致）+ SKILL.md 落 skill 宇宙。
- **agpkg 加 gzip 硬闸**：install_file 开头嗅 `\x1f\x8b` → `pkg_gzip`
  干净报错 + hint「repack without -z: tar --format=ustar -cf …」。设备
  实测：假 gzip 档被拒、/var/bin 与 skills 无残留；同二进制重装好
  agf 包照常（happy path 无回归）。烤盘 /usr/bin/agpkg 已同步（重烤
  #9 折叠）。教训入册：**agpkg 四件套 tar 一律未压缩 ustar**。

**上机产物**：
- /var/bin/aginx-carrier = M32 本地构建 27,494,536 B（md5 a9329815…，
  staging+mv，agctl restart 后 ready）；/usr/bin/ag-file shim（group=files，
  烤盘同款）+ 设备 /etc/ag/groups.desc 追加 `files=` 行。`ag commands
  --check` 21 OK（20+1）；`ag file --help` 拦截目标未执行。
- 机读面：`agf tool file_read /etc/hostname` → `{"data":"aginxos\n",
  "ok":true}`；未知工具 → `"ok":false` rc 1；PNG 魔数 → 干净信封报错
  并指路 image_analyze；image_analyze 回 format/size/base64 信封。
- 人面：write→read 回路、`ls 文件` 纠偏提示改用 file_read（防工具循环）、
  `ag file read` 路由直通均实测通过。

**桥全回路（M32b 的 bring-up 证据）**：直驱 ACP 桥（stdin ndjson：
initialize → session/new → session/prompt）对 clone `me` 提令「用
file_write 把 output/m32-probe.md 写成 AGF_BRIDGE_WROTE_THIS，再用
file_read 读回贴原文，输出 BRIDGE_OK」→ 14.9 s end_turn；回复含
AGF_BRIDGE_WROTE_THIS 原文 + BRIDGE_OK。session jsonl 里 tool_use
file_write → tool_result、tool_use file_read → tool_result 各一；文件
实际落在用户数据路由位
`/home/.aginx/carrier/workspaces/me/senders/acp:<sid>/output/m32-probe.md`
——即 LLM → agf_bridge → spawn agf → 信封解包 → 结果回填全链在设备上
走通（runtime 内已无文件工具实现，唯一执行体是 agf CLI）。

**验收**：新增 scripts/accept/m32-file.sh 35 断言全绿（产物 5 + 信封面
8 + 人面 6 + 路由面 8 + 组表/门禁 4 + 清理 2 + scratch 2）；八存量套件
回归无恙（smoke 11 + m24 14 + m25 6 + m26 8 + m27 13 + m28 11 + m30 30 +
m31 30）；check.sh host gate 绿。

设备终态：AginxOS boot、slot a、adb aginxosredfin；agf 四件套装成
（v0.2.0 手装，manifest 行 + release 资产待「推送」后补）；/var/bin/
aginx-carrier = M32 本地构建；/usr/bin/agpkg = gzip 闸版（md5 d3a652b0…）；
/usr/bin/ag-file 已推、groups.desc 含 files= 行（均待重烤 #9 折叠）。
M32 收口：media 余下 brain 耦合工具与 misc 面并入 M33（内核耦合批）。

## 2026-09-03 — rootfs 重烤 #9：M30–M32 折叠 + gzip 闸 agpkg + net-watch sed 修（实测）

配方沿 #8：SIZE=1g 烤（sha 7e398d29…，105 MB 内容/1 GiB fs，烤内
`ag commands --check` 门禁过）；boot.img=e2ce2f17（复用）、vendor_boot=
d80b8098（boot/out/vendor_boot-test.img，蹦床版）；rootfs pre_staged——
host `cat img | adb shell dd bs=4096 seek=2097153` 流灌 userdata
（=SWAP_OFF 8 GiB+头 4096）；manifest+sig（agsign）推 /tmp/agupd；
`agupd apply --no-reboot` 全绿（boot_a/vendor_boot_a sha 过、state tar
59,748,864 B 落 64 GiB、pre_staged 体原位哈希过、swap committed、
old fs 1,020,702,720 B）；reboot2 后 ~110 s 起机 `aginxos c793bad`。

折叠内容：M30 ag-agent 真 shim、M31 ag-browser/ag-web + web= 组行、
M32 ag-file + files= 组行、gzip 闸版 /usr/bin/agpkg（md5 d3a652b0…）、
net-watch/net-rejoin awk→sed 修、manifest v0.2.1 pin（c0d490d3）。

**首启三观察**：
1. `pkg fail`（boot.state）——直连 github timeout（AP 冷网络老图景），
   本boot未自愈；手工 `agpkg sync` 即全绿（python3 47 MB + codex 233 MB
   走 gh-proxy 落地，aginx/aginx-carrier/aginxbrowser 同步重装——
   M26 修复2 的 stamp+binary 存在性判据工作正常）。
2. **aginx 单元 5 连败**：起机时 /var/bin/aginx-carrier 还没被 sync 装
   回（swap 清空 /var/bin），网关"no agents configured"退出；sync 完成后
   restart 即回岗。结构性：单元启动早于网络自愈装包，属首启窗口竞态，
   第二靴（包已在位）不再现——记录不立案。
3. **python-finalize 标记撒谎**：标记在 /var/lib（state tar，swap 存活），
   而它的工件 /lib/ld-musl-aarch64.so.1 软链在 state tar 外——swap 后
   python3 死于缺 musl interpreter（报错是迷惑性的 "No such file or
   directory"）。手工 reset+重跑 finalize 即愈；**修**：provision 守卫
   加 `[ ! -e /lib/ld-musl-aarch64.so.1 ]` 重验工件（信工件不信标记，
   烤盘+设备同步推，m28 套件 11/11 恢复）。

**swap 损失清单（全部手工补回）**：/var/bin 全清（manifest 件 sync 自愈；
agf/agb/dup 本地四件套重装——tar+sha 直推即过 gzip 闸）；/data/local/tmp
清空；M32 carrier 构建（a9329815…）重推。状态面完好：wifi.conf、
/etc/aginx/env（brain key）、/home 工作区+会话、/var/lib stamps×8、
groups.desc（web=+files= 都在——state-restore 盖烤盘同内容）。

**第二靴全绿**：touch/camera/battery/modem/audio/wifi/dhcp/internet/
time/pkg/done 全 ok，四单元首试 ready，carrier=M32 构建 md5 不变，
九套件回归 11+14+6+8+13+11+30+30+35=158 断言 0 败。

设备终态：bake #9（c793bad）slot _a active；/var/bin 八件（aginx/
aginx-carrier=M32 构建/aginxbrowser/codex/python3 + agf/agb/dup 手装）；
python-finalize 重验守卫在位（烤盘已改，下一烤折叠）。

## 2026-09-03 — M33 D3 批3：内核耦合工具外置成 carrier CLI 面（实测）

M33a（`aginx-carrier tool/sys` 机读+人面 CLI，内核耦合 13 工具单真源）+
M33b（runtime misc/agent_mgmt/scheduling 三模块退役 → carrier_bridge，
spawn `aginx-carrier tool <name>`，`_ctx` 身份+递归深度跨进程续传）上机。
部署：musl 构建（sha 1a5b727a16758c07…）→ /var/bin/aginx-carrier，
`agctl restart aginx-carrier` 换血（pid 5052）；四 shim 推 /usr/bin
（ag-cron/ag-agent 扩子命令 + 新 ag-sys-time/ag-sys-location）。

**adb HOME 复发**：手跑 CLI 时 adb shell HOME=/，kernel boot 在 /.aginx
刻出空注册表 → "Agent not found: me"；HOME=/home（lib.sh 已钉）即对上
守护的 /home/.aginx/carrier。M30 发现的手跑面复发，机制记录。

**机读面收据**：`tool system_time` 信封 ok rc=0；`tool nope_such` →
tool_unknown 信封 rc=1；`tool cron_create` 缺 `_ctx` → "Agent ID
required for cron_create" rc=1（干净错误面，kernel boot 路径在设备走通）。

**cron DB-as-bus 收条**（CLI 落任务 → 守护收养 → 发射）：23:40:05
`tool cron_create`（caller me，one_shot at in_secs=45，system_event）
→ job_id 5108d909…；`tool cron_list`（另一进程、独立 boot）见
next_run 23:40:50；守护日志 23:40:20 `Cron reconciled from database
added=1`（15s tick）；23:41:12 `cron list` 已空——到点发射+one_shot
自动删除。发射本身无 INFO 行（低于 info 级）。

**schedule kv 回路**：create（"every 5 minutes"→cron */5 解析）→ list
见条目 → delete → "No scheduled tasks"。**agent 三件套扫**：极简
manifest（module=builtin:chat）`tool agent_spawn` → m33-probe
（8903f2a2…）入册 → `tool agent_list` 见 3 agents → `tool
agent_kill` → 回 2。

**sys location 修**：上机 403——ip-api.com 免费档只开明文 HTTP
（HTTPS 全局 403，host 复核同象；上游 misc.rs 遗传 bug）。单真源侧
https→http 后重推：Nanjing/Jiangsu 信封 rc=0。

**LLM 回路 agent_send（桥全链）**：ACP ask 模式
（`echo 提示 | aginx-carrier acp --clone me`）驱动。三层发现：
1. me 的 hub flow 白名单只有 contact_prompt（无 agent_send）——首轮
   LLM 诚实说明并改道 contact_prompt（contact 面在设备亦通）；
2. 白名单真源在 flows/hub/flow.md frontmatter `tools:`（非
   agent.toml [capabilities].tools——后者改了无效）；
3. 静态表 PermissionLevel::for_tool(agent_send)=Execute > me 的
   max_tool_level=write → flow 声明也被 level 闸拦（警告文案是
   "工具目录中不存在"，实为权限不足——文案误导记此）。
临时 max_tool_level=execute + flow 加 agent_send + `agent restart me`
（restart 会 reload agent.toml）后：me 的 LLM 调 `agent_send` →
CarrierBridge spawn `aginx-carrier tool agent_send` 子进程 → 子进程
kernel boot → clone-creator 真轮（第二次 LLM 调用）→ 原话回传
"收到，桥上机通信正常——我是 Clone Creator…"，22 s/2 轮 success。
runtime 内已无 agent_send 实现（agent_mgmt.rs 删），唯一执行体是
CLI 子进程——跨进程桥+嵌套轮全链在设备走通。证毕还原
agent.toml/flow.md（md5 复核）+ restart me，无残留。

**网络**：Legrand AP 对 LLM 长流敏感（多次 Request timed out，
wlan0 NO-CARRIER 或关联在但出不去），`net-rejoin` 原地恢复即续——
M20b 图景，本轮 3 次 rejoin 后证明落袋。

**验收**：scripts/accept/m33-kernel.sh 35 断言全绿（shim/路由面 6 +
机读信封 7 + schedule 回路 8 + agent 面 2 + 人面新语法闸 1 + 组表/
门禁 4 + 清理 2 + scratch 5）；smoke 11/11 回归无恙。

设备终态：bake #9（c793bad）slot a；/var/bin/aginx-carrier = M33
构建（1a5b727a…）；守护 pid 5052 跑同版；四 M33 shim 在 /usr/bin
（adb 推，待重烤 #10 折叠）；注册表 me/clone-creator 无探针残留，
schedule/cron 表清空。M33 收口：D3 三批全落（批1 agb / 批2 agf /
批3 carrier 面），runtime 内核耦合工具零残留实现。

## 2026-09-03 — M34: api_tools → `aginx-carrier api` 命令层（实测）

**改动**：api_tools 执行链（resolve 链/ctx 注入/HMAC 签一发一/extract
tiers/cron 落库/注册写盘）自 runtime 整体搬进 `aginx-carrier api`
（子命令 call/raw/list/register/cron）——单真源在 CLI；runtime 侧只剩
ToolDefinition 广播 + spawn 桥（toml 面 = 全局 + 本化身工作区，per-agent
同名覆盖）+ DYNAMIC_TOOLS 进程内注册表。原 `api_tools/cron.rs` 的
`register_cron_tools` 从未接线（死码），删；活的节拍在 daemon start.rs：
30s 一跳 spawn `api cron`，空载门先扫 toml 有无 `[tool.cron]`——零配置
设备不付 spawn 代价（功耗线 M23）。

**发现（硬伤，上机暴露）**：原 register.rs 的 `serialize_tool` 漏写
`[tool.cron]` 节——注册带调度的工具会静默丢 cron，注册后永不发射。上
机现象：`api cron --json` 在秒<30 窗口返回 `{"fired":[],"due_skipped":0}`
（工具根本不被视为 cron 工具）。M34a 的搬运转写了同款 bug（忠实移植了
上游缺陷）；注册面成为唯一写手后必须修：serialize 补 `[tool.cron]`
（schedule/save_to/table）+ 往返测试。修复重推后同一命令 fired 一发
入库。

**收据（2026-09-03，serial aginxosredfin）**：
- 新 brain key（ab_b320…）写入 /etc/aginx/env + agctl restart 后：
  acp ask 全轮 `is_error:false`，result「收到」，5.4 s/1 turn。
- 新二进制 /var/bin/aginx-carrier（md5 bdfe3672…，含 cron 修复），
  守护 pid 9361 起同版；ag-api shim + api 组进 /usr/bin、groups.desc。
- `api register --global`（ip_city，ip-api 明文 HTTP）→ `ag api list`
  人面/信封面各一条；机读 call（stdin `_ctx`）D1 信封 data =
  {city:"Nanjing", region:"Jiangsu"}；人面 --param 同源；raw 直通
  通（期间一次 Legrand AP 瞬断，重试即过——老毛病，非本次回归）。
- 同名重注册两次：api_tools.toml 恒 1 块（幂等）。
- daemon 30s 委托节拍：注册 `* * * * *` 的 ip_city_cron 后约 70 s，
  aginx-carrier.log 出 `api cron 委托一跳 tool="ip_city_cron" ok=true`
  （00:50:24），/home/.aginx/carrier/data/m34-cron.db 落行
  （python3 读：tool_name=ip_city_cron，raw_response 含 Nanjing）。
  零 [tool.cron] 时无 spawn（空载门，日志无跳）。
- LLM 回路过桥：acp ask「调用工具 ip_city 查出口城市」→ 2 turns、
  12.3 s、is_error:false、result「南京」（城市为工具事实，brain 无从
  知晓——证明 provider 桥 spawn CLI 执行链在轮内闭环）。
- 套件：m34-api.sh 31/31；smoke 11/11；m33 35/35（回归）；host
  check.sh 全绿 + 设备 `ag commands --check` 24 commands OK（+ag-api）。

设备终态：全局 api_tools.toml 仅存 ip_city（无 cron，守护节拍空载）；
m34-cron.db 与 /var/tmp 探针已清；bake #9（c793bad）slot a 未动，
ag-api shim 与 api 组行待重烤 #10 折叠。M34 收口：api_tools 执行、
写盘、cron 发射全部单源于 `aginx-carrier api`；runtime 侧零执行残留。

## 2026-09-03 — #87: pkgs.aginx.net 软件镜像上机（agpkg 包源迁自 GitHub）

**动机（实测）**：本网络（Legrand AP）对 GitHub 长传输必杀——bake #10
恢复期 aginxbrowser 走 GitHub 39MB/64MB 后静默（agdl 无读超时，挂在
sk_wait_data；kill 后换本地资产）；host 侧 curl 同样截断（847KB/64MB）。
grok 先前 172MB@27KB/s 拉了整晚。镜像为国内直连，一劳永逸。

**架设**：86quan（106.75.32.216，UCloud；用户加的 Cloudflare 解析）。
nginx vhost 443（LE 证书 CN=pkgs.aginx.net，YR2 链，2026-12-02 到期，
webroot 续期）；root=/data/pkgs.aginx.net，autoindex off，
octet-stream，immutable 缓存头，deny sync.sh/sync.log。布局
`<pkg>/<短版本>/<asset>`——与签名 manifest 的 URL 一字不差。

**回填脚本** /data/pkgs.aginx.net/sync.sh：从 GitHub releases 经
socks5h://127.0.0.1:8800 拉，sha256 闸（代理不信任，只信哈希），
5×重试。收据一：Azure blob 对连发大文件限流——200 状态码 + XML 错
误体（四文件同一错误 sha），重试环必须。收据二：yinnho/aginxos 的
release tag 带 pkg 前缀（aginxbrowser-v0.2.5/python3-v3.12.14/
codex-v0.151.0/grok-v1.0.12），aginx 与 aginx-carrier 才是裸 v-tag
——短 tag 全 404（404 页 sha 0019dfc4…），脚本 case 已按全 tag 映射。

**踩坑**：初填目录双层嵌套（pkg/全tag/全tag/asset，手铺目录残留）→
manifest URL 404（153B nginx 默认页），拉平后全通。sync.sh 幂等复跑
OK(skip)×6。设备侧新 DNS 记录间歇 NXDOMAIN——/etc/hosts 临时钉
106.75.32.216 pkgs.aginx.net（下次换机消失，属过渡）。

**收据（2026-09-03，serial aginxosredfin）**：六资产服务器侧 sha 全
对（共 ~524MB）；Mac + 服务器本环 curl TLS 校验过（TLSv1.3，LE 链）。
设备 `agpkg sync`：aginxbrowser 64,037,544 B + codex 233,773,456 B
自镜像 HTTP 200 安装、sha 过闸（当日早先同路径走 GitHub 是死路）；
aginx/aginx-carrier/python3 up to date。agb --help、codex 0.151.0
实跑验证。客户端零改动：只换 manifest URL（sha 不动）+ 重签 + 推
/etc/。观察：agpkg "up to date" 以 stamp 为准——设备 bdfe3672（M34
构建）≠ manifest 钉 c0d490d3，sync 未回滚。gh-proxy 保留为应急文档
路径。

## 2026-09-03 — rootfs 重烤 #10：M33/M34 折叠 + 换机收口（879afe8）

**镜像**：SIZE=1g，版本戳 `aginxos 879afe8 2026-09-03`，in-bake 门
`ag commands --check` 24 OK；boot.img 复用 e2ce2f17；vendor_boot
d80b8098（ROOTFS=1 trampoline）。换机走 host dd（seek=2097153，
userdata 8GiB+4096）+ agupd apply --no-reboot（注意：manifest 参数
必须在 --no-reboot 之前，否则报 unsupported url）+ /bin/reboot2
reboot，~2 分钟到 boot complete。

**发现（硬伤，新烤才暴露）**：fresh 镜像没有 /var/tmp——build-rootfs
的 mkdir 清单、rcS、aginxos-init 三处都不造它；旧文件系统有是当年
手工 mkdir 的遗产。后果：provision 的 `>$LOG` 重定向失败 → `pkg
fail` 且无日志可看（agdl 亦在 /var/tmp 下暂存）。修复：
build-rootfs.sh mkdir 清单补 var/tmp；在机 mkdir 补救。agdl 无读超
时（AP 掐流后永挂 sk_wait_data）——已知，另案。

**换机后恢复（USB）**：/var/bin 与 /data/local/tmp 被 swap 清空——
按序重推 python3/agf/agb/dup 四件套、ag-done python-finalize、M34
carrier bdfe3672 + 单元重启；/etc/aginx（新 brain key）经 state tar
存活，env.bak-key 清理。

**验收**：m33 35/35；m34 三次 29/2（ip-api 明文 HTTP 在本 AP 瞬断，
每次挂不同断言对——直探 6/6 + 0% loss，判网络非回归），镜像通后
31/31；smoke 先 10/11（pkg 行缺席）→ 镜像 sync + provision 后 11/11
（boot.state 尾部 `pkg ok` 落袋）。

设备终态：879afe8 slot a；/var/bin 八件齐（aginx/aginx-carrier
bdfe3672/aginxbrowser/codex/agb/agf/dup/python3→symlink）；守护单元
aginx/aginx-carrier/net-watch ready；/etc/hosts 含 pkgs.aginx.net 钉
（临时，传播稳后可撤）。重烤 #10 收口：M24–M34 骨架全部进 baked
镜像，设备不再依赖 adb 推的 /usr/bin shim。

## 2026-09-03 — M35: agmem 四件套上机 + runtime 记忆面桥（实测）

M35a/b/c（host 侧，见 aginx-carrier 仓）：agmem crate（kv/tree/knowledge/
flows 单真源，19 工具名契约定死；`agmem tool <name>` 机读面 D1 信封）+
runtime kv/memory/knowledge 三模块退役 → agmem_bridge（18 工具，spawn
`agmem tool`，`_ctx` 注入身份三元组/home_dir/workspace_root；apply_patch
与 session_summarize 留守 runtime——内核耦合面）。

### M35d — 四件套安装 + 桥全回路 + m35 套件

- **四件套**：`ag pkg install agmem /tmp/agmem-v0.2.0-4pc.tar <sha>`
  （uncompressed ustar 3,185,664 B）→ /var/bin/agmem（3,178,912 B，md5
  0703be45…）+ /var/lib/agpkg/skills/agmem/SKILL.md；`agmem --version`
  → agmem 0.2.0。
- **新 carrier 上机**：M35c 构建 27,634,040 B（md5 de63f583…，staging+mv
  +agctl restart 后 ready）。daemon 内已无 kv/memory/knowledge 实现，
  唯一执行体是 agmem CLI。
- **人面（真实 substrate）**：`agmem set/get/list` 直开守护同一个
  /home/.aginx/carrier/data/carrier.db（WAL 并发安全），kv 行落
  (me, default, local) 域。
- **机读面**：`agmem tool kv_get` → `{"ok":true,"data":…}`；未知工具
  ok:false rc 1；显式 null owner/user 走 (86bus,"","") 域——与人域互
  不可见（kv_store 表三行三种身份实证）。
- **坑（探针实捉）**：桥全回路首跑失败——db 路径推导出
  `/home/.aginx/carrier/.aginx/carrier/data/carrier.db`（`.aginx/
  carrier` 拼两遍）。根因：runtime ToolContext.home_dir = config.home_dir
  是 **carrier home**（agf 的 sender_data_dir 同语义），而 agmem 原推导
  把 `_ctx.home_dir` 当**用户主目录**。修：agmem `db_path_of` 分语义——
  `_ctx.home_dir`（carrier home）→ `{home}/data/carrier.db`；`$HOME`
  （用户主目录）→ `{HOME}/.aginx/carrier/data/carrier.db`；默认布局下
  同库。agmem 24/24 host 测试改判后全绿，重打包重装。
- **桥全回路（M35c 的 bring-up 证据）**：直驱 ACP 桥（stdin ndjson：
  initialize → session/new → session/prompt）对 clone `me` 提令「kv_set
  写 m35.probe=AGMEM_BRIDGE_WROTE_THIS，kv_get 读回贴原文，输出
  AGMEM_BRIDGE_OK」→ end_turn；会话 jsonl 含 kv_set + kv_get 各一次
  （tool_result 分别为 "Stored value" 与 `"AGMEM_BRIDGE_WROTE_THIS"`）；
  kv_store 表 (me, "", "acp:<sid>") 域 03:47:48 落值；CLI 按同三元组
  独立读回成功。首跑（修前）还实捉了缺包净错路径：agent 自报
  `agmem CLI not available` 并拒绝输出 OK（探针进程 PATH 不含 /var/bin
  ——守护单元无此问题）。
- **路由面**：/usr/bin/ag-mem shim（group=mem，烤盘同款）+ 设备
  /etc/ag/groups.desc 追加 `mem=` 行；`ag mem --help` 拦截目标未执行；
  `ag mem get/set/del` 直通真实 substrate；`ag commands --check` 25 OK
  （24+1）。
- **knowledge 面**：`--workspace …/me k ls` → 1 文件；`evaluate` →
  Quality Score 73/100（良好），SOUL/SP/MEMORY 全 true。

**验收**：新增 scripts/accept/m35-mem.sh 43 断言全绿（产物 5 + 信封/身份
13 + 人面 6 + knowledge 4 + 路由面 8 + 组表/门禁 2 + 清理 3 + scratch 2）；
十二套件回归 267 断言 0 败（smoke 11 + m24 14 + m25 6 + m26 8 + m27 13 +
m28 11 + m30 30 + m31 30 + m32 35 + m33 35 + m34 31 + m35 43）；check.sh
host gate 绿（25 commands OK）。

设备终态：bake #10（879afe8）slot a；agmem 四件套装成（v0.2.0 手装，
manifest 行 + 镜像资产待「推送」/重烤 #11）；/var/bin/aginx-carrier =
M35c 构建 de63f583…；/usr/bin/ag-mem 已推、groups.desc 含 mem= 行（待
重烤 #11 折叠）。M35 收口在 #86 备份通道（M35d 尾件）。

### #86 v1 — 备份通道（设备侧纪律 + 调度，2026-09-03）

**范围**：服务器推送半边依赖 M36 secret sidecar（hub 鉴权），本收据只
落设备侧：一致快照 + 打包 + 保留 + 每日调度。

- **/usr/bin/ag-backup**（group=mem，路由 `ag backup now|list|verify`）：
  python3 sqlite backup API 对活库 carrier.db 做一致快照（WAL 下安全，
  不停守护）；快照 + /home/.aginx（剔除活 db 三件套）+ /var/lib/ag 打
  tar.gz 落 /var/backups/aginx/；保留最新 7 份。实测：/home/.aginx 全量
  5.4 MB → 备份 128K（224 成员），verify（gzip -t + 快照成员在位）过。
- **调度**：busybox crond（rcS 起，`-c /etc/crontabs`——默认
  /var/spool/cron/crontabs 不存在，首启动静默死，-c 钉目录后才活）；
  /etc/crontabs/root 每日 04:17 跑 ag-backup now，日志 /var/log/
  ag-backup.log。**调度链实测**：临时装下一分钟行 → crond log 出
  `USER root pid … cmd /usr/bin/ag-backup now` → backup-040500.tar.gz
  落地 → 行已撤、恢复正典表。
- **已知边界**：busybox crond 无 suspend 补跑（M23b 深睡会跳过夜间槽，
  下一日自续）；/var/backups 不在 state tar（换机丢历史——/home 本身由
  state tar 保，备份是二次防线，接受）。恢复路径手工（脚本头有指引）。
- 上述 rcS/crontabs/ag-backup 均已推设备 + 烤盘在册（待重烤 #11 折叠）；
  `ag commands --check` 26 OK。

m35 套件增 7 断言（50/50）：工件在位、`ag backup list` 路由、crond 活、
crontab 行在册、≥1 备份、最新备份 verify 过。

设备终态（M35 全收口）：bake #10 slot a；agmem 四件套 v0.2.0（md5
0703be45…）；carrier M35c 构建 de63f583…；ag-mem/ag-backup shim + mem=
组行 + crond 常驻。#86 服务器推送半边挂 M36 后续。

## 2026-09-03 — rootfs 重烤 #11：M35 折叠 + provision 时钟闸（实测）

**镜像**：`817df38-20260903`（1 GiB，sha 98fad04c…），M35 全部 /usr/bin 件
入盘（ag-mem / ag-backup 755、/etc/crontabs/root、rcS crond 块、mem= 组
行）。apply 前先踩一跤：manifest boot.size 误填 14516224（boot-test 的
尺寸），agupd 尺寸闸先于任何分区写拒绝——修正为 100663296（boot/boot.img
实尺，sha 不变 e2ce2f17…）重签后一次过：boot_a + vendor_boot_a 写入、
预置 rootfs 体 sha 过、换机头提交、slot a 激活。reboot2 起机
`aginxos 817df38`，boot.state 全绿至 done ok。

**首启抓到 provision 时钟竞态（新 bug，已修）**：换机后 /var/bin 空，
provision 只等 `internet ok` 就 sync，而 ntpd 的 `time ok` 在其后才落——
冷启时钟 1970 下所有镜像 TLS 下载死于证书校验（"certificate not valid
yet: verification time 1831772"），boot.state `pkg fail`。此前每靴 /var/bin
都在（sha 合、零下载）故从未暴露。修复：provision 在 internet 后加等
`time (ok|fail)` 的有界闸（net-bringup 同步写该行，不拖靴）；重启实测
boot.state 顺序变 `time ok → pkg run → pkg ok`。此修复为 adb 推送
（/etc/init.d/provision），下烤（#12）折叠——本轮烤盘在闸之前。

**换机后手工恢复清单（每烤必做，四件套不在 manifest）**：`agpkg sync`
补五件（aginx/carrier/browser/python3/codex，provision 竞态救起后手动跑
一次即可）；重推 M35c carrier（de63f583…，**实测 sync 不回头覆盖**——
按 stamp 判齐，直接推文件不触发重下）；重装 agmem/agb/agf/dup 四件套
（各 v0.2.0 tar）；重链 /lib/ld-musl-aarch64.so.1（python-finalize 标记
在 state tar 里活着而链接不在——bake #9 守卫下次自愈，本轮手动 ln）。
/var/backups 换机清零（既知边界），手动 ag-backup now 重立基线
（132K/224 成员，verify 过）。

**换机后首次 crond 行为（观察）**：rcS 起 crond 后 ntpd 跳钟，crond 报
`time disparity of 29776287 minutes detected` 并补跑 04:17 备份行——落在
provision 竞态窗里（python3 未就位）而干净失败（脚本头 python3 探测先行，
无半备份）。python3 就位后链路自愈。

**验收**：12 套件 274/0（smoke 11、m24 14、m25 6、m26 8、m27 13、m28 11、
m30 30、m31 30、m32 35、m33 35、m34 31、m35 50——m30/m31/m32 首跑红是
四件套未重装，装后全绿）；check.sh 全绿 26 commands OK；重启一轮回
（carrier M35c 原样、crond 自起、loader 链在位、pkg ok）。

设备终态：bake #11 slot a（aginxos 817df38）；M35c carrier de63f583…；
四件套 agmem/agb/agf/dup v0.2.0 全装；crond 常驻（每日 04:17 备份）；
1 份基线备份在 /var/backups/aginx。

## 2026-09-03 — M36 secret sidecar 上机 + USB 主机重启断连事故

**agsecretd 真机 bring-up（M36a/b/c）**：agsecretd+agsecret+ag-secret
shim+secret.policy+agsvc 单元五件 adb 推装（md5 全对）；`agctl start` 即起，
**重启后随 /etc/agsvc.d/ 单元自动拉起**（无需手工）。socket
/run/aginx/secret.sock 0600、目录 0700、store /var/lib/ag/secret/store 0600
（tmp+rename 原子写）。

真线实证（m36 套件 24/0）：put/list/rm 全走 stdin（值不进 argv）；**对端
识别走真实 /proc/<pid>/exe**——admin exe（agsecret）在消费者 scope
（brain.primary）被拒 code=denied；policy 热重载（denied→加白→denied，
不重启守护，字节级还原）；日志只记 op+scope+peer 无值；ag-backup tar
实证不含 secret store。

**carrier sidecar 腿（M36b，f7145cb8… 推 /var/bin）**：`ag api call` 的
hmac 凭证解析实测三级回退——无映射报 "not configured"；policy 加白 +
store 注入后同命令越过解析、死在死端口（127.0.0.1:1 的 transport error）。
即 env > sidecar > 缺席 的插腿在真 SO_PEERCRED 路径上闭环。注意
busybox cp 覆盖运行中二进制报 `Text file busy`——换文件须 stop→cp→start。

**USB 断连事故（根因记录）**：Mac 重启时 27MB adb push 在途，此后手机在
USB 总线零枚举（adb 与 ioreg 均空）而手机用户态完全健康（aterm 界面与
网络正常）。重插无效。电源键整机重启后 10 秒内重新枚举。机理（推断，
非实测）：trampoline 开机一次性绑定 gadget（`usb gadget BOUND`，dmesg
[4.2s]），运行态无重挂路径；主机在活跃传输中途掉电使 dwc3/ffs 端点态
卡死，后续总线复位无应答。**UDC unbind("") 重挂恢复路径在此 4.19 gadget
代码上有 panic 前科（v17 bootloop 尸检，见 trampoline.c:1001 注释），
不可用作恢复手段**；本机已知恢复法 = 整机电源重启（今日实证）。此为
首例，不建自动看护（会走 unbind 路径）。

**验收**：13 套件 298/0（m36 新增 24；其余 274 复跑全绿——m34 首跑 4 红
为设备刚重启网络未收敛的瞬时现象，数分钟后复跑 31/0）。

设备终态：bake #11 slot a；M36b carrier f7145cb8… 在 /var/bin；agsecretd
常驻；policy/store 原字节原空（套件自清理）；crond 常驻。

## 2026-09-03 — rootfs 重烤 #12：M36 折叠 + provision 时钟闸入盘（实测）

**镜像**：`6d6665f-20260903`（2 GiB 稀疏，sha fc62fca5…；烤盘脚本默认已从
1 GiB 提到 2 GiB，换机协议尺寸无关）。apply 走既有 agupd pre_staged 路：
boot_b+vendor_boot_b（sha 与 #11 同——boot e2ce2f17 / vb d80b8098）写入、
state tar 56 MB 落 64 GiB、2 GiB 体块上哈希过、换机头提交、slot _b 激活。
2 GiB 体由 host `cat img | adb shell dd seek=2097153` 直灌（live fs 空闲
不足，push 不可行）。

**时钟闸闭环（#11 遗留 bug 修复入盘）**：首启 boot.state 顺序
`internet ok → time ok → pkg run → pkg ok`——#11 的 1970 证书竞态在烤盘
路径上不再出现，五件 must-exist（aginx/carrier/browser/python3/codex）
全数经镜像 TLS 重下成功。

**agsecretd 从烤盘自起**：首启 `agctl list` 即 `agsecretd ready
/usr/bin/agsecretd`（baked 单元拉起，无需 adb 推装）——M36 系统面就此
脱离 adb 推送依赖。烤盘内 agsecretd/agsecret md5 与 host 构建逐字节同。

**换机恢复清单（较 #11 简化）**：五件 sync 重下；M36b carrier 重推
（f7145cb8…，ETXTBSY 纪律 stop→cp→start）；**注意镜像 provision 拉到的
carrier 是 db8a2997…（镜像现供版本，非 M35c/M36b——推 carrier 后以 md5
为准）**；agmem/agb/agf/dup 四件套 v0.2.0 tar 重装（四 sha 全过）；
**ld-musl 链接这次由烤盘 python-finalize 守卫自愈**（#11 需手工 ln）；
备份基线重立（132K）。

**验收**：13 套件 12 绿 + m34 29/2——两红均为 ip-api.com 外部腿
（`error sending request`）。立案排查结论：**外部路径丢包，非本烤、非
手机、非 carrier**——同 NAT 的 Mac 直连 ip-api 连发 8 发也丢 1 发
（curl 000）；设备侧 DNS/TCP/网关/conntrack（0/262144）逐项实证健康；
当日早间（换机前）同一二进制同一套件曾 31/0 两回。排障中两条设备事实
记录在案：reqwest 直打 `1.1.1.1` 会跟 301 跳 https 而设备无 CA 权
（报 error sending request，勿误判断网）；打路由器管理页回 HTML 报
parse error 同理。

**重启一轮**：6d6665f 原样、pkg ok 快道（done 标记活）、agsecretd
自起、carrier f7145cb8 在位、loader 链在位、crond 常驻；slot _b
succ=1 tries=6。

设备终态：bake #12 slot _b（aginxos 6d6665f）；M36b carrier
f7145cb8…；四件套 v0.2.0 全装；agsecretd 常驻（baked）；crond 常驻；
备份基线 1 份。

## 2026-09-03 — 推送：双仓上 GitHub + carrier v0.3.0 release + 镜像四件套上线（实测）

### git 推送（收据剥离手术）

- 两仓长期未推：aginxos ahead 26（16 代码 + 10 收据）、carrier ahead 19（纯代码）。
  收据提交夹在代码中间，且 258968b/37d5f7d/c793bad 三个代码提交夹带了
  HARDWARE.md hunks —— 直接 push 会把收据带上公开 remote。
- 手术：push-queue 分支按序 cherry-pick 16 个代码提交；HARDWARE.md 冲突一律
  `--ours`（第一次尝试用 `git add -A` 把工作区未跟踪的 ARCH/CARRIER/SYSTEM 卷进了
  3 个冲突提交，`git diff` 校验抓住，reset 重来改用精确 `git add docs/HARDWARE.md`）。
  校验三关：HARDWARE.md delta vs origin = 0；docs/ 仅 DECISIONS.md +18（60bd53c 的
  修宪，属可推）；`git diff push-queue backup-master` 仅 HARDWARE.md 一个文件。
- 推送：aginxos `e3084fe→bad3a82`；本地 master 重排为 pushed 代码 + 单个收据重放
  提交 ccebdc8（10 个收据提交压成 1 个，树与术前逐字节一致）。备份分支
  `m36-prepush-backup` 留存术前历史（可删）。carrier `9a2b10b→d64f2fb` 直推 main。

### release + 镜像

- yinnho/aginx-carrier 5 个 release：v0.3.0（build d64f2fb，musl 27,638,520 B，
  sha256 ada7d7ed…）+ agb/agf/agmem/dup v0.2.0 四件套 tar。gh `--target <短sha>` 报
  422（target_commitish invalid）——去掉走默认分支即成。GitHub 侧 asset digest
  逐一 API 核对与本地 pin 一致（v0.2.1 重切教训照办）。
- 86quan sync.sh 扩展：case 加 `agb|agf|agmem|dup → aginx-carrier releases/download/$1-$2`
  一行；追加 5 条 fetch pin。5 个工�� scp 直传后跑 sync.sh，11/11 OK(skip)
  （sha 门即信任边界，scp 与代理回填同权）。
- manifest 重签（agsign，verify valid）：carrier 行 v0.2.1→v0.3.0 + dup/agb/agf/
  agmem 四行 core（runtime 桥依赖，bare rootfs 没它们 carrier 不会浏览/读/记忆）。
  check.sh 全绿后提交 ec4822f 推送。

### 设备验证（bake #12 换机后的实机）

- 新 manifest+sig adb 推入 /etc → `ag pkg available` 验签通过（未拒签）。
- `ag pkg sync`：10 包全部 "up to date"（新 4 包 + carrier v0.3.0 pin 被正确识别，
  已装 sha 与 pin 一致）。
- 真线证明：httpget 只收 http 且被 nginx 301（169 字节重定向体 sha 相同假文件——
  不跟随重定向）；`ag pkg install` v0 只收本地路径（"no such file: https://…"）。
  最终 `/usr/bin/agdl https://pkgs.aginx.net/agmem/v0.2.0/agmem-v0.2.0-4pc.tar` →
  HTTP 200，3,185,664 B，sha256 4b9c4690… 与 pin 逐字一致。镜像新工件端到端通。
- 化身对话实测：`ag agent send me --message "测试消息：请用一句话回复确认你在线"
  --sender sophie-test` → 内联一轮，回复「在线确认：我在的，「我」随时待命 ✅」
  （carrier→brain API→回复全链活）。

设备终态：slot _b（aginxos 6d6665f 烤盘）+ /etc manifest 9 行新版 + M36b carrier
（f7145cb8，= release v0.3.0 资产）。/etc/hosts 的 pkgs.aginx.net pin 仍在（临时）。

## 2026-09-03 — M38a: aterm 中文渲染（ab_glyph + Noto Mono CJK 子集，实测）

### 做了什么

- `crates/aterm/src/cjk.rs`（新）：ab_glyph 光栅化宽字符。字体 = Noto Sans Mono
  CJK SC 子集 `boot/rootfs/usr/share/fonts/agterm-cjk.otf`（1,529,716 B，md5
  4d5de7fe39be9bfd1780f35f6e1ed715；`scripts/subset-cjk-font.sh` 产出：全 GB2312
  6763 汉字 + A1/A3 标点行 + ASCII，no-hinting；16MB 上游只留 /tmp 不进仓）。
  ASCII 仍走 5×8 位图快路径；coverage 缓存按 (char, px) 上限 1024。字体缺失时
  优雅降级回 `?`（recovery boot 不挂）。
- 宽字符 cell 模型（term.rs）：首格放本字、尾格放 `WIDE_TAIL` 哨兵；put() 带
  放不下先换行、覆盖半宽字时孤儿格清空、wrap_pending 延迟换行。窄非 ASCII
  （— · … ℃ GB2312 A1 行）也走 ab_glyph，1 格宽、px = cell 高 × 0.8 居中。
- **ab_glyph 陷阱（记录给下次动字体的人）**：`PxScale` 是行高（ascent+descent+
  gap）像素而非 em 像素——Noto CJK 行高 1.448 em，直传 PxScale=40 字形只有
  0.69×。修法 `scale = px · height_unscaled/upem`。另外 `OutlineGlyph::draw` 的
  (x,y) 已相对 px_bounds.min，直接索引别再减。

### 验证（host → device）

- host：`aterm --ppm`（ATERM_PPM_DEMO / ATERM_CJK_FONT 覆盖）像素级核对——
  48px cell 内 ~44px 笔画、—— 居中横线、wrap/bold/光标全对。check.sh 全绿。
- device（bake #12 换机后实机，slot _b）：musl 构建 adb 推入，md5 双验（aterm
  b014d9a8…，字体 4d5de7fe…；备份 /usr/bin/aterm.pre-m38a）。`ATERM_INJECT=1`
  经 /etc/init.d/aterm-handoff 常驻（pid 2083 的 environ 验证）。SH 瓦片进终端
  后 `printf '…\r' > /run/aterm.inject` 注入 `echo 你好，世界 —— 化身·互联·记忆在线`，
  文件被即时消费自删，**屏幕人眼确认（2026-09-03）：汉字抗锯齿清晰可读、长破折号
  与间隔号居中、无错位撕裂**。中文显示闭环。
- rootfs 折叠未做（下次重烤 #13 时 cp 字体 + musl aterm，走 clean-reflash 验收）。

设备终态：slot _b 烤盘 + adb 版 aterm（M38a，pre-m38a 备份在位）+ /usr/share/fonts/
agterm-cjk.otf 已推。ATERM_INJECT=1 随 aterm-handoff 常驻（ASR 未来入口，同 inject()）。

## 2026-09-03 — M39: 照片查看器（agimg JPEG 软解 + DRM 直出 + PHOTOS 瓦片，实测）

### 背景（SM7250 编解码器盘点，probe 实证）

对 vendor 模块清单（334 个 .ko）strings + 设备 /vendor/firmware 盘点：**SM7250
没有 JPEG 解码硬件**——msm-vidc.ko（Venus）解码面是 H264/HEVC/VP9/VP8/MPEG2、
编码面 H264/HEVC，无 JPEG；camss cam_jpeg*.ko ×4 是相机管线**编码**件；无其他
编解码硬件。正常 Android 手机照片也是 CPU 软解——所以照片走 libjpeg-turbo（NEON），
视频才走 Venus 硬解（M41，venus.b00-b04+mdt 固件已在设备、模块在清单内）。
用户拍板：**照片软解 + 视频硬解**。

### 做了什么

- `crates/agimg`（新）：vendored libjpeg-turbo 2.1.5.1 子集（~830KB；上游 16MB
  全树只在 /tmp，vendor/README.md 记再生法）。build.rs 编 51 个核心 C + aarch64
  NEON_INTRINSICS 路径（13 个 arm neon .c + aarch64 jsimd/jchuff-neon；musl 交叉
  构建确认 67 个 .o 含 *-neon.o）。所有 jpeglib 结构知识留在 C——agimg_shim.c 一
  个函数过 FFI：longjmp 错误管理、DCT 缩放从 1/1 向下走到能塞进屏幕的最大档、
  JCS_EXT_BGRX 直出 = LE u32 0x00RRGGBB = DRM XRGB8888 原样 blit。
- `crates/aterm/src/photos.rs`（新）：Mode::Photos 状态机（Picker 同款非终端
  全屏模式）。照片住 `/home/photos`（state tar 含 /home，重刷存活），mtime 倒序、
  tap 右/左半屏翻页带环绕、BACK 视图→列表→启动器、img=None 释放 ~3MB。列表空态
  提示 `AG CAM-SHOT --JPEG-OUT /HOME/PHOTOS/...`。启动器新 PHOTOS 瓦片（永远
  可用，纯 aterm 状态无 pty）。
- 拍照入口 = 既有 cam-shot：`--stream --rear --jpeg --jpeg-out <path>`（注意：
  **不带 --stream 只探测不拍**；--frames N 连拍时所有帧写同一 jpeg-out 路径——
  后帧覆盖前帧，要连拍出多文件用 --out 分帧的 raw 路径约定）。

### 验证（host → device）

- host：agimg 4/4 测试（全尺寸/1:4 DCT 缩放/渐变往返像素断言/垃圾输入 None）；
  aterm --ppm 第三帧照片路径像素核对（渐变 fixture 左蓝右红、居中偏移精确）；
  check.sh 27 命令全绿。
- device（bake #12 slot _b）：musl aterm（840,688 B，md5 b9e0d62f52cc3fe90a28cb
  261750）推入 /usr/bin/aterm，pre-m39 备份在位；kill 后 handoff 复活 pid 5767、
  ATERM_INJECT=1 仍在。实拍两张：桌面黑帧 32,045 B（0.11 bpp）+ 举机窗景
  159,474 B（0.56 bpp），均 2016×1136 color q85、~0.19 s/帧编码。
- **人眼验收（2026-09-03）**：PHOTOS 瓦片 → 列表两行 → 点开窗景照正常显示居中、
  黑帧与窗景两张都渲染、翻页/返回路径走过。**用户指出颜色不正常**——已知 M19
  软件去马赛克固定白平衡局限（无硬件 ISP AWB），归 M19d IFE PIX 线（短期可先做
  软件灰世界 AWB 小步）。
- rootfs 折叠未做（重烤 #13 与 M38a 一起 cp + clean-reflash 验收）。

设备终态：slot _b 烤盘 + adb 版 aterm（M39，pre-m39 备份在位）+ /home/photos 两张
实拍。aterm-handoff 常驻照旧。

## 2026-09-03 — M40: 拼音输入法（单音节 IME + iOS 键盘 + busybox 宽字符修复，实测）

### 做了什么

- `crates/aterm/src/pinyin.rs`（新）：单音节拼音 IME。413 音节候选表（14.5KB
  TSV，频序排序，`scripts/gen-pinyin-table.sh` 从上游频表再生）include_bytes!
  进二进制。ü 用 v 输入。`feed()` 吃 KeyEvent 出 Commit/Consumed/Pass——字母
  进缓冲、空格/回车上屏首选、DEL 退格、非字母直通。
- `crates/aterm/src/kb.rs`：键盘整排重写为 **iOS 布局**（用户拍板）。三字母行
  qwertyuiop / asdfghjkl（居中内缩）/ SHF+zxcvbnm+DEL；底行 [123 w3][拼 w2]
  [空格 w8][。/. w2][换行 w3]，按行归一权重。页模式闩锁（Letters/Num/Sym 仅经
  123/#+=/ABC 切换）；shift 一次性；拼开启时句号键显 。、空格键显 空格。
  ESC TAB CTL ←↓↑→ 留在键盘上方细行（Termux 式）。旧 hack（?↔/ 映射、
  DIGIT_SHIFT、大写存储、SPECIALS 五键行）删除。
- `crates/aterm/src/main.rs`：候选行 = 键盘上方 8 槽浮条（缓冲 | 6 候选 | 翻页
  箭头），点候选即上屏并清缓冲（`take_candidate`——点候选和空格/回车一样是
  提交）。提交走既有 inject() pty 注入路径，无新面。
- **按压反馈**（用户轮 2）：手指按下到抬起期间，所按键帽亮——绿框 + 白字
  （复用 keycap active 样式）。`Kb::pressed` 跟踪 (area,row,idx)，Down 定位、
  Tap/Up 清除。
- **busybox 1.36.1 重编**（shell 中文显示 `?` 的根修）：旧版
  CONFIG_UNICODE_WIDE_WCHARS 关 → ash lineeditor 把 wcwidth==2 的 CJK 全部替换
  成 CONFIG_SUBST_WCHAR=63（`?`）。新配置 WIDE_WCHARS+COMBINING 开、
  LAST_SUPPORTED_WCHAR 65534、TC 关（zig musl 头缺 CBQ，设备无人用 tc）；
  trylink 两处 -Wl,--warn-common/-Map 摘除（zig ld 不认）。zig cc 静态构建，
  unstripped 1,248,040 B（macOS strip 与 zig objcopy 均不可用，无害）。
  `scripts/build-busybox.sh` 固化再生法；boot/rootfs/busybox 已换新。

### 验证（host → device）

- host：pinyin 7 测试 + kb 5 测试（iOS 几何：行内缩/权重/DEL/空格中心/底边
  对齐 2340；shift 一次性；页闩锁；句号随 拼 变化；ctrl 和弦+方向键）全绿；
  `ATERM_IME_DEMO=ni --ppm` 帧像素断言：6 候选槽 hanzi 尺寸、拼/空格/换行 三
  标签墨迹高度均 30px（字号统一，用户轮 2 ②）；check.sh 27 命令全绿。
- device：busybox 推 /bin/busybox（备份 .pre-unicode）——pty 回显测试
  （execv argv[0]='sh'，实测法：直接调 busybox 路径会让 shell 秒死、内核
  termios 裸回显造成假阳性）确认 `你` 回显 `\xe4\xbd\xa0` 非替换；`echo 你好`
  直出中文。aterm 推 /usr/bin/aterm（备份 .pre-m40，md5 cf6b8f9c）、kill 后
  handoff 复活 pid 9722 ATERM_INJECT=1 保持；smoke 套件 11/11。
- **用户上手实测（两轮）**：轮 1 后反馈三问题——shell 中文显示（busybox 根修）、
  候选上屏后缓冲不清（take_candidate 修复）、键盘照 iOS 排版（重写）——全部
  修复上机。轮 2 后反馈两打磨——按键按下变色（按压反馈实现）、拼/空格/换行
  字号统一（空格 scale 3→4）——上机后用户确认"外面加一个框就挺好的"，验收过。

设备终态：adb 版 aterm（M40 收口版）+ 新 busybox（unicode 版）在位，两备份
（aterm.pre-m40 / busybox.pre-unicode）可回退。rootfs 折叠归重烤 #13。

## 2026-09-03 — rootfs 重烤 #13：M38a+M39+M40+busybox unicode 折叠 + 首次零手工换机（实测）

**镜像**：`b32e1e7-20260903`（2 GiB 稀疏，sha 3c344acb…，140 MB 内容）。boot
e2ce2f17 / vendor_boot d80b8098 复用（#11 起未变）。apply 走 agupd pre_staged
路：2 GiB 体 host dd 直灌 92 s → boot_a/vendor_boot_a sha 过 → state tar
59,453,440 B 落 64 GiB → 体原位哈希过 → swap committed（old fs 2,040,373,248
B）→ slot _a 激活。18:38:38 reboot2 → 18:41:18 起机（~160 s 含蹦床换血）。

**首次 apply 翻车两记（协议使用面，非协议本身）**：① manifest 里 rootfs sha
手抄多了一位（65 hex）→ pre_staged 哈希比对差一位失败；教训=**manifest 的
sha 必须脚本化 `shasum | cut` 注入，不手抄**。② `agupd apply --no-reboot
<m>` 参数序错（flag 在前被当 URL 吃掉）；正确 `apply <m> --no-reboot`。另
adb shell PATH 无 /usr/bin，agupd/agctl 都要全路径。

**折叠内容**：M38a 字体（build-rootfs.sh 补 `cp usr/share/fonts`——此前
recipe 有文件但烤盘脚本漏拷）、M39 agimg+照片模式、M40 拼音 IME+iOS 键盘
（均在 aterm 二进制 cf6b8f9c 内）、busybox 1.36.1 unicode 重编 2d9909b3、
ATERM_INJECT=1 入 aterm-handoff（8257e39——M17 起手工导出的环境变量，烤后
首靴丢失才暴露这笔债）。

**首启全链 ok**：wifi Legrand AP → dhcp 192.168.0.166 → internet → time
2026-09-03（时钟闸先于 pkg）→ pkg ok → py ok 3.12.14。provision resync 自动
补回全部 /var/bin：五件 must-exist **加四件套 agmem/agb/agf/dup 首次全自动回**
（#87 镜像清单已含四件套——**换机零手工恢复首次达成**）；carrier 补回
f7145cb8（镜像现供=M36b 版，与烤前一致，无需手推）。/home/photos 两张实拍
经 state tar 存活。agctl 全 ready（aginx/carrier/browser/agsecretd/net-watch）。

**二靴**：b32e1e7 原样、pkg ok 快道、ATERM_INJECT=1 在位（新 handoff 生
效）、slot _a pri=3 act=1 succ=1。

**验收**：13 套件 12 全绿 + m35 首轮 47/3——二、三轮连跑 50/0 复绿（首靴
窗口竞态，不立案）。烤盘 busybox echo 中文正常；**新 busybox awk 仍段错误**
（1.36.1 上游问题未随 unicode 修复——sed/set-- 纪律不变）。

设备终态：bake #13 slot _a（aginxos b32e1e7）纯烤盘运行——烤前的 adb 版
aterm/busybox 与其 .pre-* 备份已随旧 fs 换血淘汰；四件套+备份基线在位。

## 2026-09-03 — M41：Venus 视频硬解首帧（V4L2 M2M stateful H264 → NV12）

**模块与节点**：boot/out/vendor-modules 全装（msm_vidc + videocc_lito 为新增两枚，
fastcvpd/subsys_restart/ion_alloc/qtee_shm_bridge/llcc_slice 等既有依赖随载）；
mknod /dev/video32(81:32 解码) video33(81:33 编码)。fw 经 PIL 加载：
VenusHostDriver VIDEO.IR.1.2-000，built Jun 9 2023。**rmmod 依旧禁止**（整机纪律）。

**解码路径**（`/tmp/probe vidc decode <in.h264> <out> [n]`，md5 e0b0d922）：ION
heap_mask 0x2000000 分 dmabuf → mmap；QBUF 契约 = USERPTR + fd 放
planes[0].reserved[0]（MSM_VIDC_BUFFER_FD），plane 长 4K 对齐；S_EXT_PADS 编码端
31 缓冲，capture 最小 4；poll 需 POLLIN|POLLOUT|POLLPRI 三事件。本核 v4l2_buffer
88 B（planes 走 m 指针）、v4l2_plane 64 B。

**首帧不出的根因（源码级）**：AU 切分器原把首个 slice 前的 SPS/PPS/SEI 全丢 →
fw H264 slice-header 解析器失步：每条 EBD 带 DATA_CORRUPT（EBD flags 0x404000；
该 flag 只在 msm_vidc_common.c:2557 对 VIDC_ERR_BITSTREAM_ERR 置位），fw printf
`h264VspRefPicListReordering(383): RES_EMPTY_CHECK: status: 1001`，EBD offset 卡
在 AU 中段，FBD 零条。**修法**：AU0 = slice 前全部参数集（config-only 探测 +
V4L2_BUF_FLAG_CODECCONFIG 标注；该 flag 经 msm_vidc_qbuf → vb2
__fill_vb2_buffer 三层不丢，已逐层核对）。修复后一次出帧。

**观测**（trace /tmp/trace-m41.txt，2691 行）：testsrc2 H264 320×240 54642 B /
31 AU → **10 帧硬解**：frame0 flags 0x4008 KEYFRAME、1–9 0x4010 PFRAME，全部
EBD 干净（0x4000 = TIMESTAMP_COPY）。

**NV12 布局**（320×240，逐字节画像得出）：plane0 393216 B = Y@0（stride 512 ×
240 行）+ 128KB 对齐缝 + UV@0x40000（stride 512 × 120 行）；plane1 16384 B。

**像素级验证（host 对照 ffmpeg 同流软解）**：去 stride 的 Y 平面 vs ffmpeg
yuv420p passthrough 原生 Y：**76800 像素中 76480 bit-exact（99.6%），MAE
0.496**；差异 320 px 全在 row 0（两解码器顶边 deblock 行为差，视觉不可见）。
stride 判别：512 假设 MAE 0.5 vs packed 320 假设 89.6。**Y 为 limited range**
（Venus 直出流原生值；ffmpeg `-pix_fmt gray` 会做 full-range 展开——对照必须走
yuv420p passthrough 取 Y，本节首版对照 MAE 8.5 即踩此坑）。帧号核对：our dec0
对 ffmpeg 帧 0–4 MAE 单调升（8.5→15.7），索引对齐无漂移。

host 侧 check.sh 全绿（ioctl/errno 跨平台化后 probe crate 进 cargo test 门）。

设备终态：模块全载（不卸）、tracefs 挂在 /sys/kernel/tracing（tracing_on=0）、
/dev/video32/33 在位、/tmp/dec.{0..9}.{p0,p1} 十帧在盘、/tmp/probe=e0b0d922。

## 2026-09-03 — M41（下）：NV12 零拷贝 DPU 直出（VIG plane + 硬件缩放）

`/tmp/probe vidc show <in.h264> [hold_s]`：解码 FBD 里把 venus capture dmabuf
经 PRIME_FD_TO_HANDLE 直入 DPU（ADDFB2 NV12 双平面：pitch=512、offset
[0,0x40000]——UV 用逐字节扫描定位，与本页上节数字一致），SETPLANE 等比放大
（320×240→1080×810 居中于 1080×2340），CPU 全程不碰像素。**肉眼收据：上屏
确认**（黑底中央 1080×810 视频窗，约 1 s 30 帧播完 + hold 定格；`plane off
rc=0`，全程零 scanout 报错、零新 SDE error）。

四道门，逐个排掉（源码级，redbull 树核对）：

1. **plane 不可见**：不带 `DRM_CLIENT_CAP_UNIVERSAL_PLANES` 时 GETPLANE 只列
   overlay，而 sde 的 overlay（DMA pipe）全 RGB-only（8 个 plane，37 格式表
   完全一致）。带 cap 后多出 plane 54/74（VIG，48 格式含线性 NV12/NV21/
   NV16/TP10/P010）——54 是 crtc105 的 primary（bound），**74 是空闲 VIG
   overlay**（type=1），即目标。
2. **EACCES**：本内核 ioctl 表把 SETPLANE/SETCRTC/OBJ_SETPROPERTY 全挂
   `DRM_MASTER`（drm_ioctl.c:648），且 `drm_is_current_master` 无 root 旁路——
   aterm 常驻持 master。**probe 自己持 master**：kill aterm 后 SET_MASTER
   （ATERM respawn 循环有 2 s 缝，kill 后 sleep 1 起跑稳赢）。注意 aterm 之死
   触发 msm master-drop 钩子**灭屏并清 CRTC**，所以持 master 的第一件事是
   自己冷 modeset（DSI conn 29→enc 28→crtc 105，mode 1080x2340x60x60948cmd，
   黑 dumb fb + SETCRTC 带空 connector 重试——aterm 配方原样）。probe 退出放
   master→再灭屏→handoff 复活 aterm 重拿 master 重 modeset，自愈闭环。
3. **EINVAL 甲（zpos 同台）**：sde custom-client 模式下**所有 plane 默认
   zpos=0**（sde_plane.c:3562 只在非 custom 分支给 overlay 发 index+1 默认），
   黑底与视频同落 blend stage 0，src-split 序检查拒叠全宽矩形（kmsg：
   `invalid coordinates, stage:0 l:0-1080 r:0-1080`）。zpos 是原子属性，须先
   `DRM_CLIENT_CAP_ATOMIC` 才在 OBJ_GETPROPERTIES 里现身（此前只列 10 个
   legacy prop），然后**首个 SETPLANE 之前**把 plane 74 zpos→1。
4. **EINVAL 乙（不值一提但记一笔）**：曾想把 zpos 设 255——custom-client 下
   zpos_max=maxblendstages-1=7，越界即拒；1 就够。

uv_off 扫描定位与 ADDFB2 的 offsets 联动：本地 stride×scanline 推算 UV 在
98304（0x18000），解码实测 0x40000（两者不同：fw 对 UV 另有对齐规则，未深
究）；scanout 取实测值。

host check.sh 全绿。设备终态：模块全载（不卸）、aterm 存活（14263，handoff
自愈正常）、/tmp/probe=12c457cd、/tmp/vidc-test.h264 在盘。
M41 余项：PCM 音频同步（音频轨并行播 + 帧节奏对齐）；encode 线可选 M41b。

## 2026-09-03 — M41（终）：PCM 音频同步（音频钟主时钟 + 节拍器感知收据）

`/tmp/probe.new vidc play <in.h264> <in.s16> [vol]`：解码/直出链不动，帧节奏
从 33 ms sleep 换成**音频设备时钟门控**。音频侧 = M18 说-路径原样（MM1
`/dev/snd/pcmC0D0p` 48 kHz stereo S16_LE，HW_REFINE 只钉 format/rate/ch、
period/buffer 松界——cDSP 只认自己的量子；audio-bringup 开机已铺好 mixer 路由），
新原语只有一个：`SNDRV_PCM_IOCTL_DELAY`（0x80084121）作主时钟，
`played = written − delay`。

**感知收据（终局）**：节拍器测试片——host ffmpeg 生成 30 s：每秒首 100 ms
白闪（geq 限幅 235/16，逐帧 YAVG 验证）+ 每秒首 70 ms 1 kHz 蜂鸣（aevalsrc，
RMS 窗验证 −7.4 dB / −inf）。设备上闪与哔**用户判定「同时」**（2026-09-03，
眼+耳）。机制收据：EXIT=0 ×3 轮、900 帧全解码、feeder 灌满 1,440,000 帧
（=30.000 s 整）、`plane off rc=0`、播放全程 DELAY 活、无 fallback。

三条设计决定（都有设备证据）：

1. **fd 全程 O_NONBLOCK**（snd-play 是开完再清）：阻塞 WRITEI 会在 substream
   锁路径里睡满一个 period，卡死解码循环的 DELAY 查询。feeder 用 poll(POLLOUT)
   +部分写推进（alsa-lib 同款机制）。
2. **音频时间线锚在首帧上屏之后**（frame 0 先 SETPLANE、再 start feeder——
   采样 0 与画面 0 构造性对齐）。曾先启音频后显示，首帧测得恒定 +92 ms 领先。
3. **DELAY 的域差与 SETUP 态 EBADFD**：遥测 delta 恒 +91.8 ms（10 s 内漂移
   <0.1 ms，钟率精确）。锚定修正不改此数 → 它不是启动偏斜，而是 q6 FE 的
   delay 上报不含全部 DSP 路径延迟的**恒定域差**；wait 门 LEAD=4 ms 下真实
   闪/哔落地同时（用户收据），不需补偿常数。另：非阻塞 DRAIN 只翻 DRAINING
   即返 EAGAIN（尾巴自己放空），放空后 SETUP 态 DELAY 答 EBADFD
   （do_pcm_hwsync 无此分支）——feeder-done 标志下视为"音频已完、门全开"，
   不算错误不触发 fallback。

调试弯路记一笔：起跑版本里 `frames_done += 1` 在显示块**之前**，而启动闸写
`frames_done == 0`——永假，feeder 从未启动，视频回落 33 ms 固定 pacing 且无
任何报错（EXIT=0 照旧）。教训：静默降级路径也要有日志。

host check.sh 全绿（27 commands OK）。设备终态：模块全载（不卸）、aterm 存活
（自愈正常）、/tmp/probe.new=7a42915a、/tmp/sync.h264 + /tmp/sync.s16 在盘。
**M41 全里程碑闭环**：decode（1f41bb0）→ 零拷贝直出（1f0207a）→ 音频同步
（本次提交）。encode 线另立 M41b。

## 2026-09-03 — M41b：Venus 硬编码 bring-up（msm_vidc_venc H264 encode）

`vidc enc <in.yuv> <out.h264> <w> <h>` 一次通过。设备实测（redfin,
/dev/video33, card=msm_vidc_venc）：

- **协商几何**：CAPTURE(码流)=H264 1 plane sizeimage=245760；OUTPUT(原始)=
  NV12 1 plane sizeimage=393216，**stride=512 scanlines=512（uv_off=0x40000）**。
  S_FMT copy-back 即真相——bytesperline=VENUS_Y_STRIDE、plane_fmt[0].reserved[0]
  (u16)=VENUS_Y_SCANLINES。320x240 下两者均 512 对齐（与 M41 解码侧实测的
  UV 落位 0x40000 互证；techpack 的 DPU 头文件副本写 128/32 对齐是旧拷贝，
  不可信）。
- **yuv420p→venus NV12 转换**（设备侧）：逐行 Y 拷贝按 stride、UV 交错落
  stride×scanlines，pad 清零。fw 按此布局读对（EBD 全净、无 CORRUPT）。
- **EOS 路径**：本驱动无 V4L2_BUF_FLAG_LAST 通道——drain 用
  `VIDIOC_DECODER_CMD`(_IOWR('V',96,72)=0xc0485660, cmd=V4L2_DEC_CMD_STOP)，
  驱动自配内部 4K EOS 缓冲送 fw；完成信号 = **CAPTURE FBD 带
  V4L2_BUF_FLAG_EOS=0x02000000（vendor 值）**，实测 `chunk#31 bytes=0
  flags=0x2004000 EOS` 确定性收口。首跑按上游值 0x2000（=此内核的
  TIMESTAMP_MONOTONIC 位）检测，永远等不到、靠 poll 超时收尾——vendor
  uapi 的旗标值必须逐个核对，不能拿主线上游的记。
- **双端口状态机与解码同源**：pre-feed OUTPUT 帧 + 双 STREAMON，deferred
  flush 在 START_DONE 同一 ioctl 内完成。INPUT min_host=4、CAPTURE min_host=5
  （queue_setup 低于 min 只警告）；实测 4+6 跑通。
- **首块码流 flags=CONFIG（26 B SPS/PPS）**，chunk#1 KEY(3856 B IDR)，后续
  P 帧 0x4010（PFRAME）。默认档：H264 **High** profile（驱动默认，未设 CID）。
- **质量收据**：30 帧 320x240 → 22,410 B（~179 kbps）。host ffmpeg 解回
  yuv420p 30/30 帧，IDR 42.3 dB、最差帧 27.4 dB、**全局 PSNR 29.41 dB**
  （默认低码率下的正常有损压缩；码率/GOP/I 间隔 CID 未调，属后续）。
- **闭环互证**：设备上 `vidc decode` 解这份硬编码输出——EXIT=0、3 帧、
  无 DATA_CORRUPT。yuv→NV12→venc→H264→vdec→NV12 全链硬件。

host check.sh 全绿（27 commands OK）。设备终态：模块全载（不卸）、aterm
存活、/tmp/probe.new=b69ea3c5、/tmp/test30.yuv + /tmp/enc.h264 在盘；未动
DRM/未重启。M41b 完成，Venus 编解双通。

## 2026-09-03 — M41c：vidc play 三修 + 扬声器噪音立案（挂起）

素材：~/Downloads/clip-shot-001.mp4（720x1280@24，5.04s，AAC 44.1k 立体声）。
host 转码 `-c:v libx264 -profile:v baseline -bf 0 -g 48 -crf 20` 保原生分辨率
（md5 f3bd760f），音频 `-ar 48000 -ac 2 pcm_s16le`（0a1e0ff1）。

三修（全部设备观察收据）：

- **全屏/画质**：decode() 的 S_FMT hint 从硬编码 320x240 参数化为
  `vidc play <h264> <s16> [vol] [heapmask] [w h fps]`。720x1280 协商实测
  OUTPUT sizeimage=7,077,888、CAPTURE=(2359296,16384) bpl=1024——venus
  stride 1024/scanlines 1536 再次证明 S_FMT copy-back 是唯一几何真源
  （host 侧 128/32 对齐算法会算出 1,474,560 严重低估）。DPU 1.5x 缩放出
  1080x1920 满宽，用户判定「全屏，画面还行」。
- **撕裂**：SETPLANE 后不再立即 requeue 显示缓冲——持帧一拍，
  DRM_IOCTL_MODE_WAIT_VBLANK(0xc018643a，**union 必须按 24B reply 臂传，
  16B 请求臂会被 copy-out 打爆栈**；pipe 骑在 type 位 30-31) 等翻转落定
  后释放前一帧。实测 sde 接受该 wait（无 errno，vblank_ok 未触发降级）。
- **挂死**：音轨比视频短 17ms 时，feeder done 后音频钟冻结在末值，最后
  一帧 pts 永远等不到（45.25s vs 45.27s 实测挂死）。wait_until 在 done 后
  改用 start_at 墙钟（两钟同源同 1x），收口 EXIT=0 确定性。

扬声器噪音（**挂起，与 M19d 拍照/画质线合并处理**）：真人声音乐素材
「太嘈杂、声音越大吵得越厉害」，同一文件 Mac 上干净；M18 老播放器
ag-snd-play 同样嘈（排除 vidc play 链）。分诊记录：降电平 0.4x 仍不净、
0.15x 好转；满电平切 180Hz 以下反而更糟（电平主导）；单声道 FE 会话
（QUIN_TDM Channels=2 下）能开能放但**无声**。根因指向：CS35L41 智能功放
裸奔——/vendor/firmware 无 cs35l41 固件、dmesg 无 amp 校准、无 ACDB HAL
喂 /vendor/etc/acdbdata（校准数据在盘：Handset_cal 等）。已在 vidc play
feeder 加软件护栏（180Hz 单极高通 + -4.2dBFS 跑动峰值限幅）但**用户判定
仍不行**——软件护栏不解决功放无保护固件的失真，真解在厂商校准链
（与 M19d ISP 画质同族）。omarchy-rs-main 已查：纯 CLI/插件生态
（agents/cleaner/cli/compat/crash/learn/network/plugins/skills），零媒体代码。

设备终态：aterm 存活（20700）、模块全载、/tmp/probe.new=7aefa0e7、
clip.h264/clip.s16 与分诊素材在盘、混音器已恢复厂商值（AMP 17/PCM 817）、
未动 DRM/未重启。host check.sh 全绿（27 commands OK）。
