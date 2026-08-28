#!/bin/bash
# Configure minimal aarch64 Linux kernel for AginxOS
# All networking drivers built-in (no module loading needed)

set -e
SRCDIR="$1"
cd "$SRCDIR"

# Use our zig-based cross-compiler wrapper
SCRIPTDIR="$(cd "$(dirname "$0")" && pwd)"
export PATH="$SCRIPTDIR/build/bin:$PATH"
echo "Using PATH: $(which aarch64-elf-gcc)"

# Start from defconfig (working baseline for arm64)
make ARCH=arm64 CROSS_COMPILE=aarch64-elf- defconfig

# Use scripts/config to toggle options
# Disable module support entirely — everything built-in
./scripts/config --disable MODULES
./scripts/config --disable MODULE_UNLOAD
./scripts/config --disable MODULE_FORCE_LOAD
./scripts/config --disable MODULE_FORCE_UNLOAD

# Disable RANDSTRUCT (needs GCC plugin)
./scripts/config --disable RANDSTRUCT_FULL
./scripts/config --enable RANDSTRUCT_NONE

# Disable stack protection (may need special compiler support)
./scripts/config --disable STACKPROTECTOR
./scripts/config --disable STACKPROTECTOR_PER_TASK
./scripts/config --disable STACKPROTECTOR_STRONG

# Disable things that need GCC plugins or special toolchain features
./scripts/config --disable GCC_PLUGINS

# Enable virtio built-in
./scripts/config --enable VIRTIO
./scripts/config --enable VIRTIO_MMIO
./scripts/config --enable VIRTIO_PCI
./scripts/config --enable VIRTIO_NET
./scripts/config --enable VIRTIO_BLK
./scripts/config --enable VIRTIO_CONSOLE
./scripts/config --enable VIRTIO_INPUT

# Enable e1000 built-in
./scripts/config --enable E1000
./scripts/config --enable E1000E

# Ensure networking is built-in
./scripts/config --enable NET
./scripts/config --enable INET
./scripts/config --enable PACKET
./scripts/config --enable UNIX
./scripts/config --enable NETDEVICES
./scripts/config --enable ETHERNET

# Ensure essential filesystems
./scripts/config --enable PROC_FS
./scripts/config --enable SYSFS
./scripts/config --enable DEVTMPFS
./scripts/config --enable DEVTMPFS_MOUNT
./scripts/config --enable TMPFS

# Ensure initramfs support
./scripts/config --enable BLK_DEV_INITRD
./scripts/config --enable RD_GZIP

# Serial console (for QEMU virt)
./scripts/config --enable SERIAL_AMBA_PL011
./scripts/config --enable SERIAL_AMBA_PL011_CONSOLE

# Disable unnecessary features to speed up build
./scripts/config --disable SOUND
./scripts/config --disable USB
./scripts/config --disable USB_SUPPORT
./scripts/config --disable HID
./scripts/config --disable I2C
./scripts/config --disable SPI
./scripts/config --disable DRM
./scripts/config --disable FB
./scripts/config --disable INPUT_KEYBOARD
./scripts/config --disable INPUT_MOUSE
./scripts/config --disable INPUT_TOUCHSCREEN
./scripts/config --disable WLAN
./scripts/config --disable WIRELESS
./scripts/config --disable NFC
./scripts/config --disable BT
./scripts/config --disable PHYLIB

# Disable debug info (smaller build)
./scripts/config --disable DEBUG_INFO
./scripts/config --disable GDB_SCRIPTS

# Resolve all dependencies
make ARCH=arm64 CROSS_COMPILE=aarch64-elf- olddefconfig

echo "Kernel config generated."
echo ""
echo "Key networking configs:"
grep -E "CONFIG_(VIRTIO|E1000|NET|MODULES|ETHERNET|FAILOVER)[= ]" .config | sort
