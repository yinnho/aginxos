#!/usr/bin/env bash
# Rebuild boot/rootfs/busybox — static aarch64 musl, unicode-complete.
#
# Why this exists (M40, 2026-09-03): the previous build had
# CONFIG_UNICODE_WIDE_WCHARS off, so busybox ash's lineeditor substituted
# every wide char (all CJK) with CONFIG_SUBST_WCHAR='?'. The interactive
# shell could not display Chinese at all. This script is the reproducible
# recipe for the fixed binary (md5 2d9909b329f44d7509ed1a048a905c63).
#
# Requirements: zig (cc + ar), curl, GNU/bSD tar, ~10 min. The GPL source
# tree lives in /tmp only — never commit it (AGENTS.md vendor rule).
#
# Usage:
#   ./scripts/build-busybox.sh          # build in /tmp, copy into boot/rootfs
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BB_VER="1.36.1"
WORK="/tmp/busybox-${BB_VER}"
TARBALL="/tmp/busybox-${BB_VER}.tar.bz2"
OUT="${ROOT}/boot/rootfs/busybox"

[ -x "$(command -v zig)" ] || { echo "zig not found in PATH" >&2; exit 1; }

cd /tmp
[ -f "${TARBALL}" ] || curl -sL --max-time 300 -o "${TARBALL}" \
  "https://busybox.net/downloads/busybox-${BB_VER}.tar.bz2"
if [ ! -d "${WORK}" ]; then
  tar xf "${TARBALL}"
else
  echo "==> reusing ${WORK} (delete it for a from-scratch build)"
fi
cd "${WORK}"

# --- config: defconfig plus the five deltas that matter ---
make defconfig >/dev/null
# static single binary (musl via zig; no glibc gc-sections hazard)
sed -i '' -e 's/^# CONFIG_STATIC is not set/CONFIG_STATIC=y/' .config
# wide+combining wchar support = the fix: lineeditor keeps wcwidth==2 chars
# instead of substituting CONFIG_SUBST_WCHAR ('?')
sed -i '' -e 's/^# CONFIG_UNICODE_WIDE_WCHARS is not set/CONFIG_UNICODE_WIDE_WCHARS=y/' .config
sed -i '' -e 's/^# CONFIG_UNICODE_COMBINING_WCHARS is not set/CONFIG_UNICODE_COMBINING_WCHARS=y/' .config
# defconfig only guarantees BMP up to U+02FF; CJK lives up to U+FFFF
sed -i '' -e 's/^CONFIG_LAST_SUPPORTED_WCHAR=767/CONFIG_LAST_SUPPORTED_WCHAR=65534/' .config
# tc: TCA_CBQ_* undeclared under zig's musl headers; nothing on the
# device uses tc (verified during M40) — drop the applet
sed -i '' -e 's/^CONFIG_TC=y/# CONFIG_TC is not set/' .config
sed -i '' -e '/^CONFIG_FEATURE_TC_INGRESS=y/d' .config
yes '' | make oldconfig >/dev/null 2>&1 || true

# --- zig ld rejects three informational linker flags busybox passes ---
#   INFO_OPTS emits "-Wl,--warn-common -Wl,-Map,$EXE.map -Wl,--verbose";
#   the individual-applet link adds another bare -Wl,--warn-common.
sed -i '' \
  -e 's|echo "-Wl,--warn-common -Wl,-Map,\$EXE.map -Wl,--verbose"|echo ""|' \
  -e 's|^\(\s*\)-Wl,--warn-common \\\|\1 \\|' \
  scripts/trylink

echo "== config check =="
grep -E '^(CONFIG_STATIC|CONFIG_UNICODE_WIDE_WCHARS|CONFIG_UNICODE_COMBINING_WCHARS|CONFIG_LAST_SUPPORTED_WCHAR|CONFIG_TC)=' .config || true

# --- build (final macOS-host strip fails harmlessly; ship unstripped) ---
make -j"$(sysctl -n hw.ncpu 2>/dev/null || echo 8)" \
  CC="zig cc -target aarch64-linux-musl" AR="zig ar" 2>&1 | tail -3

[ -f busybox_unstripped ] || { echo "build produced no busybox_unstripped" >&2; exit 1; }
# zig objcopy --strip-all is unimplemented; the ~1.25MB unstripped binary
# is what shipped and was verified on device
cp busybox_unstripped "${OUT}"
file "${OUT}"
md5 "${OUT}" || md5sum "${OUT}"
echo "==> ${OUT} updated — bake it with scripts/build-rootfs.sh"
