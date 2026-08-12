#!/usr/bin/env bash
# Install / verify host tools for AginxOS phone builds.
set -euo pipefail

echo "==> rustup targets"
rustup target add aarch64-linux-android
rustup target add aarch64-unknown-linux-musl

echo "==> cargo-zigbuild (Linux musl via zig)"
if ! command -v cargo-zigbuild >/dev/null 2>&1; then
  if command -v brew >/dev/null 2>&1; then
    brew install cargo-zigbuild zig
  else
    cargo install cargo-zigbuild
  fi
fi
command -v zig >/dev/null 2>&1 || {
  echo "zig is required for aarch64-unknown-linux-musl on macOS" >&2
  exit 1
}

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck disable=SC1091
source "${ROOT}/scripts/env-cross.sh"

echo "==> probe build: aarch64-linux-android"
cargo build -p aginxos-probe --release --target aarch64-linux-android

echo "==> probe build: aarch64-unknown-linux-musl (zig)"
cargo zigbuild -p aginxos-probe --release --target aarch64-unknown-linux-musl

echo
echo "OK. Binaries:"
file "${ROOT}/target/aarch64-linux-android/release/aginxos-probe"
file "${ROOT}/target/aarch64-unknown-linux-musl/release/aginxos-probe"
echo
echo "Next: ./scripts/push-probe-android.sh"
