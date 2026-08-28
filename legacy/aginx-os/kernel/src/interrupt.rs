//! Generic Timer driver and IRQ dispatch for aarch64
//!
//! QEMU virt Generic Timer frequency: 62.5 MHz (default)
//! Physical timer IRQ: PPI 30

use core::sync::atomic::{AtomicU64, Ordering};
use crate::gic;

/// Global tick counter (incremented every timer interrupt)
static TICKS: AtomicU64 = AtomicU64::new(0);

/// Timer frequency read from CNTFRQ_EL0
static mut TIMER_FREQ: u64 = 0;

/// Tick interval in milliseconds
static mut TICK_MS: u64 = 10;

fn read_cntfrq() -> u64 {
    let val: u64;
    unsafe { core::arch::asm!("mrs {}, CNTFRQ_EL0", out(reg) val) };
    val
}

fn write_cntp_tval(val: u32) {
    unsafe { core::arch::asm!("msr CNTP_TVAL_EL0, {}", in(reg) val as u64) };
}

fn write_cntp_ctl(val: u32) {
    unsafe { core::arch::asm!("msr CNTP_CTL_EL0, {}", in(reg) val as u64) };
}

/// Initialize Generic Timer with given tick interval in milliseconds
pub fn timer_init(tick_ms: u64) {
    let freq = read_cntfrq();
    unsafe {
        TIMER_FREQ = freq;
        TICK_MS = tick_ms;
    }

    // Disable timer first
    write_cntp_ctl(0x0);

    // Set TVAL = countdown value for tick_ms
    let tval = (freq / 1000 * tick_ms) as u32;
    write_cntp_tval(tval);

    // Enable timer: ENABLE=1, IMASK=0
    write_cntp_ctl(0x1);
}

/// Get current tick count
pub fn get_ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

/// Handle timer tick interrupt
fn timer_tick() {
    // Reload timer
    let freq = unsafe { TIMER_FREQ };
    let tick_ms = unsafe { TICK_MS };
    let tval = (freq / 1000 * tick_ms) as u32;
    write_cntp_tval(tval);

    // Increment tick counter
    TICKS.fetch_add(1, Ordering::Relaxed);
}

/// Top-level IRQ handler (called from assembly exc_irq_spx)
#[no_mangle]
pub extern "C" fn handle_irq() {
    let intid = gic::acknowledge();

    match intid {
        gic::TIMER_INTID => {
            timer_tick();
            crate::task::scheduler_tick();
            gic::end(intid);
        }
        0x3FF => {
            // Spurious interrupt
            gic::end(intid);
        }
        _ => {
            gic::end(intid);
        }
    }
}

/// Poll the timer (polling fallback when interrupts are unavailable).
/// Call this from the main loop when interrupt-driven mode isn't working.
pub fn poll_timer() {
    let ctl = unsafe {
        let val: u64;
        core::arch::asm!("mrs {0}, cntp_ctl_el0", out(reg) val);
        val as u32
    };
    if ctl & 0x4 != 0 {
        // ISTATUS set: timer has expired
        timer_tick();
        crate::task::scheduler_tick();
    }
}

/// Handle EL0 fault — print diagnostics, mark task as Dead, switch to next
#[no_mangle]
pub extern "C" fn handle_el0_fault(esr: u64, elr: u64, far: u64) {
    unsafe {
        let idx = crate::task::CURRENT;
        let u = crate::platform::UART;

        crate::uart::puts(u, "[FAULT] EL0 exception\r\n");
        crate::uart::puts(u, "  ESR=0x");
        crate::print_hex(u, esr as u32);
        crate::uart::puts(u, " ELR=0x");
        crate::print_hex(u, elr as u32);
        crate::uart::puts(u, " FAR=0x");
        crate::print_hex(u, far as u32);
        crate::uart::puts(u, "\r\n");

        // Decode ESR EC field
        let ec = (esr >> 26) & 0x3F;
        crate::uart::puts(u, "  EC=");
        crate::print_dec_u32(u, ec as u32);

        match ec {
            0x00 => crate::uart::puts(u, " (unknown)"),
            0x20..=0x23 => crate::uart::puts(u, " (insn abort)"),
            0x24..=0x25 => crate::uart::puts(u, " (data abort)"),
            0x26 => crate::uart::puts(u, " (SP alignment)"),
            0x30 => crate::uart::puts(u, " (breakpoint)"),
            _ => crate::uart::puts(u, " (other)"),
        }
        crate::uart::puts(u, "\r\n");

        if let Some(ref mut task) = crate::task::TASKS[idx] {
            crate::uart::puts(u, "  Task '");
            crate::uart::puts(u, task.name);
            crate::uart::puts(u, "' killed\r\n");
            task.state = crate::task::TaskState::Dead;
        }
    }
    crate::task::scheduler_tick();
}
