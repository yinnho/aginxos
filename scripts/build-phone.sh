#!/usr/bin/env bash
# Build AginxOS phone binaries.
# Usage:
#   ./scripts/build-phone.sh              # both android + musl probe/agent
#   ./scripts/build-phone.sh android      # Android shell (adb)
#   ./scripts/build-phone.sh musl         # static Linux rootfs
#   ./scripts/build-phone.sh android agent
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "${ROOT}"

TARGET_KIND="${1:-both}"
CRATE="${2:-all}"

# shellcheck disable=SC1091
source "${ROOT}/scripts/env-cross.sh"

build_android() {
  local pkg="$1"
  echo "==> android: ${pkg}"
  cargo build -p "${pkg}" --release --target aarch64-linux-android
}

build_musl() {
  local pkg="$1"
  echo "==> musl: ${pkg}"
  if command -v cargo-zigbuild >/dev/null 2>&1; then
    cargo zigbuild -p "${pkg}" --release --target aarch64-unknown-linux-musl
  else
    cargo build -p "${pkg}" --release --target aarch64-unknown-linux-musl
  fi
}

pkgs=()
case "${CRATE}" in
  all) pkgs=(aginxos-probe aginxos-agent aginxos-init agdl) ;;
  probe|aginxos-probe) pkgs=(aginxos-probe) ;;
  agent|aginxos-agent) pkgs=(aginxos-agent) ;;
  init|aginxos-init) pkgs=(aginxos-init) ;;
  agdl) pkgs=(agdl) ;;
  *) echo "unknown crate: ${CRATE}" >&2; exit 1 ;;
esac

for pkg in "${pkgs[@]}"; do
  case "${TARGET_KIND}" in
    android) build_android "${pkg}" ;;
    musl|linux) build_musl "${pkg}" ;;
    both)
      build_android "${pkg}"
      build_musl "${pkg}"
      ;;
    *) echo "usage: $0 [android|musl|both] [all|probe|agent|init]" >&2; exit 1 ;;
  esac
done

# The musl init is a ramdisk artifact (pack-vendor-boot.sh needs it in place).
if [[ -f "target/aarch64-unknown-linux-musl/release/aginxos-init" ]]; then
  cp -f "target/aarch64-unknown-linux-musl/release/aginxos-init" boot/initramfs/aginxos-init
  echo "==> updated boot/initramfs/aginxos-init"
fi

echo "done."
