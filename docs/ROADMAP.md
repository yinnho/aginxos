# AginxOS bring-up roadmap (2026-08-26)

Companion to [DECISIONS.md](DECISIONS.md) §1 (system form factor). That note
locks *what* the system is; this doc is *how* we get there, in order, with exit
criteria per phase. Update status tables as results land; record observations
in [HARDWARE.md](HARDWARE.md) only.

## Why this ordering

Months were spent on splash/bootloader experiments with no interactive debug
channel — every bootloop was blind. Meanwhile stock adb proves the downstream
kernel's USB drivers work; what our pid1 lacks is only the **gadget
configuration** that Android's `init.usb.rc` normally performs. So the plan
front-loads USB: one milestone that converts every later experiment from
"reflash and pray" into normal embedded development.

## Phase 0 — USB gadget console under our pid1

**Goal: host `adb devices` shows redfin; `adb shell` gives us a shell.**
Nothing else counts as done.

Recon (2026-08-26, see HARDWARE.md "USB gadget recon") settled the approach:

- **acm is absent from this kernel** — no `usb_f_acm.ko`, never referenced. Dead end.
- **ffs (FunctionFS) + configfs are built-in**; stock init.rc's configfs sequence is
  the recipe to mirror.
- **adbd + linker64 + all its bionic libs are inside the vendor ramdisk we already
  patch** — zero extraction, zero cross-builds.
- rndis.ko exists and is self-contained, but macOS ≥12.3 has no RNDIS driver →
  rndis is only a Linux-host option, not the first target.

| Variant | Mechanism | Status |
|---------|-----------|--------|
| A (**first**) | configfs `ffs.adb` + ramdisk's own `/system/bin/adbd` | implemented: `/aginxos/usb-adb` flag + `modules.usb` chain |
| C (later) | configfs `rndis` + udhcpc — Linux hosts only | not started |
| B | configfs `acm` serial | **impossible** — kernel lacks usb_f_acm |

Steps (implemented in `usb_console()` in `boot/trampoline/trampoline.c`):

1. Load `modules.usb` chain (dwc3 controller + PHYs; topological order per
   `modules.dep`; load failures are logged and harmless).
2. Wait for a UDC under `/sys/class/udc/` (expect `a600000.dwc3`).
3. Mount configfs at `/config`, build gadget `g1` (vid `0x18d1`, pid `0xd001` —
   stock recovery IDs), create `functions/ffs.adb`, symlink into `configs/b.1`.
4. Mount functionfs at `/dev/usb-ffs/adb`, fork+exec the ramdisk's adbd, wait
   for `ep1`.
5. Write the UDC name to `g1/UDC` → device enumerates on host.

First test **with HOLD=1** (console + hold, no Android handoff):

```bash
HOLD=1 SPLASH=0 MODULES=0 USBADB=1 ./scripts/flash-early-splash.sh
```

Exit criteria: `adb devices` lists the device; `adb shell dmesg | grep aginxos`
returns trampoline lines. Record in HARDWARE.md.

Risk notes: if no UDC appears, a module in the chain failed with unknown-symbol —
kmsg (`modules ok=N fail=N`) names the list; add the missing dep and retry.
Handoff (`HOLD=0`) with USBADB is untested — the trampoline keeps USB modules
loaded in that mode (skips unload) on the theory that USB ≠ display set, but
verify bootloop-free before trusting it.

## Phase 1 — rootfs lands, iteration becomes seconds

1. Over the Phase 0 shell: format userdata ext4, install minimal rootfs
   (busybox + our musl binaries + wpa_supplicant later).
2. Slim the initramfs to three jobs: bring up USB gadget → mount userdata →
   `switch_root`.
3. Dev loop after this phase: edit files over the wire, restart a process,
   observe. No more image repacks for userspace changes.

Exit criteria: boot reaches a rootfs on userdata; a changed agent binary can be
deployed without rebuilding any image.

## Phase 2 — network + resident agent = headless MVP

1. Networking first via RNDIS/NCM (device online through the host), then Wi-Fi:
   wpa_supplicant (static, ctrl interface only — no D-Bus) + firmware extracted
   locally from vendor image per DECISIONS §7.
2. `aginxos-agent` runs at boot (socket + heartbeat), supervised by busybox init.
3. This is the **headless MVP**: power on → pure AginxOS appliance reachable over
   USB/network. Like an early Raspberry Pi — no screen needed to be a computer.
   "No other apps" holds by construction: no package manager exists in the rootfs.

Exit criteria: unplug/replug power → agent up, socket reachable, outbound
connectivity demonstrated.

## Phase 3 — display/touch (enhancement, must not block)

- Display: DRM direct paint already prototyped in the trampoline; with a shell,
  missing modules can be probed one at a time safely. Later: kmscon or a small
  DRM renderer for real UI.
- Input: evdev direct read (`evdev` crate).
- If redfin display proves stubborn, pivot hardware instead of sinking months:
  mainlined devices (e.g. OnePlus 6 / sdm845-class, cheap second-hand) have
  upstream USB/Wi-Fi/display/touch working today, and our static musl binaries
  run unchanged.

## Status

| Phase | State | Evidence |
|-------|-------|----------|
| 0 | implemented, awaiting on-device test | HARDWARE.md "USB gadget recon"; `usb_console()` in trampoline; `USBADB=1` in pack script |
| 1 | not started | — |
| 2 | partial assets exist | `aginxos-agent` socket+heartbeat code; probe ran on stock Android |
| 3 | research done | trampoline DRM experiments, HARDWARE.md early-splash findings |
