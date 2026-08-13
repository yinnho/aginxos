#!/usr/bin/env bash
# Build static init+probe and pack boot/out/boot-test.img from stock kernel.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "${ROOT}"

if [[ ! -f boot/unpack/kernel ]]; then
  echo "boot/unpack/kernel missing. Unpack stock boot first:" >&2
  echo "  ./boot/unpack-boot.sh boot/stock-boot.img" >&2
  exit 1
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
# Drop wrong-arch busybox if present
if [[ -f boot/initramfs/busybox ]] && file boot/initramfs/busybox | grep -q '32-bit'; then
  echo "removing 32-bit busybox from initramfs sources"
  rm -f boot/initramfs/busybox
fi

./boot/pack-boot.sh
ls -lh boot/out/boot-test.img
echo
echo "Temporary boot (does not flash partition):"
echo "  adb reboot bootloader"
echo "  fastboot boot boot/out/boot-test.img"
echo "Hold power to leave; stock Android returns after reboot if boot slot unchanged."
