#!/usr/bin/env bash
# Restore stock vendor_boot on redfin.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
STOCK="${ROOT}/boot/stock-vendor_boot.img"
export PATH="/opt/homebrew/bin:$PATH"
test -f "${STOCK}"

if adb devices 2>/dev/null | grep -qE $'\tdevice$'; then
  adb reboot bootloader
fi
for i in $(seq 1 40); do
  fastboot devices 2>/dev/null | grep -q fastboot && break
  sleep 2
done
fastboot flash vendor_boot "${STOCK}"
fastboot reboot
echo "stock vendor_boot restored; waiting adb..."
for i in $(seq 1 40); do
  adb devices 2>/dev/null | grep -qE $'\tdevice$' && adb devices -l && exit 0
  sleep 3
done
