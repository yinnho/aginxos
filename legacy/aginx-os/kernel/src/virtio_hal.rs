//! VirtIO Hal implementation for Aginx OS
//!
//! Uses identity mapping (phys=virt) for DMA.

use core::alloc::Layout;
use core::ptr::NonNull;
use virtio_drivers::{BufferDirection, Hal, PhysAddr};

#[allow(dead_code)]
pub struct AginxHal;

unsafe impl Hal for AginxHal {
    fn dma_alloc(pages: usize, _direction: BufferDirection) -> (PhysAddr, NonNull<u8>) {
        let size = pages * 4096;
        let layout = match Layout::from_size_align(size, 4096) {
            Ok(l) => l,
            Err(_) => return (0, NonNull::dangling()),
        };
        let vaddr = unsafe { alloc::alloc::alloc_zeroed(layout) };
        match NonNull::new(vaddr) {
            Some(ptr) => (ptr.as_ptr() as PhysAddr, ptr),
            None => (0, NonNull::dangling()),
        }
    }

    unsafe fn dma_dealloc(_paddr: PhysAddr, vaddr: NonNull<u8>, pages: usize) -> i32 {
        let layout = Layout::from_size_align(pages * 4096, 4096).unwrap();
        alloc::alloc::dealloc(vaddr.as_ptr(), layout);
        0
    }

    unsafe fn mmio_phys_to_virt(paddr: PhysAddr, _size: usize) -> NonNull<u8> {
        NonNull::new(paddr as *mut u8).unwrap()
    }

    unsafe fn share(buffer: NonNull<[u8]>, _direction: BufferDirection) -> PhysAddr {
        buffer.as_ptr() as *mut u8 as PhysAddr
    }

    unsafe fn unshare(_paddr: PhysAddr, _buffer: NonNull<[u8]>, _direction: BufferDirection) {}
}
