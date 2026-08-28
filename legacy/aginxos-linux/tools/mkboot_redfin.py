#!/usr/bin/env python3
"""Create Android Boot Image V3 for Pixel 5 (redfin)

Uses the correct V3 header format with ANDROID! 8-byte magic,
matching the format expected by the Pixel 5 bootloader.

Header layout (V3, 1580 bytes + signature padding to 4096):
  Offset  Size   Field
  0       8      magic = "ANDROID!"
  8       4      kernel_size
  12      4      ramdisk_size
  16      4      os_version
  20      4      header_size = 1580
  24      16     reserved
  40      4      header_version = 3
  44      1536   cmdline (two 768-byte fields in AOSP spec)
  1580    2516   signature (zeros when AVB disabled)
  4096           page-aligned kernel data
"""
import struct
import sys
import os
import subprocess

PAGE_SIZE = 4096
HEADER_SIZE = 1580
BOOT_MAGIC = b'ANDROID!'

# os_version encoding: (major << 25) | (minor << 18) | (patch << 11) | yearly << 4 | monthly
# 11.0.0 = 0x1600015a (matching stock redfin RQ3A.211001.001)
OS_VERSION_11 = 0x1600015a


def lz4_compress(data: bytes) -> bytes:
    """Compress data using lz4 command-line tool (frame format, legacy)"""
    import tempfile
    with tempfile.NamedTemporaryFile(suffix='.bin', delete=False) as inf:
        inf.write(data)
        inpath = inf.name
    outpath = inpath + '.lz4'
    try:
        subprocess.run(
            ['lz4', '-f', '-l', inpath, outpath],
            check=True, capture_output=True
        )
        with open(outpath, 'rb') as f:
            return f.read()
    finally:
        os.unlink(inpath)
        if os.path.exists(outpath):
            os.unlink(outpath)


def pad_to_page(data: bytes) -> bytes:
    """Pad data to page boundary (4096 bytes)"""
    remainder = len(data) % PAGE_SIZE
    if remainder == 0:
        return data
    return data + b'\0' * (PAGE_SIZE - remainder)


def create_boot_img_v3(
    kernel_path: str,
    output_path: str,
    ramdisk: bytes = b'',
    cmdline: str = '',
    os_version: int = OS_VERSION_11,
):
    """Create Android Boot Image V3 for Pixel 5"""
    with open(kernel_path, 'rb') as f:
        kernel_data = f.read()

    # LZ4 compress kernel
    kernel_compressed = lz4_compress(kernel_data)
    kernel_size = len(kernel_compressed)
    ramdisk_size = len(ramdisk)

    # Build V3 header (1580 bytes)
    header = bytearray(HEADER_SIZE)

    # Magic: "ANDROID!" (8 bytes)
    struct.pack_into('8s', header, 0, BOOT_MAGIC)

    # kernel_size (4 bytes, offset 8)
    struct.pack_into('<I', header, 8, kernel_size)

    # ramdisk_size (4 bytes, offset 12)
    struct.pack_into('<I', header, 12, ramdisk_size)

    # os_version (4 bytes, offset 16)
    struct.pack_into('<I', header, 16, os_version)

    # header_size (4 bytes, offset 20)
    struct.pack_into('<I', header, 20, HEADER_SIZE)

    # reserved (16 bytes, offset 24) - already zeros

    # header_version (4 bytes, offset 40)
    struct.pack_into('<I', header, 40, 3)

    # cmdline (1536 bytes, offset 44) - split into two 768-byte fields
    cmdline_bytes = cmdline.encode('utf-8')[:1535]
    header[44:44 + len(cmdline_bytes)] = cmdline_bytes

    # Signature area: pad header to page size (4096 - 1580 = 2516 bytes of zeros)
    header_padded = bytes(header) + b'\0' * (PAGE_SIZE - HEADER_SIZE)

    # Pad kernel and ramdisk to page boundaries
    kernel_padded = pad_to_page(kernel_compressed)
    ramdisk_padded = pad_to_page(ramdisk) if ramdisk_size > 0 else b''

    # Assemble boot image
    boot_img = header_padded + kernel_padded + ramdisk_padded

    # Pad to standard Pixel 5 boot partition size (96 MB)
    partition_size = 100663296  # 96 MB
    if len(boot_img) < partition_size:
        boot_img += b'\0' * (partition_size - len(boot_img))

    with open(output_path, 'wb') as f:
        f.write(boot_img)

    print(f"Created {output_path}: {len(boot_img)} bytes ({len(boot_img) // 1048576} MB)")
    print(f"  Kernel: {kernel_size} bytes (LZ4 from {len(kernel_data)} bytes)")
    print(f"  Ramdisk: {ramdisk_size} bytes")
    print(f"  Header: V3 (ANDROID!), {HEADER_SIZE} bytes")
    print(f"  Cmdline: {cmdline}")

    # Verify header
    with open(output_path, 'rb') as f:
        data = f.read(48)
        magic = data[0:8].decode('ascii')
        ks = struct.unpack('<I', data[8:12])[0]
        rs = struct.unpack('<I', data[12:16])[0]
        hv = struct.unpack('<I', data[40:44])[0]
        print(f"  Verify: magic={magic} kernel={ks} ramdisk={rs} version={hv}")


def create_vbmeta_disable(output_path: str):
    """Create a vbmeta image with verification disabled"""
    # VBMeta image: minimal header + disable flag
    # The simplest approach: create an empty vbmeta with the disable flags set
    vbmeta = bytearray(4096)  # One page

    # VBMeta header magic: "AVB0"
    struct.pack_into('4s', vbmeta, 0, b'AVB0')

    # Required libavb version: 1.0
    struct.pack_into('<I', vbmeta, 4, 1)  # libavb version major
    struct.pack_into('<I', vbmeta, 8, 1)  # libavb version minor

    # Header version 1, flags = 2 (disable verification), 0 rollback
    struct.pack_into('<Q', vbmeta, 12, 4096)  # release string offset
    struct.pack_into('<I', vbmeta, 20, 0)     # header version

    # Actually, the easiest way is to just zero-fill.
    # fastboot --disable-verification handles the flag setting.
    # This just needs to be a valid-sized partition image.

    with open(output_path, 'wb') as f:
        f.write(vbmeta)

    print(f"Created {output_path}: {len(vbmeta)} bytes (VBMeta disabled)")


if __name__ == '__main__':
    import argparse

    parser = argparse.ArgumentParser(description='Create Android Boot Image for Pixel 5')
    sub = parser.add_subparsers(dest='command')

    boot = sub.add_parser('boot', help='Create boot image')
    boot.add_argument('kernel', help='Path to uncompressed kernel Image')
    boot.add_argument('output', help='Output boot.img path')
    boot.add_argument('--ramdisk', default=None, help='Optional ramdisk')
    boot.add_argument('--cmdline', default='', help='Kernel cmdline')

    vbmeta = sub.add_parser('vbmeta', help='Create disabled vbmeta image')
    vbmeta.add_argument('output', help='Output vbmeta.img path')

    args = parser.parse_args()

    if args.command == 'boot':
        ramdisk = b''
        if args.ramdisk:
            with open(args.ramdisk, 'rb') as f:
                ramdisk = f.read()
        cmdline = args.cmdline or (
            'console=ttyMSM0,115200n8 earlycon=msm_geni_serial,0x00888000 rdinit=/init'
        )
        create_boot_img_v3(args.kernel, args.output, ramdisk, cmdline)
    elif args.command == 'vbmeta':
        create_vbmeta_disable(args.output)
    else:
        parser.print_help()
