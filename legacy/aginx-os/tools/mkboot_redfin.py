#!/usr/bin/env python3
"""Create Android Boot Image V3 for Pixel 5 (redfin)
Compresses kernel with LZ4 and wraps in V3 boot image header.
"""
import struct
import sys
import os
import subprocess

BOOT_MAGIC = b'BOOT'
BOOT_MAGIC_SIZE = 4
BOOT_ARGS_SIZE = 512
BOOT_ID_SIZE = 32

def lz4_compress(data):
    """Compress data using lz4 command-line tool"""
    import tempfile
    with tempfile.NamedTemporaryFile(suffix='.bin', delete=False) as inf:
        inf.write(data)
        inpath = inf.name
    outpath = inpath + '.lz4'
    try:
        subprocess.run(['lz4', '-f', '-l', inpath, outpath], check=True, capture_output=True)
        with open(outpath, 'rb') as f:
            compressed = f.read()
        return compressed
    finally:
        os.unlink(inpath)
        if os.path.exists(outpath):
            os.unlink(outpath)

def create_boot_img_v3(kernel_path, output_path, ramdisk=b''):
    """Create Android Boot Image V3 (GKI format)"""
    with open(kernel_path, 'rb') as f:
        kernel_data = f.read()

    # Compress kernel with LZ4
    kernel_compressed = lz4_compress(kernel_data)

    page_size = 4096
    kernel_size = len(kernel_compressed)
    ramdisk_size = len(ramdisk)
    os_version = 0  # GKI

    # V3 header: 1588 bytes
    header = bytearray(1588)
    offset = 0

    # Magic: "BOOT" (4 bytes)
    struct.pack_into('4s', header, offset, BOOT_MAGIC)
    offset += 4

    # kernel_size (4 bytes)
    struct.pack_into('<I', header, offset, kernel_size)
    offset += 4

    # ramdisk_size (4 bytes)
    struct.pack_into('<I', header, offset, ramdisk_size)
    offset += 4

    # os_version (4 bytes)
    struct.pack_into('<I', header, offset, os_version)
    offset += 4

    # header_size (4 bytes)
    struct.pack_into('<I', header, offset, 1588)
    offset += 4

    # Reserved (4 bytes)
    offset += 4

    # header_version = 3 (4 bytes)
    struct.pack_into('<I', header, offset, 3)
    offset += 4

    # cmdline (512 + 512 = 1024 bytes, split into two fields)
    cmdline = b'clk_ignore_unused console=ttyMSM0,115200n8 earlycon=msm_geni_serial,0x00888000 rdinit=/init'
    header[offset:offset+len(cmdline)] = cmdline[:BOOT_ARGS_SIZE]
    offset += BOOT_ARGS_SIZE
    offset += BOOT_ARGS_SIZE  # extra_cmdline (512 bytes)

    # vendor_bootimg + signature skipped — just pad to header size

    # Pad header to page boundary
    header_padded = header + b'\0' * (page_size - len(header) % page_size) if len(header) % page_size != 0 else bytes(header)

    # Pad kernel to page boundary
    kernel_padded = kernel_compressed + b'\0' * (page_size - kernel_size % page_size) if kernel_size % page_size != 0 else kernel_compressed

    # Pad ramdisk to page boundary
    if ramdisk_size > 0:
        ramdisk_padded = ramdisk + b'\0' * (page_size - ramdisk_size % page_size) if ramdisk_size % page_size != 0 else ramdisk
    else:
        ramdisk_padded = b''

    boot_img = header_padded + kernel_padded + ramdisk_padded

    with open(output_path, 'wb') as f:
        f.write(boot_img)

    print(f"Created {output_path}: {len(boot_img)} bytes")
    print(f"  Kernel: {kernel_size} bytes (LZ4 compressed from {len(kernel_data)} bytes)")
    print(f"  Header: V3, {len(header_padded)} bytes")

if __name__ == '__main__':
    kernel = sys.argv[1] if len(sys.argv) > 1 else 'target/aarch64-unknown-none/release/aginx-kernel'
    output = sys.argv[2] if len(sys.argv) > 2 else 'boot.img'
    ramdisk_path = sys.argv[3] if len(sys.argv) > 3 else None

    ramdisk = b''
    if ramdisk_path:
        with open(ramdisk_path, 'rb') as f:
            ramdisk = f.read()

    create_boot_img_v3(kernel, output, ramdisk)
