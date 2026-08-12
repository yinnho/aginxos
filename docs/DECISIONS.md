# AginxOS — locked decisions

Status: **accepted** (2026-08-12). Change only with an explicit superseding note.

## 1. Bring-up / recovery strategy

**Choice: A — full experimental disk.**

- Pixel 5 is a dedicated experiment unit; wipe and reflash are acceptable.
- Keep a local **factory image** for `flash-all` recovery.
- Prefer `fastboot boot` while iterating boot images; once rootfs install starts, overwriting Android partitions is fine.

## 2. MVP definition

Demo is successful when all of the following work on device:

1. Boots into an **AginxOS rootfs** (not Android userspace as the primary UI).
2. **Touch** hits a on-screen control (minimum: one button).
3. **Network**: Wi‑Fi associate + outbound connectivity (`curl` or equivalent).

Out of MVP (later): cellular voice/SMS, camera, fingerprint, GPU polish, full desktop shell.

## 3. Product role

**AginxOS is the host OS** on the phone.

- Owns boot path into Linux userspace, session, input/display policy, netd, and (later) telephony front-end.
- Aginx protocol stack / agents / apps run **on top of** AginxOS; they are not the OS itself.
- Sibling projects (`aginx`, `aginx-controller`, `AginxBrain`, …) integrate as userspace clients/services, not as replacements for the kernel/userspace host.

## 4. UI stack

| Phase | Approach |
|-------|----------|
| Early | DRM/KMS fullscreen + direct input (`evdev`) |
| Later | Optional Wayland (e.g. Smithay) or other compositor |

Do not block bring-up on a full desktop environment.

## 5. Development hosts

| Work | Preferred host |
|------|----------------|
| Rust crates, adb push, docs | macOS or Linux |
| Kernel, boot.img, initramfs, rootfs image builds | **Linux** (VM or bare metal) |

Convention: treat Linux as the authority for anything that packs a bootable phone image.

## 6. License & firmware

- Project license: **MIT**; repo is **public** (`yinnho/aginxos`).
- **Do not commit vendor firmware blobs** or redistribute proprietary radio/Wi‑Fi images in-tree.
- Scripts may document **how to extract** firmware from a factory/vendor image the developer already has; extracted files stay local / gitignored.

## Naming (unchanged)

| Use | Form |
|-----|------|
| Product | AginxOS |
| Crates / paths | `aginxos-*` |
| Env | `AGINXOS_*` |
| Runtime | `/run/aginxos/`, `/var/log/aginxos-*` |
| GitHub | https://github.com/yinnho/aginxos |
