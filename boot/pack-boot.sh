#!/usr/bin/env bash
# Pack boot/out/boot-test.img from unpacked stock kernel + AginxOS initramfs.
#
# Prerequisites:
#   ./boot/unpack-boot.sh boot/stock-boot.img
#   ./boot/fetch-tools.sh   # unless magiskboot-only flow
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
UNPACK="${ROOT}/boot/unpack"
INITSRC="${ROOT}/boot/initramfs"
WORK="${ROOT}/boot/out/initramfs-root"
OUTDIR="${ROOT}/boot/out"
TOOLS="${ROOT}/boot/tools"
OUTIMG="${OUTDIR}/boot-test.img"

mkdir -p "${OUTDIR}"

if [[ ! -d "${UNPACK}" ]] || [[ -z "$(ls -A "${UNPACK}" 2>/dev/null || true)" ]]; then
  echo "boot/unpack is empty. Run ./boot/unpack-boot.sh <boot.img> first." >&2
  exit 1
fi

KERNEL=""
for cand in "${UNPACK}/kernel" "${UNPACK}/Image" "${UNPACK}/Image.gz"; do
  if [[ -f "${cand}" ]]; then
    KERNEL="${cand}"
    break
  fi
done
if [[ -z "${KERNEL}" ]]; then
  echo "No kernel found in ${UNPACK}" >&2
  ls -la "${UNPACK}" >&2
  exit 1
fi

echo "==> building initramfs from ${INITSRC}"
rm -rf "${WORK}"
mkdir -p "${WORK}"/{bin,dev,proc,sys,sysroot,tmp}

# BusyBox is optional: if not provided, ship a tiny /init shell script only.
if [[ -n "${BUSYBOX:-}" && -f "${BUSYBOX}" ]]; then
  cp "${BUSYBOX}" "${WORK}/bin/busybox"
  chmod 755 "${WORK}/bin/busybox"
  (
    cd "${WORK}/bin"
    for a in sh mount umount mkdir ls cat echo sleep switch_root mknod; do
      ln -sfn busybox "${a}"
    done
  )
elif [[ -f "${INITSRC}/busybox" ]]; then
  cp "${INITSRC}/busybox" "${WORK}/bin/busybox"
  chmod 755 "${WORK}/bin/busybox"
  (
    cd "${WORK}/bin"
    for a in sh mount umount mkdir ls cat echo sleep switch_root mknod; do
      ln -sfn busybox "${a}"
    done
  )
else
  echo "note: no busybox; init will be a static-friendly shell script only"
  echo "      put aarch64 busybox at boot/initramfs/busybox or set BUSYBOX=..."
fi

cp "${INITSRC}/init" "${WORK}/init"
chmod 755 "${WORK}/init"
if [[ -f "${INITSRC}/aginxos-probe" ]]; then
  mkdir -p "${WORK}/bin"
  cp "${INITSRC}/aginxos-probe" "${WORK}/bin/aginxos-probe"
  chmod 755 "${WORK}/bin/aginxos-probe"
fi

RAMDISK_CPIO="${OUTDIR}/initramfs.cpio"
RAMDISK_GZ="${OUTDIR}/initramfs.cpio.gz"
(
  cd "${WORK}"
  # newc cpio; root ownership for kernel consumption
  find . -print0 | cpio --null --create --format=newc 2>/dev/null | gzip -9 >"${RAMDISK_GZ}"
) || {
  # macOS bsdtar fallback
  echo "note: using bsdtar/cpio fallback"
  (
    cd "${WORK}"
    if command -v bsdtar >/dev/null 2>&1; then
      bsdtar --format=newc -cf - . | gzip -9 >"${RAMDISK_GZ}"
    else
      find . | cpio -o -H newc | gzip -9 >"${RAMDISK_GZ}"
    fi
  )
}
cp "${RAMDISK_GZ}" "${RAMDISK_CPIO}.gz" 2>/dev/null || true
echo "==> ramdisk: ${RAMDISK_GZ} ($(wc -c <"${RAMDISK_GZ}") bytes)"

# Parse cmdline / header hints from unpack info if present
CMDLINE="${CMDLINE:-}"
if [[ -z "${CMDLINE}" && -f "${UNPACK}/info.txt" ]]; then
  # unpack_bootimg --format=mkbootimg: one line of flags, possibly shell-quoted
  CMDLINE="$(python3 - <<'PY' "${UNPACK}/info.txt"
import shlex, sys
text = open(sys.argv[1]).read()
args = shlex.split(text)
for i, a in enumerate(args):
    if a == "--cmdline" and i + 1 < len(args):
        print(args[i + 1])
        break
PY
)"
fi
if [[ -z "${CMDLINE}" && -f "${UNPACK}/cmdline" ]]; then
  CMDLINE="$(tr -d '\n' <"${UNPACK}/cmdline")"
fi
if [[ -z "${CMDLINE}" ]]; then
  CMDLINE="console=ttyMSM0,115200n8 androidboot.console=ttyMSM0 androidboot.hardware=redfin"
  echo "warn: using fallback cmdline (replace after inspecting stock boot)"
fi

HEADER_VERSION="${HEADER_VERSION:-3}"
if [[ -f "${UNPACK}/info.txt" ]]; then
  hv="$(python3 - <<'PY' "${UNPACK}/info.txt"
import shlex, sys
args = shlex.split(open(sys.argv[1]).read())
for i, a in enumerate(args):
    if a == "--header_version" and i + 1 < len(args):
        print(args[i + 1])
        break
PY
)"
  [[ -n "${hv}" ]] && HEADER_VERSION="${hv}"
fi

OS_VERSION="${OS_VERSION:-}"
OS_PATCH_LEVEL="${OS_PATCH_LEVEL:-}"
if [[ -f "${UNPACK}/info.txt" ]]; then
  eval "$(python3 - <<'PY' "${UNPACK}/info.txt"
import shlex, sys
args = shlex.split(open(sys.argv[1]).read())
kv = {}
i = 0
while i < len(args):
    a = args[i]
    if a.startswith("--") and i + 1 < len(args) and not args[i+1].startswith("--"):
        kv[a[2:]] = args[i+1]
        i += 2
    else:
        i += 1
for key in ("os_version", "os_patch_level"):
    if key in kv:
        print(f"{key.upper()}={kv[key]!r}")
PY
)"
fi
# Drop placeholders like 0.0.0 / 2000-00 from synthetic or broken unpacks
if [[ "${OS_VERSION}" == "0.0.0" || "${OS_VERSION}" == "0" ]]; then
  OS_VERSION=""
fi
if [[ ! "${OS_PATCH_LEVEL}" =~ ^[0-9]{4}-[0-9]{2}$ ]] || [[ "${OS_PATCH_LEVEL}" == *"-00" ]]; then
  OS_PATCH_LEVEL=""
fi

if [[ -f "${TOOLS}/mkbootimg.py" ]]; then
  echo "==> mkbootimg.py → ${OUTIMG}"
  args=(
    python3 "${TOOLS}/mkbootimg.py"
    --header_version "${HEADER_VERSION}"
    --kernel "${KERNEL}"
    --ramdisk "${RAMDISK_GZ}"
    --cmdline "${CMDLINE}"
    -o "${OUTIMG}"
  )
  # v3+ often needs no base/pagesize; v2 needs them
  if [[ "${HEADER_VERSION}" -lt 3 ]]; then
    args+=(--base 0x00000000 --pagesize 4096)
  fi
  if [[ -n "${OS_VERSION}" ]]; then
    args+=(--os_version "${OS_VERSION}")
  fi
  if [[ -n "${OS_PATCH_LEVEL}" ]]; then
    args+=(--os_patch_level "${OS_PATCH_LEVEL}")
  fi
  # Vendor boot / dtb: if stock unpack has dtb, pass through
  if [[ -f "${UNPACK}/dtb" ]]; then
    args+=(--dtb "${UNPACK}/dtb")
  fi
  "${args[@]}"
elif command -v magiskboot >/dev/null 2>&1; then
  echo "==> magiskboot repack"
  cp "${KERNEL}" "${UNPACK}/kernel"
  cp "${RAMDISK_GZ}" "${UNPACK}/ramdisk.cpio"
  # magiskboot expects uncompressed cpio sometimes — try gzip path via rename
  (
    cd "${UNPACK}"
    # Decompress to ramdisk.cpio if needed
    if file ramdisk.cpio | grep -q gzip; then
      mv ramdisk.cpio ramdisk.cpio.gz
      gzip -dc ramdisk.cpio.gz >ramdisk.cpio
    fi
    magiskboot repack boot.img "${OUTIMG}" 2>/dev/null || magiskboot repack boot.img
    if [[ -f new-boot.img ]]; then
      mv -f new-boot.img "${OUTIMG}"
    fi
  )
else
  echo "No pack tool. Run ./boot/fetch-tools.sh" >&2
  exit 1
fi

ls -la "${OUTIMG}"
echo
echo "Test boot (does not flash):"
echo "  adb reboot bootloader"
echo "  fastboot boot ${OUTIMG}"
