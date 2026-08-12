# AginxOS

[![GitHub](https://img.shields.io/badge/GitHub-yinnho%2Faginxos-black)](https://github.com/yinnho/aginxos)

**AginxOS** is a phone-oriented operating system built as:

```text
Linux kernel (drivers)  +  Rust userspace (the system)
```

First target: **Google Pixel 5** (`redfin`, Snapdragon 765G), experimental device, unlocked bootloader.

## Route A

We do **not** rewrite Wi‑Fi firmware or the 5G baseband in Rust.

| Layer | Owner |
|-------|--------|
| Bootloader | Stock Pixel ABOOT/ABL |
| Kernel + drivers | Android/downstream Linux (bring-up), mainline later where useful |
| Userspace / policy / UI / telephony front-end | **AginxOS (Rust)** |

## Workspace

| Crate | Role |
|-------|------|
| `aginxos-probe` | Bring-up probe: kernel version, input/DRM nodes |
| `aginxos-agent` | Early system agent: Unix socket + heartbeat |

```bash
# Host check
cargo build -p aginxos-probe

# Phone (once aarch64 musl toolchain is set up)
rustup target add aarch64-unknown-linux-musl
cargo build -p aginxos-probe --release --target aarch64-unknown-linux-musl
adb push target/aarch64-unknown-linux-musl/release/aginxos-probe /data/local/tmp/
adb shell chmod +x /data/local/tmp/aginxos-probe
adb shell /data/local/tmp/aginxos-probe
```

## Repo layout

```text
aginxos/
  crates/           # Rust userspace
  boot/             # boot.img unpack, initramfs, mkboot scripts
  rootfs/           # rootfs overlays for the phone
  docs/             # hardware notes, bring-up log
  scripts/          # build / push / fastboot helpers
```

## Near-term milestones

1. `aginxos-probe` runs on Pixel 5 (Android shell is fine)
2. Custom `boot.img` via `fastboot boot` (no flash until stable)
3. Minimal rootfs + SSH or shell
4. Touch + display loop
5. Wi‑Fi (host driver + vendor firmware)
6. Modem control path (QMI/MBIM), not a custom baseband

## Name

- Product: **AginxOS**
- Crate / path prefix: `aginxos-*`
- Env vars: `AGINXOS_*`
- Runtime paths: `/run/aginxos/`, `/var/log/aginxos-*`
