#!/usr/bin/env bash
# Flash early color splash vendor_boot, watch for bootloop, collect dmesg if Android returns.
# Usage:
#   HOLD=0 MODULES=drm ./scripts/flash-early-splash.sh
#   HOLD=1 MODULES=drm ./scripts/flash-early-splash.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "${ROOT}"
export PATH="/opt/homebrew/bin:$PATH"

HOLD="${HOLD:-0}"
SPLASH="${SPLASH:-1}"
MODULES="${MODULES:-drm}"
USBADB="${USBADB:-0}"
USBDIAG="${USBDIAG:-0}"
STOCK="${ROOT}/boot/stock-vendor_boot.img"
IMG="${ROOT}/boot/out/vendor_boot-test.img"

echo "==> build trampoline + pack HOLD=${HOLD} SPLASH=${SPLASH} MODULES=${MODULES} USBADB=${USBADB} USBDIAG=${USBDIAG}"
rm -f boot/initramfs/trampoline
zig cc -target aarch64-linux-musl -static -O2 \
  -o boot/initramfs/trampoline boot/trampoline/trampoline.c
HOLD="${HOLD}" SPLASH="${SPLASH}" MODULES="${MODULES}" USBADB="${USBADB}" USBDIAG="${USBDIAG}" ./boot/pack-vendor-boot.sh

if ! fastboot devices 2>/dev/null | grep -q fastboot; then
  if adb devices 2>/dev/null | grep -qE $'\tdevice$'; then
    echo "==> adb → bootloader"
    adb reboot bootloader
  else
    echo "No device. Enter fastboot: Volume Down + Power, then re-run." >&2
    exit 1
  fi
fi
for i in $(seq 1 40); do
  fastboot devices 2>/dev/null | grep -q fastboot && break
  sleep 2
done
fastboot devices -l
fastboot getvar product 2>&1 | head -3

echo "==> flash ${IMG}"
fastboot flash vendor_boot "${IMG}"
# Reset the slot retry counter: repeated failed boots mark the slot unbootable
# and the device then lands in fastboot on every reboot (HARDWARE.md 2026-08-26).
fastboot set_active a
echo "==> reboot — watch for green/red/blue/yellow (white border)"
fastboot reboot

if [[ "${HOLD}" == "1" ]]; then
  echo "HOLD=1: should NOT reach Android. Look for colors, then long-press to fastboot to restore."
  exit 0
fi

seen_fb=0
for i in $(seq 1 50); do
  sleep 3
  if adb devices 2>/dev/null | grep -qE $'\tdevice$'; then
    sleep 5
    if adb devices 2>/dev/null | grep -qE $'\tdevice$'; then
      echo "ANDROID_OK"
      adb shell uptime || true
      echo "==> full dmesg -> /tmp/aginx-boot-dmesg.txt"
      adb shell 'su -c dmesg' > /tmp/aginx-boot-dmesg.txt 2>/dev/null \
        || adb shell dmesg > /tmp/aginx-boot-dmesg.txt 2>/dev/null || true
      echo "==> trampoline kmsg lines:"
      grep -i aginxos /tmp/aginx-boot-dmesg.txt || true
      adb shell "cat /sys/fs/pstore/console-ramoops-0 2>/dev/null | grep -i aginxos | tail -50" || true
      exit 0
    fi
  fi
  if fastboot devices 2>/dev/null | grep -q fastboot; then
    seen_fb=$((seen_fb + 1))
    echo "fastboot again count=${seen_fb}"
    if [[ "${seen_fb}" -ge 2 ]]; then
      echo "BOOTLOOP → restore stock"
      fastboot flash vendor_boot "${STOCK}"
      fastboot set_active a
      fastboot reboot
      exit 2
    fi
  fi
  echo "[$i] waiting..."
done
echo "timeout waiting for Android"
exit 1
