#!/usr/bin/env bash
# Build the AginxOS ext4 rootfs image for the userdata partition (M2).
#
# Assembles a tree from: the unpacked vendor ramdisk (/system + Android prop
# and SELinux context files adbd needs), boot/rootfs/ (busybox + /etc), and
# the musl release binaries. Then packs it with mke2fs -d.
#
# Flash with:  fastboot flash userdata out/rootfs.img
# Boot needs a vendor_boot packed with ROOTFS=1 (scripts/pack-vendor-boot.sh).
#
# Note: mke2fs -d records the building user's uid (501 on macOS) as owner.
# rcS chowns everything back to 0:0 on first boot — do not "fix" that here.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RAMDISK="${ROOT}/boot/out/vendor-ramdisk-root"
RECIPE="${ROOT}/boot/rootfs"
TARGET="${ROOT}/target/aarch64-unknown-linux-musl/release"
TREE="${TREE:-/tmp/aginxos-rootfs}"
IMG="${IMG:-${ROOT}/out/rootfs.img}"
SIZE="${SIZE:-512m}"

test -x "${RAMDISK}/system/bin/adbd" || { echo "missing ${RAMDISK} — run boot/unpack-boot.sh first" >&2; exit 1; }
test -x "${RECIPE}/busybox" || { echo "missing ${RECIPE}/busybox" >&2; exit 1; }
test -x "${TARGET}/aginxos-init" || { echo "missing musl binaries — run scripts/build-phone.sh musl first" >&2; exit 1; }
MKE2FS="$(command -v mke2fs || true)"
test -z "${MKE2FS}" && MKE2FS=/opt/homebrew/bin/mke2fs
test -x "${MKE2FS}" || { echo "mke2fs not found (android-platform-tools provides it)" >&2; exit 1; }

rm -rf "${TREE}"
mkdir -p "${TREE}"

# Mountpoints (and /var/log — the only place boot evidence survives; the
# kernel has no pstore, so /var/adbd.log is our cross-boot record).
mkdir -p "${TREE}"/{dev,proc,sys,etc,home,media,mnt,opt,root,run,srv,tmp,var/log}

# Android pieces: /system (adbd + linker config + lib64) and the root-level
# property/SELinux files adbd reads at startup.
cp -R "${RAMDISK}/system" "${TREE}/system"
for f in default.prop prop.default *_contexts; do
  cp "${RAMDISK}"/${f} "${TREE}/" 2>/dev/null || true
done

# Our pieces.
mkdir -p "${TREE}/bin" "${TREE}/sbin" "${TREE}/aginxos"
cp "${RECIPE}/busybox" "${TREE}/bin/busybox"
cp -R "${RECIPE}/etc/." "${TREE}/etc/"
chmod 755 "${TREE}/etc/init.d/rcS" "${TREE}/etc/init.d/adbd"
cp "${TARGET}/aginxos-init" "${TARGET}/aginxos-agent" "${TREE}/aginxos/"

# Curated applet symlinks — enough for init and debugging; rcS runs
# `busybox --install -s /bin` to fill in the full set on first boot.
APPLETS="[ awk blkid cat chmod chown clear cp cut date dd df dmesg echo env \
expr false fdisk find free getty grep gunzip gzip head hostname id insmod ip \
kill less ln ls lsmod mkdir mknod more mount mv netcat netstat nice passwd \
pidof ping printf ps renice rm rmdir route sed setsid sh sleep sort \
start-stop-daemon stat su switch_root sync tail tar telnet test top touch tr \
true umount uname uniq uptime vi wc wget which whoami xargs zcat"
for a in ${APPLETS}; do ln -sf busybox "${TREE}/bin/${a}"; done
ln -sf ../bin/busybox "${TREE}/sbin/init"
ln -sf ../bin/busybox "${TREE}/sbin/reboot"
ln -sf ../bin/busybox "${TREE}/sbin/poweroff"
ln -sf ../bin/busybox "${TREE}/sbin/ifconfig"

mkdir -p "${ROOT}/out"
"${MKE2FS}" -t ext4 -b 4096 -F -d "${TREE}" "${IMG}" "${SIZE}"
echo "built ${IMG} ($(du -h "${IMG}" | cut -f1)) from ${TREE}"
