#!/usr/bin/env bash
# Host test gate (M29) — everything that must pass before a device
# session. Run it before adb pushing anything; run it again before a
# commit that changed Rust or shims.
#
#   ./scripts/check.sh          # host cargo tests + registry lint
#   ./scripts/check.sh lint     # registry lint only (skip cargo test)
#
# Two layers:
#
#   1. cargo test over the host-compatible crates. On Linux that is the
#      whole workspace. agupd / agsvc / aginxos-init do not compile on
#      macOS (prctl, SO_PEERCRED, ioctl c_ulong — Linux-only libc
#      faces), so there the set is explicit. The day one of the three
#      grows a host build, add it to HOST_CRATES.
#
#   2. Registry lint: `ag commands --check` over a scratch copy of the
#      shims. build-rootfs.sh runs the same gate against the assembled
#      TREE (the authoritative one — C tools really compiled); this is
#      the fast pre-bake pass, so every declared ag:exec target that the
#      bare repo cannot provide (C tools live only inside the bake) is
#      stubbed as an empty executable. Presence is what --check lints.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "${ROOT}"

MODE="${1:-all}"

HOST_CRATES=(ag agio agpkg agdone agdl agsign aterm voiced wifi-wizard aginxos-probe aginxos-agent)

# ---- 1. host tests ---------------------------------------------------------
if [ "${MODE}" != "lint" ]; then
  if [ "$(uname)" = "Linux" ]; then
    cargo test --workspace
  else
    echo "==> cargo test (host set: ${HOST_CRATES[*]} — agupd/agsvc/aginxos-init are Linux-only)"
    pargs=()
    for c in "${HOST_CRATES[@]}"; do pargs+=(-p "${c}"); done
    cargo test "${pargs[@]}"
  fi
fi

# ---- 2. registry lint ------------------------------------------------------
cargo build -p ag --release >/dev/null
SCRATCH="$(mktemp -d)"
trap 'rm -rf "${SCRATCH}"' EXIT

mkdir -p "${SCRATCH}/usr/bin"
cp -R boot/rootfs/usr/bin/. "${SCRATCH}/usr/bin/"
# git carries the shims 100644; the router only registers executables
# (the bake chmods them inside TREE — same story there).
chmod 755 "${SCRATCH}"/usr/bin/ag-*

# stub every declared ag:exec target the bare repo cannot provide
for t in $(grep -h '^# ag:exec=' boot/rootfs/usr/bin/ag-* \
             | sed 's/^# ag:exec=//' | sort -u); do
  [ -n "${t}" ] || continue
  if [ ! -e "${SCRATCH}/usr/bin/${t}" ]; then
    printf '#!/bin/sh\n' >"${SCRATCH}/usr/bin/${t}"
    chmod 755 "${SCRATCH}/usr/bin/${t}"
  fi
done

echo "==> ag commands --check (scratch shim tree)"
AG_CMD_PATH="${SCRATCH}/usr/bin" \
AG_GROUPS_DESC="${ROOT}/boot/rootfs/etc/ag/groups.desc" \
  "${ROOT}/target/release/ag" commands --check \
  || { echo "ag commands --check failed — fix the shims" >&2; exit 1; }

echo "check: all green"
