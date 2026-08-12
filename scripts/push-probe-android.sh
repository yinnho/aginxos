#!/usr/bin/env bash
# Build aginxos-probe for aarch64 and push to a running Android userspace on the phone.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

TARGET="${TARGET:-aarch64-unknown-linux-musl}"
OUT="target/${TARGET}/release/aginxos-probe"
REMOTE="${REMOTE:-/data/local/tmp/aginxos-probe}"

echo "==> building aginxos-probe ($TARGET)"
cargo build -p aginxos-probe --release --target "$TARGET"

echo "==> adb push → $REMOTE"
adb push "$OUT" "$REMOTE"
adb shell chmod 755 "$REMOTE"

echo "==> run"
adb shell "$REMOTE"
