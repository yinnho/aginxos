# AginxOS — locked decisions

Status: **accepted** (2026-08-12). Change only with an explicit superseding note.

## 0. Bootloader (supersedes README “Route A / stock ABL”, 2026-08-13)

**AginxOS writes its own bootloader.** We do not treat Google ABL as the product boot path.

On Pixel 5 the SoC ROM / Qualcomm XBL is signed and not replaceable. First bring-up form:

1. Stock XBL (unavoidable silicon) loads **our** second-stage bootloader (AginxOS BL).
2. AginxOS BL owns early splash (color on screen), kernel + DTB + initramfs load, and jump to Linux.
3. Linux starts **AginxOS Rust userspace** — not Android.

Early color is a bootloader job, not `rdinit` fighting `msm_drm`. Do not flash vendor / vbmeta to fake a splash.

## 1. System form factor (supersedes §0 bootloader-as-product, §2 MVP ordering, part of §3 agent positioning; 2026-08-26)

**AginxOS is a closed appliance OS: Linux kernel + busybox + static Rust binaries. Nothing else.**

Purpose restated (2026-08-26): the product is "one agent system per person" — a device
that boots straight into our Rust agent. The OS is a means to that end, not the deliverable.
Kernel ownership, splash, and bootloader work are optional research, not requirements.

- **No Android userspace, ever.** We do not hand off to Android as the final state; handoff
  remains only an experiment/recovery tool.
- **No package manager, no app installer, no APK concept.** "Cannot install other apps" is
  not a restriction we build — it is what is absent from the rootfs. Ecosystem expansion
  happens through the agent's Unix socket (§4 sibling projects), never through installation.
- **System layer = busybox** (init, sh, getty, ip/udhcpc, mount, syslogd, crond …) plus
  wpa_supplicant for Wi-Fi. Everything else that is "system functionality" (service
  management, network config, OTA, monitoring) is exposed **by the agent**, over its socket.
  Architecture filter for any new component: *is this something busybox already does, or
  does it belong inside the Rust agent?* If neither, justify it in writing before adding it.
- **Userspace = static musl Rust binaries** (`aginxos-agent`, `aginxos-init`, future daemons).
  Single-file, zero-dependency, portable across downstream-kernel Pixel, mainlined devices,
  and Termux alike.
- Rootfs target size: tens of MB, runs from RAM or userdata ext4. `ps` should show only
  processes we can name and justify.

Relationship to the superseded sections:

- **vs §0 (bootloader):** a self-written bootloader is demoted from product
  requirement to optional research. The product boot path is whatever gets our
  kernel+initramfs running — currently the vendor_boot patch + trampoline on top of
  Google ABL/XBL. §0's "do not fake splash via vendor/vbmeta" finding stands.
- **vs §2→§3 renumbered below (MVP):** the demo bar (rootfs boot, touch, Wi-Fi) is
  unchanged; only the attempt order changes (§1 milestone list below). Touch/display
  move to last.
- **vs §3→§4 renumbered below (agent positioning):** the agent remains a userspace
  process above busybox/kernel — it does not replace the OS substrate. But it replaces
  the traditional daemon layer: where the old §3 assigned service management / netd /
  session / monitoring to "the host OS", those responsibilities now belong to the
  agent's socket API. The agent *is* the system's interface, not merely an app on it.

### Milestone order (replaces "boot into AginxOS rootfs → touch → Wi-Fi" ordering)

1. **USB gadget console under our pid1** (configfs: acm / ffs.adb / rndis). This unblocks
   all interactive debugging and is the single gating milestone. Rationale: months of blind
   bootloop experiments happened because there was no shell; USB drivers in the downstream
   kernel are known-good (stock adb works); only the gadget configuration was missing.
2. Minimal rootfs on userdata + `switch_root` from a slim initramfs.
3. Network (RNDIS via host, then Wi-Fi with locally extracted firmware per §7).
4. `aginxos-agent` resident = **headless MVP**: boot-to-agent appliance without display.
5. Display/touch (DRM direct paint, evdev input) — enhancement phase, must not block 1–4.

The old §3's original MVP list (rootfs boot, touch button, Wi-Fi) remains the eventual
demo bar; the ordering above supersedes the attempt order, not the goal.

## 2. Bring-up / recovery strategy

**Choice: A — full experimental disk.**

- Pixel 5 is a dedicated experiment unit; wipe and reflash are acceptable.
- Keep a local **factory image** for `flash-all` recovery.
- Prefer `fastboot boot` while iterating boot images; once rootfs install starts, overwriting Android partitions is fine.

## 3. MVP definition

Demo is successful when all of the following work on device:

1. Boots into an **AginxOS rootfs** (not Android userspace as the primary UI).
2. **Touch** hits a on-screen control (minimum: one button).
3. **Network**: Wi‑Fi associate + outbound connectivity (`curl` or equivalent).

Out of MVP (later): cellular voice/SMS, camera, fingerprint, GPU polish, full desktop shell.

## 4. Product role

**AginxOS is the host OS** on the phone.

- Owns boot path into Linux userspace, session, input/display policy, netd, and (later) telephony front-end.
- Aginx protocol stack / agents / apps run **on top of** AginxOS; they are not the OS itself. *(Partially superseded by §1: system-level services are exposed by the agent's socket API; see §1 "vs §3→§4".)*
- Sibling projects (`aginx`, `aginx-controller`, `AginxBrain`, …) integrate as userspace clients/services, not as replacements for the kernel/userspace host.

## 5. UI stack

| Phase | Approach |
|-------|----------|
| Early | DRM/KMS fullscreen + direct input (`evdev`) |
| Later | Optional Wayland (e.g. Smithay) or other compositor |

Do not block bring-up on a full desktop environment.

## 6. Development hosts

| Work | Preferred host |
|------|----------------|
| Rust crates, adb push, docs | macOS or Linux |
| Kernel, boot.img, initramfs, rootfs image builds | **Linux** (VM or bare metal) |

Convention: treat Linux as the authority for anything that packs a bootable phone image.

## 7. License & firmware

- Project license: **MIT**; repo is **public** (`yinnho/aginxos`).
- **Do not commit vendor firmware blobs** or redistribute proprietary radio/Wi‑Fi images in-tree.
- Scripts may document **how to extract** firmware from a factory/vendor image the developer already has; extracted files stay local / gitignored.

## 8. Positioning: an OS for agents (refines §1 purpose, §4 role, §5 UI scope; 2026-08-31)

**AginxOS is built for agents, not for humans.** The system's primary user is an
agent; humans are operators and debuggers. Every feature is judged first by "how
does an agent use this", then by "how does a human debug it".

- **arm64 only, always.** One architecture, one target: `aarch64` static musl.
  No x86_64, no second build matrix. Current practice, now locked as principle.
- **Low power is a first-class constraint, not a feature.** Agent nodes are
  7×24 residents that sleep and wake on events; the power budget defines the
  product form. Battery node = phone, powered node = server — same OS, different
  node shapes.
- **Hardware is agent perception.** Audio (M18), camera (M19) are the agent's
  input channels, not human multimedia.
- **Humans reach the system over the network, through channels** — e.g. iLink
  (the WeChat channel adapter in aginx-carrier): human → own phone → channel →
  relay → aginx gateway → ACP → agent on the AginxOS node. Human access is one
  more inbound channel over the protocol surface, never a local UI requirement.
- **The local screen/keyboard is a debug/rescue console.** aterm stays (agent
  applications like codex/grok TUIs hard-require a real pty terminal), but the
  UI line ends there: no further human-facing UI work is in scope.

## Naming (unchanged)


| Use | Form |
|-----|------|
| Product | AginxOS |
| Crates / paths | `aginxos-*` |
| Env | `AGINXOS_*` |
| Runtime | `/run/aginxos/`, `/var/log/aginxos-*` |
| GitHub | https://github.com/yinnho/aginxos |
