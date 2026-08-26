# AginxOS — Agent Guide

Phone host OS: Linux kernel + Rust userspace on a Google Pixel 5 (redfin,
SM7250), booting through our own bootloader path instead of Android userspace.
All hardware work happens on one real device. Every hard rule below exists
because a previous experiment proved it — see `docs/HARDWARE.md` for the receipts.

## Task Guides

Deeper instructions for specific kinds of work live in `agents/skills/`. Read the
matching guide before starting:

- [`agents/skills/boot-experiments.md`](agents/skills/boot-experiments.md) - flashing a test image, verifying behavior on device, recovering from bootloop
- [`agents/skills/vendor-boot.md`](agents/skills/vendor-boot.md) - creating or changing vendor_boot patches under `boot/`

## Documentation Layout

- `docs/DECISIONS.md` — locked decisions (bootloader strategy, MVP scope, license).
  Change only with an explicit superseding note dated later than the original.
- `docs/HARDWARE.md` — the device experiment log. Append observed results only;
  never promote an *expected* result to a recorded one. "Confirm on device" is
  not done until someone saw it.
- `README.md` — overview, crate table, build entry points.
- `legacy/` — archived research from the earlier from-scratch kernel attempt.
  Read-only context. Do not build on it without an explicit decision.

## Ground Rules

- **Compile success ≠ bring-up success.** A change is done when its behavior was
  observed on device and logged in `docs/HARDWARE.md`, not when it builds.
- Do not invent hardware state. Only nodes and paths that appear in probe output,
  kmsg, or `docs/HARDWARE.md` exist. If a node is unprobed, say "not probed", do
  not assume it.
- Do not commit vendor firmware blobs (DECISIONS §7). Extracted firmware stays
  local and gitignored. `.factory/` holds the developer's local factory image for
  `flash-all` recovery — never commit or redistribute it.
- Authoritative sources live in exactly one place. Cmdline/header parameters come
  from `boot/out/vendor_boot_unpack/info.txt`; the feature-flag list lives in
  `agents/skills/vendor-boot.md`; experiment history lives only in
  `docs/HARDWARE.md`. Do not fork copies of any of these.

## Hosts & Toolchains

| Work | Where |
|------|-------|
| Rust crates, `adb push`, docs | macOS or Linux |
| Kernel, boot/vendor_boot images, rootfs | prefer Linux (DECISIONS §6); current pack scripts have been run on macOS |

Two targets, never mix them up:

- `aarch64-linux-android` (NDK) — probe/agent pushed onto **stock Android** via adb.
- `aarch64-unknown-linux-musl` (zig / cargo-zigbuild) — binaries inside our
  initramfs. Must be fully static; the trampoline additionally needs
  `zig cc -target aarch64-linux-musl -static`.

## Scripts

Use the existing wrappers instead of raw commands:

- `./scripts/build-phone.sh musl` — build Rust crates for the initramfs target
- `./boot/fetch-tools.sh` → `./boot/unpack-boot.sh <img>` → `./boot/pack-boot.sh` — boot.img pipeline
- `./scripts/pack-vendor-boot.sh` (env flags `HOLD/SPLASH/MODULES/MODULES_FULL`) — build patched vendor_boot
- `./scripts/flash-early-splash.sh` — pack + flash + watch + auto-recover
- `./scripts/restore-vendor-boot.sh` — back to stock vendor_boot

Shell style: `set -euo pipefail`, `#!/usr/bin/env bash`, quote paths with spaces
rather than escaping them.

## Device Safety

- `boot/stock-boot.img` and `boot/stock-vendor_boot.img` are the known-good
  restore points. Never overwrite them with test artifacts.
- Before any destructive fastboot command, confirm the attached device is the
  experiment unit (serial in `docs/HARDWARE.md`).
- End every device session in a known state: stock `vendor_boot` flashed back,
  or an explicit note in `docs/HARDWARE.md` saying what state the device is in.

## Git

- Atomic commits: one coherent change or one experiment result, not both mixed.
- Commit messages succinct, imperative.
- A commit that changes boot behavior should be accompanied by (or follow within
  the same session) a `docs/HARDWARE.md` entry recording the observed result.
