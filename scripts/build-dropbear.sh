#!/usr/bin/env bash
# Build dropbear (sshd + ssh client + keygen) for the phone, static musl
# aarch64 via zig cc (#142 ops channel, 2026-09-04).
#
# Output: .local/dropbear/bin/{dropbear,dbclient,dropbearkey} — build-rootfs.sh
# copies them into the image (required dependency, like build-phone.sh musl).
# Binaries are build artifacts: never committed (.local/ is gitignored).
#
# Traps proven on device 2026-09-04:
# - AR/RANLIB must be zig's (LLVM): macOS BSD ar writes an archive lld
#   cannot pull members from → every libtomcrypt/libtommath symbol comes
#   up undefined at link.
# - Do NOT override CFLAGS: the defaults carry the -I paths into the
#   dropbear tree (dbmalloc.h et al); a bare "-g0 -Os" breaks libtommath.
# - ~2.7 MB per binary with debug_info (zig objcopy --strip-all is
#   "unimplemented" in zig 0.16) — fine against the 2 GB image.
set -euo pipefail

VER=2026.94
URL="https://matt.ucc.asn.au/dropbear/releases/dropbear-${VER}.tar.bz2"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${ROOT}/.local/dropbear"
SRC="${OUT}/src/dropbear-${VER}"

command -v zig >/dev/null || { echo "zig not found — it's the cross toolchain" >&2; exit 1; }

mkdir -p "${OUT}/src"
if [ ! -f "${OUT}/dropbear-${VER}.tar.bz2" ]; then
  curl -sLo "${OUT}/dropbear-${VER}.tar.bz2" "${URL}"
fi
[ -d "${SRC}" ] || tar xf "${OUT}/dropbear-${VER}.tar.bz2" -C "${OUT}/src"

cd "${SRC}"
# Keep a pristine configure; rerun make only (rebuilds are non-reproducible,
# md5s move between runs — compare by behavior, not hash).
[ -f Makefile ] || CC="zig cc -target aarch64-linux-musl" \
  ./configure --host=aarch64-linux-musl --disable-zlib

make PROGRAMS="dropbear dbclient dropbearkey" LDFLAGS="-static" \
  AR='zig ar' RANLIB='zig ranlib' -j"$(sysctl -n hw.ncpu 2>/dev/null || echo 4)"

mkdir -p "${OUT}/bin"
for b in dropbear dbclient dropbearkey; do
  cp "${SRC}/${b}" "${OUT}/bin/${b}"
  chmod 755 "${OUT}/bin/${b}"
done
echo "built: ${OUT}/bin"
md5 -r "${OUT}"/bin/dropbear "${OUT}"/bin/dbclient "${OUT}"/bin/dropbearkey 2>/dev/null \
  || md5sum "${OUT}"/bin/dropbear "${OUT}"/bin/dbclient "${OUT}"/bin/dropbearkey
file "${OUT}"/bin/*
