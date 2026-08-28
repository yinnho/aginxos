#!/usr/bin/env python3
"""Inject a file into an AginxFS disk image."""
import struct, sys

BLOCK_SIZE = 4096
MAGIC = 0x41475846  # "AGXF"
MAX_FILES = 128
FILE_NAME_LEN = 64
FILE_ENTRY_SIZE = 128

def main():
    if len(sys.argv) < 4:
        print(f"Usage: {sys.argv[0]} <disk.img> <filename> <input_file>")
        sys.exit(1)

    img_path = sys.argv[1]
    filename = sys.argv[2].encode('ascii')
    input_path = sys.argv[3]

    with open(input_path, 'rb') as f:
        file_data = f.read()

    with open(img_path, 'r+b') as f:
        # Read superblock (block 0)
        f.seek(0)
        sb_data = f.read(BLOCK_SIZE)
        magic, version, block_size, total_blocks = struct.unpack_from('<IIII', sb_data, 0)
        if magic != MAGIC:
            print(f"Error: not an AginxFS image (magic=0x{magic:08x})")
            sys.exit(1)

        bitmap_start = struct.unpack_from('<I', sb_data, 16)[0]
        bitmap_blocks = struct.unpack_from('<I', sb_data, 20)[0]
        filetable_start = struct.unpack_from('<I', sb_data, 24)[0]
        filetable_blocks = struct.unpack_from('<I', sb_data, 28)[0]
        data_start = struct.unpack_from('<I', sb_data, 32)[0]

        print(f"Superblock: total_blocks={total_blocks}, data_start={data_start}")
        print(f"  bitmap: block {bitmap_start} ({bitmap_blocks} blocks)")
        print(f"  filetable: block {filetable_start} ({filetable_blocks} blocks)")

        # Read bitmap
        bitmap_size = bitmap_blocks * BLOCK_SIZE
        f.seek(bitmap_start * BLOCK_SIZE)
        bitmap = bytearray(f.read(bitmap_size))

        # Read filetable
        ft_size = filetable_blocks * BLOCK_SIZE
        f.seek(filetable_start * BLOCK_SIZE)
        ft_data = bytearray(f.read(ft_size))

        # Find free file entry
        free_idx = None
        for i in range(MAX_FILES):
            off = i * FILE_ENTRY_SIZE
            # Check if name is empty (first byte is 0)
            if ft_data[off] == 0:
                free_idx = i
                break

        if free_idx is None:
            print("Error: no free file entries")
            sys.exit(1)

        # Allocate data blocks
        blocks_needed = (len(file_data) + BLOCK_SIZE - 1) // BLOCK_SIZE
        allocated = []
        for blk in range(total_blocks):
            byte_idx = blk // 8
            bit_idx = blk % 8
            if byte_idx < len(bitmap) and (bitmap[byte_idx] & (1 << bit_idx)) == 0:
                allocated.append(blk)
                if len(allocated) >= blocks_needed:
                    break

        if len(allocated) < blocks_needed:
            print("Error: not enough free blocks")
            sys.exit(1)

        # Write file data to allocated blocks
        for i, blk in enumerate(allocated):
            offset = i * BLOCK_SIZE
            chunk = file_data[offset:offset + BLOCK_SIZE]
            chunk = chunk + b'\x00' * (BLOCK_SIZE - len(chunk))
            f.seek(blk * BLOCK_SIZE)
            f.write(chunk)

        # Mark blocks as used in bitmap
        for blk in allocated:
            byte_idx = blk // 8
            bit_idx = blk % 8
            bitmap[byte_idx] |= (1 << bit_idx)

        f.seek(bitmap_start * BLOCK_SIZE)
        f.write(bytes(bitmap))

        # Write file entry
        entry_off = free_idx * FILE_ENTRY_SIZE
        name_bytes = filename + b'\x00' * (FILE_NAME_LEN - len(filename))
        ft_data[entry_off:entry_off + FILE_NAME_LEN] = name_bytes
        struct.pack_into('<Q', ft_data, entry_off + FILE_NAME_LEN, len(file_data))
        struct.pack_into('<I', ft_data, entry_off + FILE_NAME_LEN + 8, allocated[0])
        struct.pack_into('<I', ft_data, entry_off + FILE_NAME_LEN + 12, blocks_needed)
        struct.pack_into('<I', ft_data, entry_off + FILE_NAME_LEN + 16, 0)  # flags

        f.seek(filetable_start * BLOCK_SIZE)
        f.write(bytes(ft_data))

        print(f"Injected '{filename.decode()}' ({len(file_data)} bytes, {blocks_needed} blocks at block {allocated[0]})")

if __name__ == '__main__':
    main()
