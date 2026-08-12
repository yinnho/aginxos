#!/usr/bin/env bash
# Unpack an Android boot.img into boot/unpack/
# Usage: ./boot/unpack-boot.sh path/to/boot.img
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
IMG="${1:-}"
OUT="${ROOT}/boot/unpack"
TOOLS="${ROOT}/boot/tools"

if [[ -z "${IMG}" || ! -f "${IMG}" ]]; then
  echo "usage: $0 path/to/boot.img" >&2
  exit 1
fi

mkdir -p "${OUT}"
rm -rf "${OUT:?}/"*

if [[ -x "${TOOLS}/unpack_bootimg.py" ]] || [[ -f "${TOOLS}/unpack_bootimg.py" ]]; then
  echo "==> unpack_bootimg.py → ${OUT}"
  python3 "${TOOLS}/unpack_bootimg.py" --boot_img "${IMG}" --out "${OUT}" --format=mkbootimg | tee "${OUT}/info.txt"
elif command -v magiskboot >/dev/null 2>&1; then
  echo "==> magiskboot unpack"
  cp "${IMG}" "${OUT}/boot.img"
  (
    cd "${OUT}"
    magiskboot unpack boot.img
    {
      echo "tool=magiskboot"
      echo "note=see kernel, ramdisk.cpio, header in this directory"
      ls -la
    } | tee info.txt
  )
else
  echo "No unpack tool. Run: ./boot/fetch-tools.sh" >&2
  echo "Or install magiskboot and re-run." >&2
  exit 1
fi

# Normalize common filenames for pack-boot.sh
if [[ -f "${OUT}/kernel" && ! -f "${OUT}/Image" ]]; then
  ln -sfn kernel "${OUT}/Image" 2>/dev/null || cp "${OUT}/kernel" "${OUT}/Image"
fi

echo
echo "Unpacked. Next:"
echo "  cat ${OUT}/info.txt"
echo "  ./boot/pack-boot.sh"
