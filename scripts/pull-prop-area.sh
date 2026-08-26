#!/usr/bin/env bash
# Pull the property-area files scripts/patch-prop-area.py needs from a
# normally-booted Android on this unit into boot/out/props/ (gitignored —
# device-specific dump; never commit or redistribute).
#
# bionic needs: properties_serial (global serial), property_info (the
# serialized contexts trie), plus the one context area we patch. Missing
# context areas resolve to "property not found" — lazily, per context — so
# a 3-file staging is enough for the rdinit env.
#
# Transfer MUST go through base64: `su -c cat` output passes through a pty
# that turns every 0x0a into 0x0d 0x0a (verified: +1 byte per newline,
# md5 mismatch). base64 output is strip-then-decode, mangling-proof.
# The area files are root-only (dir is --x), hence su.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="${ROOT}/boot/out/props"
FILES=("properties_serial" "property_info" "u:object_r:userdebug_or_eng_prop:s0")

if [[ "$(adb get-state 2>/dev/null || true)" != "device" ]]; then
  echo "error: no authorized adb device — boot normal Android (stock vendor_boot) first" >&2
  exit 1
fi
if ! adb shell su -c true >/dev/null 2>&1; then
  echo "error: no root (su) on this device — property files are root-only" >&2
  exit 1
fi

rm -rf "${DEST}"
mkdir -p "${DEST}"
for f in "${FILES[@]}"; do
  adb shell su -c "base64 < '/dev/__properties__/${f}'" | tr -d '\r\n' | base64 -d >"${DEST}/${f}"
done

echo "pulled ${#FILES[@]} files into ${DEST}:"
ls -la "${DEST}"
adb shell su -c "md5sum $(printf "/dev/__properties__/%s " "${FILES[@]}")" | tr -d '\r' >"${DEST}/.md5-device"
(cd "${DEST}" && md5 -q "${FILES[@]}" | paste -d' ' - <(printf '%s\n' "${FILES[@]}") >.md5-local)
echo "device md5s:"; cat "${DEST}/.md5-device"
echo "local  md5s:"; cat "${DEST}/.md5-local"
