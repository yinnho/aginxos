# AginxOS

[![GitHub](https://img.shields.io/badge/GitHub-yinnho%2Faginxos-black)](https://github.com/yinnho/aginxos)

**AginxOS** is the **phone host OS**: Linux kernel (drivers) + Rust userspace (the system).  
Aginx agents and apps run on top of it.

```text
XBL (signed, SoC) → AginxOS bootloader → Linux kernel → AginxOS userspace → Aginx / apps
```

First target: **Google Pixel 5** (`redfin`, Snapdragon 765G) — unlocked, dedicated experiment unit (wipe OK; keep factory image for recovery).

## Route A

We do **not** rewrite Wi‑Fi firmware or the 5G baseband in Rust.  
We **do** write our own bootloader (early splash + load kernel). Qualcomm XBL stays only because it is fused/signed.

| Layer | Owner |
|-------|--------|
| SoC ROM / XBL | Qualcomm (not replaceable on Pixel 5) |
| Bootloader | **AginxOS BL** (our code; first form: second-stage loaded by XBL/ABL) |
| Kernel + drivers | Android/downstream Linux first; mainline where useful |
| Host userspace | **AginxOS (Rust)** |
| Agents / protocol clients | Aginx ecosystem on top |

## MVP

1. Boot AginxOS rootfs  
2. Touch: one on-screen control  
3. Wi‑Fi + outbound network  

See [docs/DECISIONS.md](docs/DECISIONS.md) for full locked choices (bring-up policy, UI phases, dev hosts, firmware rules).

## Workspace

| Crate | Role |
|-------|------|
| `aginxos-probe` | Bring-up probe: kernel version, input/DRM nodes |
| `aginxos-agent` | Early system agent: Unix socket + heartbeat |

```bash
# One-time: verify NDK + zig cross builds
./scripts/setup-cross.sh

# Host check
cargo build -p aginxos-probe

# On Pixel (Android shell) — uses NDK target
./scripts/push-probe-android.sh

# Static Linux (AginxOS rootfs) — uses cargo-zigbuild
./scripts/build-phone.sh musl

# Boot image (after you have a stock redfin boot.img)
./boot/fetch-tools.sh
./boot/unpack-boot.sh path/to/boot.img
./boot/pack-boot.sh
fastboot boot boot/out/boot-test.img
```

| Target | Toolchain | Use |
|--------|-----------|-----|
| `aarch64-linux-android` | Android NDK | probe/agent via `adb` on stock ROM |
| `aarch64-unknown-linux-musl` | zig / cargo-zigbuild | binaries inside AginxOS rootfs / initramfs |

Kernel / `boot.img` / rootfs image work: prefer **Linux**. Rust + `adb` can stay on macOS.

## Repo layout

```text
aginxos/
  crates/           # Rust userspace
  boot/             # boot.img unpack, initramfs, mkboot scripts
  rootfs/           # rootfs overlays (no vendor firmware blobs)
  docs/             # decisions, hardware log
  scripts/          # build / push / fastboot helpers
```

## Milestones

1. `aginxos-probe` on Pixel 5  
2. **AginxOS bootloader** (second-stage on redfin) + early color splash  
3. Custom `boot.img` / kernel load via our BL  
4. Minimal rootfs as primary userspace  
5. DRM + touch button  
6. Wi‑Fi (host driver + locally extracted firmware)  
7. Later: modem control (QMI/MBIM), richer shell  

## Name

- Product: **AginxOS**  
- Crates: `aginxos-*` · Env: `AGINXOS_*` · Runtime: `/run/aginxos/`  
- GitHub: [yinnho/aginxos](https://github.com/yinnho/aginxos)  
