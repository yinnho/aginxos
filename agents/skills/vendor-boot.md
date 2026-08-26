# vendor_boot Patches

Read this before creating or changing anything under `boot/` that modifies the
Pixel 5 boot path.

## Why we patch vendor_boot

On redfin the bootloader loads **two** ramdisks: `boot.img`'s and `vendor_boot`'s.
The vendor ramdisk's `init` symlink overwrites anything we put in the boot
ramdisk for the same path. So patching only `boot.img` `/init` has no effect —
the patch must go in `vendor_boot`. That is the whole reason this directory and
`pack-vendor-boot.sh` exist.

## The model

`pack-vendor-boot.sh` unpacks the stock `vendor_boot`, injects our binaries and
flags into a `/aginxos/` dir in the vendor ramdisk, sets
`rdinit=/aginxos/trampoline` on the vendor cmdline, and repacks.

What goes into the ramdisk:

| Path | What it is |
|------|-----------|
| `/aginxos/trampoline` | C program, the `rdinit` entry. mounts, optional module load + DRM splash, then `execve`s `/aginxos/first_stage_init`. |
| `/aginxos/first_stage_init` | stock static first-stage init, preserved for handoff. |
| `/aginxos/aginxos-init` | Rust helper (splash-test child / future work), optional. |
| `/aginxos/aginxos-probe` | probe binary, optional. |
| `/system/bin/init.android` | stock Android init, renamed + preserved so handoff still works. |

The C trampoline is the **only** proven `rdinit` on redfin. A Rust `rdinit` that
`execve`s first_stage was tried and hung — do not reintroduce it as the entry
point (see `docs/HARDWARE.md` "Handoff breakthrough").

## Feature flags

Empty files under `/aginxos/` gate behavior at runtime. The trampoline checks
for them with `exists()`; `/aginxos/storage` is checked by `aginxos-init`
after the PID 1 takeover:

| Flag | Effect |
|------|--------|
| `/aginxos/hold` | pid1 loops forever, never hands off. Success = stable stuck Google logo. |
| `/aginxos/splash` | run the DRM splash sequence before handoff. |
| `/aginxos/usb-adb` | bring up the ffs.adb gadget console (see below). |
| `/aginxos/usb-diag` | load the same `modules.usb` chain, dump extcon + deferred-probe state, force a `drivers_probe` replay of `a600000.ssusb`, then hand off with modules loaded so the full kernel log can be pulled via `su -c dmesg` on booted Android (the ring buffer survives handoff). |
| `/aginxos/usb-probe` | probe-verdict mode: binary reboot-timing signal (auto-reboot = UDC appeared, hold = no UDC). |
| `/aginxos/load-modules` | load `/aginxos/modules.allow` (small curated list). |
| `/aginxos/load-modules-loadfile` | load stock `modules.load` but stop at `msm_drm.ko`. |
| `/aginxos/load-modules-full` | load the entire `modules.load`. Riskier. |
| `/aginxos/storage` | aginxos-init (as PID 1) loads the UFS chain and mknods the block nodes from `/proc/partitions`. |
| `/aginxos/super` | aginxos-init also parses super's liblp metadata and mounts the `_a` sub-partitions (system, vendor, product, system_ext) ext4-ro via dm-linear at `/<name>`; implies `storage`. |

`pack-vendor-boot.sh` takes env flags that write these:

```bash
HOLD=1 SPLASH=0 USBADB=1 STORAGE=1 SUPER=1 ./boot/pack-vendor-boot.sh
# HOLD:       0|1
# SPLASH:     0|1
# USBADB:     0|1  — ffs.adb console + /aginxos/modules.usb chain
# USBDIAG:    0|1  — same chain; diagnostics + Android handoff for log pull
# USBPROBE:   0|1  — verdict mode (reboot-timing signal)
# MODULES:    0 | 1 (modules.allow) | drm (modules.load through msm_drm)
# MODULES_FULL: 0|1
# STORAGE:    0|1  — aginxos-init brings up UFS after the takeover
# SUPER:      0|1  — aginxos-init mounts super _a sub-partitions after the takeover
```

### USB gadget console (`/aginxos/usb-adb`)

Implements ROADMAP Phase 0. The trampoline loads the `modules.usb` dwc3 chain
(topological order, per HARDWARE.md recon), waits for the UDC
(`a600000.dwc3`), builds gadget `g1` on configfs with stock recovery IDs
(`0x18d1`/`0xd001`), mounts functionfs at `/dev/usb-ffs/adb`, forks the
ramdisk's own `/system/bin/adbd` (bionic runtime included in-tree of the
ramdisk), then binds the UDC.

Note: acm (serial gadget) does not exist in this kernel — do not plan around
it. rndis works only with a Linux host (macOS ≥12.3 dropped RNDIS).

When `usb-adb` is set, the trampoline **skips the pre-handoff module unload**:
yanking dwc3 under adbd kills the console, and USB modules are not the display
set that poisons first_stage. Handoff with the console up is untested — verify
with HOLD=1 first.

The safe default (no flags) is HOLD only, no modules/splash.

## Trampoline kmsg vocabulary

The C trampoline logs to `/dev/kmsg` with the `aginxos-trampoline:` prefix.
These lines are the bring-up signal — grep for them after a handoff attempt:

- `start v5` — pid1 ran
- `usb console begin` / `no /aginxos/modules.usb list` — console entry
- `splash disabled` / `splash without module load` / `loading modules.load through msm_drm` / `loading modules.allow` — which splash module policy was taken
- `modules ok=N fail=N skip=N from <list>` — per-list module load result
- `udc=<name>` / `usb NO UDC — controller did not come up` — dwc3 state
- `usb diag begin` / `no /aginxos/modules.usb list` — diag-mode entry
- `extcon <path>: <name|state>` — extcon supplier snapshot (stock success: extcon0=eud extcon3=smb5 extcon4=pdphy USB=1)
- `regulator <path>: <name>` — regulator snapshot; `ext_boost` proves tps-regulator probed, `smb5-vbus`/`smb5-vconn` only exist after smb5 probed
- `deferred /dbg/devices_deferred: <devices>` — who never finished probing (debugfs mounted at `/dbg`; this kernel has no /sys/kernel/debug)
- `ssusb bound to msm-dwc3: yes|no` — driver binding state
- `drivers_probe <dev> -> <rc>` — forced probe-replay result (0 = write accepted)
- `udc=<name> AFTER drivers_probe - replay was the gap` / `still no UDC after drivers_probe kick` — H2 verdict
- `diag done - modules stay loaded for Android handoff` — diag exit; kmsg is dead after this (mounts detached)
- `configfs mount errno=N` / `functionfs mount errno=N` / `ffs symlink errno=N` — gadget setup failures
- `adbd did not open ep1 (exec ok?)` — adbd failed to start
- `usb gadget BOUND — adb should enumerate` / `UDC bind FAILED` — final result
- `usb console on — skipping module unload` — handoff path with console active
- `opened /dev/dri/card0` / `open card* failed` — DRM node state
- `res crtc=N conn=N enc=N fb=N` — mode resources
- `using active CRTC mode` / `using connector mode` / `using hardcoded 1080x2340` — which mode path won
- `ADDFB2 fmt=... ok` / `ADDFB legacy ok` / `ADDFB errno=...` — framebuffer creation
- `SETCRTC fail e1=.. e2=..` — modeset failure
- `DRM splash OK` — ioctls returned success (not the same as visible pixels)
- `splash SUCCESS` / `splash FAILED all frames` — frame-level summary
- `unloading modules before handoff` / `rmmod ok=.. fail=..` — pre-handoff module unload
- `exec first_stage` — attempting Android handoff

When debugging a bootloop, reproduce the exact same flag set, capture kmsg via
pstore, and map the last trampoline line to the section of `trampoline.c` that
emitted it.

## Non-negotiables (each one has a bootloop in `docs/HARDWARE.md` behind it)

- **Do not** load full `modules.load` in early init — bootloops.
- **Do not** `mmap /dev/mem` at `cont_splash_region` — bootloops on redfin.
- **Do not** fake `/dev/dri/card*` with mknod — that masks a missing driver.
- **Do not** hand off to Android *after* loading display modules without first
  unloading them — the pre-loaded modules poison first_stage. The trampoline
  calls `delete_module` (force) before `exec first_stage`.
- **Do not** rewrite `vendor` or `vbmeta` to "fake" a splash — early color is a
  bootloader job, not an `rdinit`/init-hook job (DECISIONS §0).

## cmdline parameters

The vendor cmdline is read from `boot/out/vendor_boot_unpack/info.txt` by
`pack-vendor-boot.sh`; it appends `rdinit=/aginxos/trampoline` if absent. Never
hardcode the full cmdline in a script or a doc — the unpack output is the source
of truth.

## Testing a patch

Full loop is `./scripts/flash-early-splash.sh` (see
[`agents/skills/boot-experiments.md`](boot-experiments.md)). Quick manual sanity:

```bash
# rebuild trampoline + pack with flags, no flash:
HOLD=1 MODULES=drm ./boot/pack-vendor-boot.sh
# inspect the produced image + cmdline:
ls -lh boot/out/vendor_boot-test.img
```
