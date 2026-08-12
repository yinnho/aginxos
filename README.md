# AginxOS

[![GitHub](https://img.shields.io/badge/GitHub-yinnho%2Faginxos-black)](https://github.com/yinnho/aginxos)

**AginxOS** is the **phone host OS**: Linux kernel (drivers) + Rust userspace (the system).  
Aginx agents and apps run on top of it.

```text
Pixel bootloader → Linux kernel → AginxOS userspace → Aginx / apps
```

First target: **Google Pixel 5** (`redfin`, Snapdragon 765G) — unlocked, dedicated experiment unit (wipe OK; keep factory image for recovery).

## Route A

We do **not** rewrite Wi‑Fi firmware or the 5G baseband in Rust.

| Layer | Owner |
|-------|--------|
| Bootloader | Stock Pixel ABOOT/ABL |
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
# Host check
cargo build -p aginxos-probe

# Phone (aarch64 musl toolchain required)
rustup target add aarch64-unknown-linux-musl
cargo build -p aginxos-probe --release --target aarch64-unknown-linux-musl
adb push target/aarch64-unknown-linux-musl/release/aginxos-probe /data/local/tmp/
adb shell chmod +x /data/local/tmp/aginxos-probe
adb shell /data/local/tmp/aginxos-probe
```

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
2. Custom `boot.img` (`fastboot boot`, then flash when ready)  
3. Minimal rootfs as primary userspace  
4. DRM + touch button  
5. Wi‑Fi (host driver + locally extracted firmware)  
6. Later: modem control (QMI/MBIM), richer shell  

## Name

- Product: **AginxOS**  
- Crates: `aginxos-*` · Env: `AGINXOS_*` · Runtime: `/run/aginxos/`  
- GitHub: [yinnho/aginxos](https://github.com/yinnho/aginxos)  
