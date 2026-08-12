# Boot image tools (Pixel 5 / redfin)

AginxOS reuses the **stock or Lineage kernel** inside an Android `boot.img`, and swaps the ramdisk / init for bring-up.

## Layout

```text
boot/
  README.md
  unpack-boot.sh      # split boot.img → unpack/
  pack-boot.sh        # build boot-test.img from unpack/ + initramfs
  initramfs/          # minimal ramdisk sources
  fetch-tools.sh      # download mkbootimg/unpack_bootimg helpers
  out/                # gitignored build products
  unpack/             # gitignored working tree from a real boot.img
```

## Prerequisites

| Tool | Role |
|------|------|
| `unpack_bootimg` / `mkbootimg` | AOSP scripts (Python) — preferred |
| or `magiskboot` | unpack/repack alternative |
| stock `boot.img` | from factory image or `adb` dump |

**Kernel/boot packing is most reliable on Linux** (see `docs/DECISIONS.md`). macOS can unpack/repack if Python tools are present; always verify with `fastboot boot` before `flash`.

## Workflow

```bash
# 1) Put a redfin boot.img somewhere, e.g. boot/stock-boot.img
./boot/fetch-tools.sh
./boot/unpack-boot.sh boot/stock-boot.img

# 2) Inspect
cat boot/unpack/info.txt
ls boot/unpack/

# 3) Pack test image (stock kernel + AginxOS initramfs)
./boot/pack-boot.sh
# → boot/out/boot-test.img

# 4) Temporary boot (does not flash)
adb reboot bootloader
fastboot boot boot/out/boot-test.img
```

## Safety

- Prefer **`fastboot boot`**, not `fastboot flash boot`, until the image is known good.
- Device policy is experimental-wipe, but recovery still needs a local **factory image**.
