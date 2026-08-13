#!/usr/bin/env bash
# Build patched vendor_boot, flash it, reboot. Optional HOLD=1.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "${ROOT}"
export PATH="/opt/homebrew/bin:$PATH"
HOLD="${HOLD:-0}"

echo "==> build musl binaries"
cargo zigbuild -p aginxos-init -p aginxos-probe --release --target aarch64-unknown-linux-musl
cp -f target/aarch64-unknown-linux-musl/release/aginxos-init boot/initramfs/aginxos-init
cp -f target/aarch64-unknown-linux-musl/release/aginxos-probe boot/initramfs/aginxos-probe
chmod 755 boot/initramfs/aginxos-init boot/initramfs/aginxos-probe

HOLD="${HOLD}" ./boot/pack-vendor-boot.sh

echo "==> device"
adb devices -l || true
if adb devices 2>/dev/null | grep -qE $'\tdevice$'; then
  adb reboot bootloader
fi
for i in $(seq 1 40); do
  fastboot devices 2>/dev/null | grep -q fastboot && break
  sleep 2
done
fastboot devices -l
fastboot getvar product 2>&1

echo "==> flash patched vendor_boot (NOT permanent forever — restore with stock)"
fastboot flash vendor_boot boot/out/vendor_boot-test.img
echo "==> reboot"
fastboot reboot

if [[ "${HOLD}" == "1" ]]; then
  echo "HOLD=1: expect NOT to return to Android; look for color splash."
  echo "Leave: long-press power → fastboot → ./scripts/restore-vendor-boot.sh"
else
  echo "Expect color splash then Android. Waiting for adb..."
  for i in $(seq 1 60); do
    if adb devices 2>/dev/null | grep -qE $'\tdevice$'; then
      echo ONLINE
      adb devices -l
      exit 0
    fi
    sleep 3
  done
fi
