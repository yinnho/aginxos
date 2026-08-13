#!/usr/bin/env bash
# Build static init+probe and pack hybrid boot/out/boot-test.img
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "${ROOT}"

if [[ ! -f boot/unpack/kernel ]]; then
  echo "boot/unpack/kernel missing. Unpack stock boot first:" >&2
  echo "  ./boot/unpack-boot.sh boot/stock-boot.img" >&2
  exit 1
fi

# lz4 preferred for redfin-compatible ramdisk
if ! command -v lz4 >/dev/null 2>&1; then
  echo "note: lz4 not found; attempting brew install lz4"
  if command -v brew >/dev/null 2>&1; then
    brew install lz4 || true
  fi
fi

echo "==> musl: aginxos-init + aginxos-probe"
if command -v cargo-zigbuild >/dev/null 2>&1; then
  cargo zigbuild -p aginxos-init -p aginxos-probe --release --target aarch64-unknown-linux-musl
else
  cargo build -p aginxos-init -p aginxos-probe --release --target aarch64-unknown-linux-musl
fi

cp -f target/aarch64-unknown-linux-musl/release/aginxos-init boot/initramfs/aginxos-init
cp -f target/aarch64-unknown-linux-musl/release/aginxos-probe boot/initramfs/aginxos-probe
chmod 755 boot/initramfs/aginxos-init boot/initramfs/aginxos-probe
rm -f boot/initramfs/busybox

export HYBRID="${HYBRID:-1}"
./boot/pack-boot.sh
ls -lh boot/out/boot-test.img
echo
echo "Temporary boot:"
echo "  adb reboot bootloader && fastboot boot boot/out/boot-test.img"
echo "Expect green screen ~4s then Android (hybrid)."
