//! AginxFS — Simple flat filesystem for agent nodes
//!
//! On-disk layout (4096-byte blocks):
//!   Block 0:      Superblock (magic + metadata)
//!   Block 1..N:   Block allocation bitmap
//!   Block N+1..M: File table (fixed 128 entries × 128 bytes)
//!   Block M+1..:  Data blocks

extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;

use crate::blk;

const BLOCK_SIZE: usize = 4096;
const SECTORS_PER_BLOCK: usize = BLOCK_SIZE / 512; // 8
const MAGIC: u32 = 0x41475846; // "AGXF"
const VERSION: u32 = 1;
const MAX_FILES: usize = 128;
const FILE_ENTRY_SIZE: usize = 128;
const FILE_NAME_LEN: usize = 64;

#[cfg(not(feature = "board-redfin"))]
use crate::uart;
#[cfg(feature = "board-redfin")]
use crate::qup_uart as uart;

use crate::platform::UART;

/// Superblock structure (lives in block 0)
#[repr(C)]
struct Superblock {
    magic: u32,
    version: u32,
    block_size: u32,
    total_blocks: u32,
    bitmap_start: u32,      // block index of bitmap
    bitmap_blocks: u32,     // number of bitmap blocks
    filetable_start: u32,   // block index of file table
    filetable_blocks: u32,  // number of file table blocks
    data_start: u32,        // block index where data blocks begin
    max_files: u32,
}

/// File entry in the file table
#[derive(Clone, Copy)]
#[repr(C)]
struct FileEntry {
    name: [u8; FILE_NAME_LEN],
    size: u64,
    start_block: u32,
    blocks_used: u32,
    flags: u32,
    _reserved: [u8; 40],
}

/// Filesystem state (cached in memory)
pub struct FileInfo {
    pub name: [u8; FILE_NAME_LEN],
    pub size: u64,
}

static mut FS_MOUNTED: bool = false;
static mut FS_SUPER: Superblock = Superblock {
    magic: 0, version: 0, block_size: 0, total_blocks: 0,
    bitmap_start: 0, bitmap_blocks: 0,
    filetable_start: 0, filetable_blocks: 0,
    data_start: 0, max_files: 0,
};
static mut FS_BITMAP: Option<Vec<u8>> = None;
static mut FS_FILETABLE: [FileEntry; MAX_FILES] = [FileEntry {
    name: [0u8; FILE_NAME_LEN],
    size: 0,
    start_block: 0,
    blocks_used: 0,
    flags: 0,
    _reserved: [0u8; 40],
}; MAX_FILES];

/// Read a block (8 sectors) from disk
fn read_block(block_idx: u32, buf: &mut [u8; BLOCK_SIZE]) -> bool {
    let sector = block_idx as u64 * SECTORS_PER_BLOCK as u64;
    for i in 0..SECTORS_PER_BLOCK {
        let mut sector_buf = [0u8; 512];
        if !blk::read_block(sector + i as u64, &mut sector_buf) {
            return false;
        }
        buf[i * 512..(i + 1) * 512].copy_from_slice(&sector_buf);
    }
    true
}

/// Write a block (8 sectors) to disk
fn write_block(block_idx: u32, buf: &[u8; BLOCK_SIZE]) -> bool {
    let sector = block_idx as u64 * SECTORS_PER_BLOCK as u64;
    for i in 0..SECTORS_PER_BLOCK {
        let mut sector_buf = [0u8; 512];
        sector_buf.copy_from_slice(&buf[i * 512..(i + 1) * 512]);
        if !blk::write_block(sector + i as u64, &sector_buf) {
            return false;
        }
    }
    true
}

/// Format the filesystem on the block device
pub fn format() -> bool {
    let capacity = blk::capacity();
    if capacity == 0 {
        uart::puts(UART, "[FAIL] FS: no block device\r\n");
        return false;
    }

    let total_sectors = capacity;
    let total_blocks = (total_sectors / SECTORS_PER_BLOCK as u64) as u32;

    // Layout: superblock(1) + bitmap + filetable(4) + data
    let bitmap_blocks = (total_blocks + 8 * BLOCK_SIZE as u32 - 1) / (8 * BLOCK_SIZE as u32);
    let filetable_bytes = MAX_FILES * FILE_ENTRY_SIZE;
    let filetable_blocks = (filetable_bytes + BLOCK_SIZE - 1) / BLOCK_SIZE;

    let bitmap_start = 1u32;
    let filetable_start = bitmap_start + bitmap_blocks as u32;
    let data_start = filetable_start + filetable_blocks as u32;

    let superblock = Superblock {
        magic: MAGIC,
        version: VERSION,
        block_size: BLOCK_SIZE as u32,
        total_blocks,
        bitmap_start,
        bitmap_blocks: bitmap_blocks as u32,
        filetable_start,
        filetable_blocks: filetable_blocks as u32,
        data_start,
        max_files: MAX_FILES as u32,
    };

    // Write superblock (block 0)
    let mut buf = [0u8; BLOCK_SIZE];
    unsafe {
        let src = &superblock as *const Superblock as *const u8;
        let super_size = core::mem::size_of::<Superblock>();
        for i in 0..super_size {
            buf[i] = core::ptr::read_volatile(src.add(i));
        }
    }
    if !write_block(0, &buf) {
        uart::puts(UART, "[FAIL] FS: write superblock\r\n");
        return false;
    }

    // Write bitmap (mark metadata blocks as used)
    let mut bitmap = vec![0u8; (bitmap_blocks as usize) * BLOCK_SIZE];
    for i in 0..data_start as usize {
        bitmap[i / 8] |= 1 << (i % 8);
    }
    for i in 0..bitmap_blocks as u32 {
        let mut block_buf = [0u8; BLOCK_SIZE];
        let offset = i as usize * BLOCK_SIZE;
        let end = core::cmp::min(offset + BLOCK_SIZE, bitmap.len());
        // Use volatile copy to avoid codegen issues
        for j in 0..(end - offset) {
            unsafe { core::ptr::write_volatile(block_buf.as_mut_ptr().add(j), bitmap[offset + j]); }
        }
        if !write_block(bitmap_start + i, &block_buf) {
            uart::puts(UART, "[FAIL] FS: write bitmap\r\n");
            return false;
        }
    }

    // Write empty file table
    for i in 0..filetable_blocks as u32 {
        let block_buf = [0u8; BLOCK_SIZE];
        if !write_block(filetable_start + i, &block_buf) {
            uart::puts(UART, "[FAIL] FS: write filetable\r\n");
            return false;
        }
    }

    // Cache in memory
    unsafe {
        FS_SUPER = superblock;
        FS_BITMAP = Some(bitmap);
        // Zero the filetable using volatile byte-level clear
        let ft_ptr = FS_FILETABLE.as_mut_ptr() as *mut u8;
        let ft_size = MAX_FILES * core::mem::size_of::<FileEntry>();
        for i in 0..ft_size {
            core::ptr::write_volatile(ft_ptr.add(i), 0);
        }
        FS_MOUNTED = true;
    }

    uart::puts(UART, "[OK] FS formatted\r\n");
    true
}

/// Mount the filesystem
pub fn mount() -> bool {
    let mut buf = [0u8; BLOCK_SIZE];
    if !read_block(0, &mut buf) {
        uart::puts(UART, "[FAIL] FS: read superblock\r\n");
        return false;
    }

    let superblock: Superblock = unsafe {
        let mut sb = Superblock {
            magic: 0, version: 0, block_size: 0, total_blocks: 0,
            bitmap_start: 0, bitmap_blocks: 0,
            filetable_start: 0, filetable_blocks: 0,
            data_start: 0, max_files: 0,
        };
        let dst = &mut sb as *mut Superblock as *mut u8;
        let src = buf.as_ptr();
        for i in 0..core::mem::size_of::<Superblock>() {
            core::ptr::write_volatile(dst.add(i), *src.add(i));
        }
        sb
    };

    if superblock.magic != MAGIC {
        uart::puts(UART, "[INFO] FS: no filesystem found\r\n");
        return false;
    }

    if superblock.version != VERSION {
        uart::puts(UART, "[FAIL] FS: version mismatch\r\n");
        return false;
    }

    // Read bitmap
    let bitmap_size = (superblock.bitmap_blocks as usize) * BLOCK_SIZE;
    let mut bitmap = vec![0u8; bitmap_size];
    for i in 0..superblock.bitmap_blocks as u32 {
        if !read_block(superblock.bitmap_start + i, &mut buf) {
            uart::puts(UART, "[FAIL] FS: read bitmap\r\n");
            return false;
        }
        let offset = i as usize * BLOCK_SIZE;
        let end = core::cmp::min(offset + BLOCK_SIZE, bitmap.len());
        for j in 0..(end - offset) {
            bitmap[offset + j] = buf[j];
        }
    }

    // Read file table into static array
    let entries_per_block = BLOCK_SIZE / FILE_ENTRY_SIZE;
    let total_entries = (superblock.filetable_blocks as usize) * entries_per_block;
    let num_files = core::cmp::min(total_entries, MAX_FILES);

    // Clear filetable first (volatile byte-level clear)
    unsafe {
        let ft_ptr = FS_FILETABLE.as_mut_ptr() as *mut u8;
        let ft_size = MAX_FILES * core::mem::size_of::<FileEntry>();
        for i in 0..ft_size {
            core::ptr::write_volatile(ft_ptr.add(i), 0);
        }
    }

    for i in 0..superblock.filetable_blocks as u32 {
        if !read_block(superblock.filetable_start + i, &mut buf) {
            uart::puts(UART, "[FAIL] FS: read filetable\r\n");
            return false;
        }
        for j in 0..entries_per_block {
            let idx = i as usize * entries_per_block + j;
            if idx >= num_files { break; }
            let src_offset = j * FILE_ENTRY_SIZE;
            unsafe {
                let dst = &mut FS_FILETABLE[idx] as *mut FileEntry as *mut u8;
                for k in 0..FILE_ENTRY_SIZE {
                    core::ptr::write_volatile(dst.add(k), buf[src_offset + k]);
                }
            }
        }
    }

    unsafe {
        FS_SUPER = superblock;
        FS_BITMAP = Some(bitmap);
        FS_MOUNTED = true;
    }

    uart::puts(UART, "[OK] FS mounted\r\n");
    true
}

/// Initialize filesystem: mount if exists, otherwise skip
pub fn init() {
    if blk::capacity() == 0 {
        uart::puts(UART, "[SKIP] FS: no block device\r\n");
        return;
    }

    uart::puts(UART, "[..] FS: mounting\r\n");

    if mount() {
        return;
    }

    uart::puts(UART, "[INFO] FS: use mkfs command\r\n");
}

/// List files directly (no Vec allocation)
pub fn list_files() {
    if !is_mounted() { return; }
    unsafe {
        let mut found = false;
        for i in 0..MAX_FILES {
            if FS_FILETABLE[i].name[0] == 0 { continue; }
            found = true;
            let name_len = FS_FILETABLE[i].name.iter().position(|&b| b == 0).unwrap_or(FILE_NAME_LEN);
            crate::uart::puts(UART, "  ");
            crate::uart::puts(UART, core::str::from_utf8(&FS_FILETABLE[i].name[..name_len]).unwrap_or("?"));
            crate::uart::puts(UART, "  ");
            crate::print_hex(UART, FS_FILETABLE[i].size as u32);
            crate::uart::puts(UART, " bytes\r\n");
        }
        if !found {
            crate::uart::puts(UART, "(empty)\r\n");
        }
    }
}

pub fn is_mounted() -> bool {
    unsafe { FS_MOUNTED }
}

/// Flush bitmap and file table to disk
fn flush_metadata() -> bool {
    unsafe {
        let sb = &FS_SUPER;

        // Write bitmap
        if let Some(ref bitmap) = FS_BITMAP {
            for i in 0..sb.bitmap_blocks as u32 {
                let mut buf = [0u8; BLOCK_SIZE];
                let offset = i as usize * BLOCK_SIZE;
                let end = core::cmp::min(offset + BLOCK_SIZE, bitmap.len());
                for j in 0..(end - offset) {
                    core::ptr::write_volatile(buf.as_mut_ptr().add(j), bitmap[offset + j]);
                }
                if !write_block(sb.bitmap_start + i, &buf) {
                    return false;
                }
            }
        }

        // Write file table
        {
            let entries_per_block = BLOCK_SIZE / FILE_ENTRY_SIZE;
            for i in 0..sb.filetable_blocks as u32 {
                let mut buf = [0u8; BLOCK_SIZE];
                for j in 0..entries_per_block {
                    let idx = i as usize * entries_per_block + j;
                    if idx >= MAX_FILES { break; }
                    let dst_offset = j * FILE_ENTRY_SIZE;
                    let src = &FS_FILETABLE[idx] as *const FileEntry as *const u8;
                    for k in 0..FILE_ENTRY_SIZE {
                        buf[dst_offset + k] = core::ptr::read_volatile(src.add(k));
                    }
                }
                if !write_block(sb.filetable_start + i, &buf) {
                    return false;
                }
            }
        }
    }
    true
}

/// Write a file entry using volatile byte-level operations
/// This avoids compiler codegen issues with struct field assignments on QEMU ARM64
unsafe fn write_entry(idx: usize, name: &[u8], size: u64, start_block: u32, blocks_used: u32) {
    let entry_ptr = FS_FILETABLE.as_mut_ptr().add(idx) as *mut u8;
    let entry_size = core::mem::size_of::<FileEntry>();
    // Zero the entire entry first
    for j in 0..entry_size {
        core::ptr::write_volatile(entry_ptr.add(j), 0);
    }
    // Write name
    for (i, &b) in name.iter().enumerate() {
        core::ptr::write_volatile(entry_ptr.add(i), b);
    }
    // Write size (u64 at offset after name[64])
    let size_offset = FILE_NAME_LEN;
    for j in 0..8 {
        core::ptr::write_volatile(entry_ptr.add(size_offset + j), (size >> (j * 8)) as u8);
    }
    // Write start_block (u32 at offset after size)
    let sb_offset = size_offset + 8;
    for j in 0..4 {
        core::ptr::write_volatile(entry_ptr.add(sb_offset + j), (start_block >> (j * 8)) as u8);
    }
    // Write blocks_used (u32 at offset after start_block)
    let bu_offset = sb_offset + 4;
    for j in 0..4 {
        core::ptr::write_volatile(entry_ptr.add(bu_offset + j), (blocks_used >> (j * 8)) as u8);
    }
}

/// Allocate contiguous data blocks
/// Returns start block index, or None if not enough space
fn alloc_blocks(count: u32) -> Option<u32> {
    unsafe {
        let sb = &FS_SUPER;
        let bitmap = FS_BITMAP.as_mut()?;

        let data_start = sb.data_start as usize;
        let total = sb.total_blocks as usize;

        // Linear scan for contiguous free blocks
        let mut run_start = 0usize;
        let mut run_len = 0usize;

        for i in data_start..total {
            let byte_idx = i / 8;
            let bit_idx = i % 8;
            if byte_idx >= bitmap.len() { break; }
            let used = (bitmap[byte_idx] >> bit_idx) & 1;
            if used == 0 {
                if run_len == 0 { run_start = i; }
                run_len += 1;
                if run_len >= count as usize {
                    // Mark blocks as used
                    for j in run_start..run_start + run_len {
                        bitmap[j / 8] |= 1 << (j % 8);
                    }
                    return Some(run_start as u32);
                }
            } else {
                run_len = 0;
            }
        }
        None
    }
}

/// Free contiguous data blocks
fn free_blocks(start: u32, count: u32) {
    unsafe {
        let bitmap = match FS_BITMAP.as_mut() {
            Some(b) => b,
            None => return,
        };
        for i in start..start + count {
            let idx = i as usize;
            if idx / 8 < bitmap.len() {
                bitmap[idx / 8] &= !(1 << (idx % 8));
            }
        }
    }
}

/// Find a file entry by name, returns index
fn find_file(name: &[u8]) -> Option<usize> {
    unsafe {
        for i in 0..MAX_FILES {
            if FS_FILETABLE[i].name[0] == 0 { continue; }
            let entry_name_len = FS_FILETABLE[i].name.iter().position(|&b| b == 0).unwrap_or(FILE_NAME_LEN);
            if eq_bytes(&FS_FILETABLE[i].name[..entry_name_len], name) {
                return Some(i);
            }
        }
        None
    }
}

fn eq_bytes(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() { return false; }
    for i in 0..a.len() { if a[i] != b[i] { return false; } }
    true
}

/// Find a free file table slot
fn find_free_slot() -> Option<usize> {
    unsafe {
        for i in 0..MAX_FILES {
            if FS_FILETABLE[i].name[0] == 0 { return Some(i); }
        }
        None
    }
}

/// Create or overwrite a file
pub fn create(name: &[u8], data: &[u8]) -> bool {
    if !is_mounted() { return false; }
    if name.is_empty() || name.len() >= FILE_NAME_LEN { return false; }

    unsafe {
        // Calculate blocks needed
        let blocks_needed = ((data.len() + BLOCK_SIZE - 1) / BLOCK_SIZE) as u32;
        if blocks_needed == 0 {
            // Empty file
            let idx = find_file(name).or_else(|| find_free_slot()).unwrap_or(MAX_FILES);
            if idx >= MAX_FILES { return false; }
            if FS_FILETABLE[idx].blocks_used > 0 {
                free_blocks(FS_FILETABLE[idx].start_block, FS_FILETABLE[idx].blocks_used);
            }
            write_entry(idx, name, 0, 0, 0);
            return flush_metadata();
        }

        // Allocate new blocks
        let new_start = match alloc_blocks(blocks_needed) {
            Some(s) => s,
            None => {
                uart::puts(UART, "[FAIL] FS: no space\r\n");
                return false;
            }
        };

        // Write data to blocks
        for i in 0..blocks_needed as usize {
            let mut buf = [0u8; BLOCK_SIZE];
            let offset = i * BLOCK_SIZE;
            let end = core::cmp::min(offset + BLOCK_SIZE, data.len());
            for j in 0..(end - offset) {
                core::ptr::write_volatile(buf.as_mut_ptr().add(j), data[offset + j]);
            }
            if !write_block(new_start + i as u32, &buf) {
                free_blocks(new_start, blocks_needed);
                return false;
            }
        }

        // Find or create file entry
        let idx = find_file(name).or_else(|| find_free_slot()).unwrap_or(MAX_FILES);
        if idx >= MAX_FILES {
            free_blocks(new_start, blocks_needed);
            return false;
        }

        // Free old blocks if overwriting
        if FS_FILETABLE[idx].blocks_used > 0 {
            free_blocks(FS_FILETABLE[idx].start_block, FS_FILETABLE[idx].blocks_used);
        }

        write_entry(idx, name, data.len() as u64, new_start, blocks_needed);
        flush_metadata()
    }
}

/// Read a file's contents into a buffer
/// Returns the number of bytes read
pub fn read(name: &[u8], buf: &mut [u8]) -> Option<usize> {
    if !is_mounted() { return None; }

    unsafe {
        let idx = find_file(name)?;
        let entry = &FS_FILETABLE[idx];

        let size = entry.size as usize;
        if size == 0 { return Some(0); }

        let read_len = core::cmp::min(size, buf.len());
        let blocks = entry.blocks_used as usize;

        let mut total_read = 0;
        for i in 0..blocks {
            let mut block_buf = [0u8; BLOCK_SIZE];
            if !read_block(entry.start_block + i as u32, &mut block_buf) {
                return None;
            }
            let copy_start = i * BLOCK_SIZE;
            let copy_end = core::cmp::min(copy_start + BLOCK_SIZE, read_len);
            if copy_start >= read_len { break; }
            buf[copy_start..copy_end].copy_from_slice(&block_buf[..copy_end - copy_start]);
            total_read = copy_end;
        }

        Some(total_read)
    }
}

/// Delete a file
pub fn delete(name: &[u8]) -> bool {
    if !is_mounted() { return false; }

    unsafe {
        let idx = match find_file(name) {
            Some(i) => i,
            None => return false,
        };

        // Free blocks
        if FS_FILETABLE[idx].blocks_used > 0 {
            free_blocks(FS_FILETABLE[idx].start_block, FS_FILETABLE[idx].blocks_used);
        }

        // Clear entry using volatile writes
        write_entry(idx, &[], 0, 0, 0);
        flush_metadata()
    }
}

/// List all files
pub fn list() -> Vec<FileInfo> {
    let mut result = Vec::new();
    if !is_mounted() { return result; }

    unsafe {
        for i in 0..MAX_FILES {
            if FS_FILETABLE[i].name[0] == 0 { continue; }
            let mut info = FileInfo {
                name: [0u8; FILE_NAME_LEN],
                size: FS_FILETABLE[i].size,
            };
            info.name.copy_from_slice(&FS_FILETABLE[i].name);
            result.push(info);
        }
    }

    result
}

/// Check if a file exists
#[allow(dead_code)]
pub fn exists(name: &[u8]) -> bool {
    find_file(name).is_some()
}
