//! Aginx OS syscall definitions
//!
//! Syscalls follow Redox OS conventions with some simplifications.

#![no_std]

use bitflags::bitflags;

/// File descriptor type
pub type Fd = usize;

/// Process ID type
pub type Pid = usize;

/// Scheme ID type
pub type SchemeId = usize;

/// Syscall numbers
#[repr(usize)]
pub enum Syscall {
    /// Open a resource: open(path: &str, flags: OFlag) -> Fd
    Open = 0,
    /// Close a file descriptor: close(fd: Fd)
    Close = 1,
    /// Read from fd: read(fd: Fd, buf: &mut [u8]) -> usize
    Read = 2,
    /// Write to fd: write(fd: Fd, buf: &[u8]) -> usize
    Write = 3,
    /// Seek in fd: seek(fd: Fd, pos: SeekFrom) -> isize
    Seek = 4,
    /// Duplicate fd: dup(fd: Fd, buf: &[u8]) -> Fd
    Dup = 5,
    /// Get fd info: fstat(fd: Fd, stat: &mut Stat) -> isize
    Fstat = 6,
    /// Map memory: mmap(addr: usize, size: usize, flags: MapFlags) -> *mut u8
    Mmap = 7,
    /// Unmap memory: munmap(addr: usize, size: usize)
    Munmap = 8,
    /// Create new process: clone(flags: CloneFlags) -> Pid
    Clone = 9,
    /// Exit process: exit(code: i32) -> !
    Exit = 10,
    /// Wait for process: wait(pid: Pid, status: &mut i32) -> Pid
    Wait = 11,
    /// Yield to scheduler: yield()
    Yield = 12,
    /// Send message: send(pid: Pid, buf: &[u8]) -> isize
    Send = 13,
    /// Receive message: recv(pid: Pid, buf: &mut [u8]) -> isize
    Recv = 14,
    /// Register scheme: scheme(name: &str) -> SchemeId
    Scheme = 15,
}

bitflags! {
    /// Open flags
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct OFlag: usize {
        const O_RDONLY = 0;
        const O_WRONLY = 1;
        const O_RDWR = 2;
        const O_CREAT = 1 << 2;
        const O_EXCL = 1 << 3;
        const O_TRUNC = 1 << 4;
        const O_APPEND = 1 << 5;
        const O_NONBLOCK = 1 << 6;
        const O_DIRECTORY = 1 << 7;
        const O_CLOEXEC = 1 << 8;
    }
}

bitflags! {
    /// Memory map flags
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct MapFlags: usize {
        const MAP_SHARED = 1;
        const MAP_PRIVATE = 2;
        const MAP_FIXED = 1 << 2;
        const MAP_ANONYMOUS = 1 << 3;
        const PROT_READ = 1 << 4;
        const PROT_WRITE = 1 << 5;
        const PROT_EXEC = 1 << 6;
    }
}

bitflags! {
    /// Clone flags
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct CloneFlags: usize {
        const CLONE_VM = 1;
        const CLONE_FS = 1 << 1;
        const CLONE_FILES = 1 << 2;
        const CLONE_SIGHAND = 1 << 3;
        const CLONE_THREAD = 1 << 4;
    }
}

/// Seek from position
#[repr(C)]
pub enum SeekFrom {
    Start(u64),
    Current(i64),
    End(i64),
}

/// File status structure
#[repr(C)]
pub struct Stat {
    pub st_mode: u16,
    pub st_uid: u32,
    pub st_gid: u32,
    pub st_size: u64,
    pub st_blksize: u32,
    pub st_blocks: u64,
    pub st_atime: u64,
    pub st_mtime: u64,
    pub st_ctime: u64,
}

/// Error codes
#[repr(usize)]
pub enum Error {
    NoSuchEntry = 1,
    NotPermitted = 2,
    BadAddress = 3,
    OutOfMemory = 4,
    InvalidArgs = 5,
    WouldBlock = 6,
    TooBig = 7,
    NoEntity = 8,
    AlreadyExists = 9,
    BrokenPipe = 10,
    Read_only = 11,
    ConnectionRefused = 12,
    ConnectionReset = 13,
    TimedOut = 14,
}

/// Raw syscall interface
#[cfg(target_os = "aginx")]
mod arch {
    use super::*;

    #[inline(always)]
    pub unsafe fn syscall0(num: Syscall) -> usize {
        let ret: usize;
        core::arch::asm!(
            "svc #0",
            in("x8") num as usize,
            lateout("x0") ret,
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall1(num: Syscall, arg0: usize) -> usize {
        let ret: usize;
        core::arch::asm!(
            "svc #0",
            in("x8") num as usize,
            in("x0") arg0,
            lateout("x0") ret,
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall3(num: Syscall, arg0: usize, arg1: usize, arg2: usize) -> usize {
        let ret: usize;
        core::arch::asm!(
            "svc #0",
            in("x8") num as usize,
            in("x0") arg0,
            in("x1") arg1,
            in("x2") arg2,
            lateout("x0") ret,
        );
        ret
    }
}

#[cfg(target_os = "aginx")]
pub use arch::*;
