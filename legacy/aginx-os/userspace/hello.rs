//! Hello World — minimal EL0 userspace program for Aginx OS
//!
//! Entry point at VA 0x10000 (set by linker script).
//! Uses raw SVC for syscalls (no aginx-syscall crate needed for this simple test).

#![no_std]
#![no_main]

const SYS_WRITE: usize = 3;
const SYS_EXIT: usize = 10;
const SYS_RETURN_TO_KERNEL: usize = 99;

#[inline(always)]
unsafe fn syscall3(nr: usize, a0: usize, a1: usize, a2: usize) -> usize {
    let ret: usize;
    core::arch::asm!(
        "svc #0",
        in("x8") nr,
        in("x0") a0,
        in("x1") a1,
        in("x2") a2,
        lateout("x0") ret,
        lateout("x1") _,
        lateout("x2") _,
        lateout("x8") _,
    );
    ret
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let msg = b"Hello from EL0!\n";
    unsafe {
        syscall3(SYS_WRITE, 1, msg.as_ptr() as usize, msg.len());
        // Use SYS_RETURN_TO_KERNEL for execdirect testing.
        // Real user programs would use SYS_EXIT with task scheduler.
        syscall3(SYS_RETURN_TO_KERNEL, 0, 0, 0);
    }
    loop {}
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
