#!/usr/bin/env bash
# Build aginxos-probe for Android and run it on a connected device.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "${ROOT}"

# shellcheck disable=SC1091
source "${ROOT}/scripts/env-cross.sh"

TARGET="aarch64-linux-android"
OUT="target/${TARGET}/release/aginxos-probe"
REMOTE="${REMOTE:-/data/local/tmp/aginxos-probe}"

echo "==> building aginxos-probe (${TARGET})"
cargo build -p aginxos-probe --release --target "${TARGET}"

command -v adb >/dev/null 2>&1 || {
  echo "adb not found; install platform-tools" >&2
  exit 1
}

adb wait-for-device
echo "==> adb push → ${REMOTE}"
adb push "${OUT}" "${REMOTE}"
adb shell chmod 755 "${REMOTE}"

echo "==> run"
adb shell "${REMOTE}"
