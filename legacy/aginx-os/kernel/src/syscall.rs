//! Syscall dispatcher
//!
//! Handles SVC-based syscalls from EL0 (userspace).
//! Syscall convention:
//!   x8 = syscall number
//!   x0-x5 = arguments
//!   x0 = return value

use crate::platform::UART;

/// Syscall numbers — aligned with aginx-syscall crate
pub const SYS_OPEN: u64 = 0;
pub const SYS_CLOSE: u64 = 1;
pub const SYS_READ: u64 = 2;
pub const SYS_WRITE: u64 = 3;
pub const SYS_MMAP: u64 = 7;
pub const SYS_EXIT: u64 = 10;
pub const SYS_YIELD: u64 = 12;
pub const SYS_RETURN_TO_KERNEL: u64 = 99; // Special: return from inline EL0 test

/// Main syscall handler called from entry.S SVC path
#[no_mangle]
pub extern "C" fn handle_syscall(nr: u64, x0: u64, x1: u64, x2: u64, _x3: u64, _x4: u64, _x5: u64) -> u64 {
    match nr {
        SYS_OPEN  => sys_open(x0, x1, x2),
        SYS_CLOSE => sys_close(x0),
        SYS_READ  => sys_read(x0, x1, x2),
        SYS_WRITE => sys_write(x0, x1, x2),
        SYS_MMAP  => sys_mmap(x0, x1),
        SYS_EXIT  => sys_exit(x0),
        SYS_YIELD => sys_yield(),
        SYS_RETURN_TO_KERNEL => 0,
        _ => {
            crate::uart::puts(UART, "[syscall] unknown ");
            crate::print_dec_u32(UART, nr as u32);
            crate::uart::puts(UART, "\r\n");
            u64::MAX
        }
    }
}

/// sys_write(fd, buf, len) -> bytes written
/// FD 1,2 = UART (stdout/stderr)
/// FD 3+ = file write at current offset
fn sys_write(fd: u64, buf: u64, len: u64) -> u64 {
    let ptr = buf as *const u8;
    let count = len as usize;
    if ptr.is_null() || count > 4096 {
        return u64::MAX;
    }
    let slice = unsafe { core::slice::from_raw_parts(ptr, count) };

    match fd {
        1 | 2 => {
            // stdout/stderr -> UART
            unsafe {
                for i in 0..count {
                    let b = core::ptr::read_volatile(ptr.add(i));
                    crate::uart::putc(UART, b);
                }
            }
            count as u64
        }
        3..=255 => {
            // File write: read current file, append data, rewrite
            let fd_idx = fd as usize - 3; // FD 3 = fd_table index 0
            let fd_ptr = match crate::task::fd_get(fd_idx) {
                Some(p) => p,
                None => return u64::MAX,
            };
            let fd_entry = unsafe { &mut *fd_ptr };

            // Get filename as &[u8]
            let name_len = fd_entry.name.iter().position(|&b| b == 0).unwrap_or(crate::task::FD_NAME_LEN);
            let name = &fd_entry.name[..name_len];

            if name.is_empty() { return u64::MAX; }

            // Read existing file content
            let mut file_buf = [0u8; 4096];
            let existing_len = match crate::fs::read(name, &mut file_buf) {
                Some(n) => n,
                None => {
                    // File doesn't exist yet — create it
                    0
                }
            };

            // Append data at offset
            let write_start = core::cmp::min(fd_entry.offset as usize, existing_len);
            let new_len = core::cmp::max(existing_len, write_start + count);

            if new_len > 4096 { return u64::MAX; } // Buffer too small

            // Build new content: existing + gap + new data
            // For simplicity, just append to existing content
            let mut combined = [0u8; 4096];
            for i in 0..existing_len {
                combined[i] = file_buf[i];
            }
            for i in 0..count {
                if write_start + i < 4096 {
                    combined[write_start + i] = slice[i];
                }
            }

            // Write back
            if !crate::fs::create(name, &combined[..new_len]) {
                return u64::MAX;
            }

            fd_entry.offset = (write_start + count) as u64;
            count as u64
        }
        _ => u64::MAX,
    }
}

/// sys_read(fd, buf, len) -> bytes read
/// FD 0 = stdin (UART, blocking)
/// FD 3+ = file read at current offset
fn sys_read(fd: u64, buf: u64, len: u64) -> u64 {
    let ptr = buf as *mut u8;
    let count = len as usize;
    if ptr.is_null() || count > 4096 {
        return u64::MAX;
    }

    match fd {
        0 => {
            // stdin -> UART read (non-blocking, returns what's available)
            let mut read = 0usize;
            while read < count {
                let c = crate::uart::getc(UART);
                unsafe { core::ptr::write_volatile(ptr.add(read), c); }
                if c == 0 { break; }
                read += 1;
            }
            read as u64
        }
        3..=255 => {
            // File read
            let fd_idx = fd as usize - 3;
            let fd_ptr = match crate::task::fd_get(fd_idx) {
                Some(p) => p,
                None => return u64::MAX,
            };
            let fd_entry = unsafe { &mut *fd_ptr };

            let name_len = fd_entry.name.iter().position(|&b| b == 0).unwrap_or(crate::task::FD_NAME_LEN);
            let name = &fd_entry.name[..name_len];

            if name.is_empty() { return u64::MAX; }

            // Read entire file
            let mut file_buf = [0u8; 4096];
            let file_len = match crate::fs::read(name, &mut file_buf) {
                Some(n) => n,
                None => return u64::MAX,
            };

            // Copy from offset
            let off = fd_entry.offset as usize;
            if off >= file_len {
                return 0; // EOF
            }
            let avail = core::cmp::min(file_len - off, count);
            for i in 0..avail {
                unsafe { core::ptr::write_volatile(ptr.add(i), file_buf[off + i]); }
            }
            fd_entry.offset += avail as u64;
            avail as u64
        }
        _ => u64::MAX,
    }
}

/// sys_open(path, flags, _mode) -> fd (3+ on success, -1 on failure)
/// Opens a file on the filesystem. FD 0-2 are reserved (stdin/stdout/stderr).
/// Returns fd >= 3 on success, u64::MAX on failure.
fn sys_open(path: u64, flags: u64, _mode: u64) -> u64 {
    let path_ptr = path as *const u8;
    if path_ptr.is_null() { return u64::MAX; }

    // Find length of path string (null-terminated)
    let mut path_len = 0usize;
    unsafe {
        while *path_ptr.add(path_len) != 0 && path_len < 256 {
            path_len += 1;
        }
    }
    if path_len == 0 || path_len >= crate::task::FD_NAME_LEN {
        return u64::MAX;
    }
    let path_slice = unsafe { core::slice::from_raw_parts(path_ptr, path_len) };

    // Allocate FD
    let fd_idx = match crate::task::fd_alloc(path_slice, flags as u32) {
        Some(i) => i,
        None => return u64::MAX,
    };

    // If O_WRONLY or O_RDWR and O_CREAT, create the file if it doesn't exist
    let o_wronly = 1u64;
    let o_rdwr = 2u64;
    let o_creat = 1u64 << 2;
    if (flags & o_wronly != 0 || flags & o_rdwr != 0) && flags & o_creat != 0 {
        if !crate::fs::exists(path_slice) {
            // Create empty file
            if !crate::fs::create(path_slice, &[]) {
                let _ = crate::task::fd_free(fd_idx);
                return u64::MAX;
            }
        }
    }

    // FD index 0 -> returned as FD 3 (since 0,1,2 are stdin/stdout/stderr)
    (fd_idx + 3) as u64
}

/// sys_close(fd) -> 0 on success, -1 on failure
fn sys_close(fd: u64) -> u64 {
    match fd {
        0..=2 => 0, // stdin/stdout/stderr can't be closed
        3..=255 => {
            let fd_idx = fd as usize - 3;
            if crate::task::fd_free(fd_idx) { 0 } else { u64::MAX }
        }
        _ => u64::MAX,
    }
}

fn sys_exit(code: u64) -> u64 {
    crate::uart::puts(UART, "[exit] code=");
    crate::print_dec_u32(UART, code as u32);
    crate::uart::puts(UART, "\r\n");
    crate::task::task_exit()
}

fn sys_yield() -> u64 {
    0
}

/// sys_mmap(addr, len) -> mapped address
/// Stub: allocates heap memory (no page table mapping yet)
fn sys_mmap(_addr: u64, len: u64) -> u64 {
    if len == 0 || len > 0x100000 {
        return u64::MAX;
    }
    let size = len as usize;
    let layout = match alloc::alloc::Layout::from_size_align(size, 4096) {
        Ok(l) => l,
        Err(_) => return u64::MAX,
    };
    unsafe {
        let ptr = alloc::alloc::alloc_zeroed(layout);
        if ptr.is_null() { u64::MAX } else { ptr as u64 }
    }
}
