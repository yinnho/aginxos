//! Aginx OS Scheme implementation
//!
//! A scheme is a resource provider (like a filesystem, device driver, etc.)
//! Following Redox OS conventions.

#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use aginx_syscall::{Fd, OFlag, Stat};

/// Scheme trait - implement this to create a resource provider
pub trait Scheme {
    /// Open a resource by path
    fn open(&mut self, path: &str, flags: OFlag) -> Result<Fd, Error>;

    /// Close a file descriptor
    fn close(&mut self, fd: Fd) -> Result<(), Error>;

    /// Read from a file descriptor
    fn read(&mut self, fd: Fd, buf: &mut [u8]) -> Result<usize, Error>;

    /// Write to a file descriptor
    fn write(&mut self, fd: Fd, buf: &[u8]) -> Result<usize, Error>;

    /// Get file status
    fn fstat(&mut self, fd: Fd, stat: &mut Stat) -> Result<(), Error>;

    /// Duplicate file descriptor
    fn dup(&mut self, fd: Fd, buf: &[u8]) -> Result<Fd, Error>;

    /// Get scheme name
    fn name(&self) -> &str;
}

/// Scheme error type
#[derive(Debug, Clone, Copy)]
pub enum Error {
    NoSuchEntry,
    NotPermitted,
    BadAddress,
    InvalidArgs,
    WouldBlock,
    NoEntity,
    AlreadyExists,
    BrokenPipe,
    Unknown(usize),
}

impl From<usize> for Error {
    fn from(code: usize) -> Self {
        match code {
            1 => Error::NoSuchEntry,
            2 => Error::NotPermitted,
            3 => Error::BadAddress,
            5 => Error::InvalidArgs,
            6 => Error::WouldBlock,
            8 => Error::NoEntity,
            9 => Error::AlreadyExists,
            10 => Error::BrokenPipe,
            _ => Error::Unknown(code),
        }
    }
}

/// Socket abstraction for scheme communication
pub struct Socket {
    fd: Fd,
}

impl Socket {
    /// Create a new socket for scheme communication
    pub fn new(scheme: &str) -> Result<Self, Error> {
        // Open the scheme
        // In real implementation, this would use syscalls
        Ok(Socket { fd: 0 })
    }

    /// Read a request from the socket
    pub fn read_request(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        // Read request packet
        Ok(0)
    }

    /// Write a response to the socket
    pub fn write_response(&mut self, buf: &[u8]) -> Result<usize, Error> {
        // Write response packet
        Ok(0)
    }
}

/// Packet format for scheme communication
#[repr(C)]
pub struct Packet {
    pub id: u64,
    pub pid: u32,
    pub uid: u32,
    pub gid: u32,
    pub a: usize,
    pub b: usize,
    pub c: usize,
    pub d: usize,
}

/// Request types
pub enum Request {
    Open { path: String, flags: OFlag },
    Close { fd: Fd },
    Read { fd: Fd, size: usize },
    Write { fd: Fd, data: Vec<u8> },
    Fstat { fd: Fd },
    Dup { fd: Fd, data: Vec<u8> },
}

/// Response types
pub enum Response {
    Fd(Fd),
    Size(usize),
    Stat(Stat),
    Empty,
    Error(Error),
}
