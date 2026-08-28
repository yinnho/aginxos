---
name: Pixel 5 Boot Status & Verified Facts
description: All confirmed working/failed configurations, vbmeta fix, LTO requirement, and boot procedure
type: project
---

## Confirmed Working ✅

| Config | Binary | image_size | Date | Notes |
|--------|--------|------------|------|-------|
| FB + MMU + halt (incremental, no LTO) | 32KB | 0xCF98 | 2026-04-11 | First working MMU kernel |
| FB + MMU + halt (LTO) | 24KB | 0xAFB8 | 2026-04-11 | LTO minimal, works |
| FB + MMU + GCC + halt (LTO) | 20KB | — | 2026-04-11 | GCC clock init works |
| FB only (green screen, no MMU) | 27KB | — | 2026-04-11 | Basic framebuffer confirmed |

## Confirmed Failed ❌

| Config | Binary | image_size | Notes |
|--------|--------|------------|-------|
| FB + MMU + GCC + UART + Heap + GIC + Tasks + Shell (LTO) | 50KB | 0x1A458 | Crashes, no output |
| FB + MMU + GCC + UART + Heap (no LTO) | 34KB | 0x11650 | Crashes (was BEFORE vbmeta fix) |
| Full kernel (no LTO) | 186KB | 0x40070 | Crashes |

**Note**: The 34KB and 186KB failures were tested BEFORE vbmeta fix. They might work now with vbmeta disabled. Only the 50KB LTO full kernel was tested AFTER vbmeta fix and still fails.

## Critical: VBMeta Verification

**Root cause of ALL post-factory-restore crashes**: factory restore resets vbmeta to stock (verification enabled). Custom kernels crash immediately with no output.

**Fix (MUST do after factory restore or stock flash):**
```bash
# Create vbmeta with verification disabled (flag 2 = disable verification)
python3 /tmp/avb/avbtool.py make_vbmeta_image \
  --output /tmp/vbmeta-custom.img \
  --algorithm NONE \
  --flag 2 \
  --padding_size 4096

# Flash to both slots
fastboot flash vbmeta /tmp/vbmeta-custom.img
fastboot flash vbmeta_b /tmp/vbmeta-custom.img
```

**Why:** With stock vbmeta (verification enabled), the kernel runs in a restricted environment where memory/hardware access fails silently, causing immediate crash before any output.

**How to apply:** ALWAYS flash custom vbmeta after any operation that restores stock images (factory restore, `fastboot update`, etc.)

## LTO Requirement

**`lto = true` is required** for redfin builds. Without LTO, unused static arrays stay in BSS:
- `frame_alloc::BITMAP`: 16KB (`[u64; 2048]`)
- `task::FD_TABLES`: 20KB
- Total BSS: ~36KB

With LTO, unused code/data is stripped → smaller binary, smaller BSS.

Config: `Cargo.toml` → `[profile.release] lto = true`

## Pending Tests (after vbmeta fix)

These need re-testing NOW with vbmeta disabled:

1. **Full kernel without LTO** (186KB) — was tested before vbmeta fix, might work now
2. **Step-by-step progress kernel** — add GCC → UART → Heap → GIC → Tasks → Shell one at a time to find exact crash point with 50KB LTO full kernel
3. **Frame alloc init** — may crash if memory layout wrong
4. **GIC init** — GIC base addresses need verification on real hardware
5. **Task scheduler** — depends on GIC timer working

## Boot Procedure (Verified Working)

```bash
# 1. Build (with LTO!)
cd /Users/mac8684/Documents/agentos/aginx-os
cargo build --target aarch64-redfin.json --release --features board-redfin

# 2. Binary + LZ4
rust-objcopy --strip-all -O binary \
  target/aarch64-redfin/release/aginx-kernel \
  target/aarch64-redfin/release/aginx-kernel.bin
python3 -c "
import lz4.frame
raw = open('target/aarch64-redfin/release/aginx-kernel.bin','rb').read()
c = lz4.frame.compress(raw, len(raw), store_size=False,
  block_size=lz4.frame.BLOCKSIZE_MAX4MB,
  block_linked=False, content_checksum=True)
open('/tmp/aginx-kernel.lz4','wb').write(c)
"

# 3. Boot image
python3 /tmp/mkbootimg/mkbootimg/mkbootimg.py \
  --kernel /tmp/aginx-kernel.lz4 \
  --ramdisk /tmp/stock_ramdisk \
  --os_version 11.0.0 --os_patch_level 2021-10 \
  --header_version 3 --output /tmp/boot.img

# 4. AVB footer (algorithm NONE)
python3 /tmp/avb/avbtool.py add_hash_footer \
  --image /tmp/boot.img --partition_size 100663296 \
  --partition_name boot --algorithm NONE

# 5. Flash (ensure vbmeta is disabled first!)
fastboot set_active a
fastboot flash boot_a /tmp/boot.img
fastboot reboot
```

## File Locations
- mkbootimg: `/tmp/mkbootimg/mkbootimg/mkbootimg.py`
- avbtool: `/tmp/avb/avbtool.py`
- stock ramdisk: `/tmp/stock_ramdisk`
- factory images: `/tmp/pixel5-restore/redfin-rq3a.211001.001/`

## Key Files Modified
- `kernel/src/main.rs` — redfin path with step-by-step init
- `kernel/src/entry.S` — BSS zeroing (standard, no skip)
- `kernel/linker-redfin.ld` — BSS symbols computed with ADDR()+SIZEOF()
- `kernel/src/frame_alloc.rs` — cfg-gated RAM addresses for redfin
- `kernel/src/gic.rs` — cfg-gated GIC base addresses for redfin
- `Cargo.toml` — `lto = true` required
