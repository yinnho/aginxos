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
