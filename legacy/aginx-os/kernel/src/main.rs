//! Aginx OS Kernel

#![no_std]
#![no_main]
#![feature(alloc_error_handler)]
#![allow(static_mut_refs)]

extern crate alloc;

mod allocator;
mod blk;
mod frame_alloc;
mod fs;
mod platform;
mod gic;
mod interrupt;
mod mmu;
mod syscall;
mod net;
mod pci;
mod ip_stack;
mod task;
mod tcp;
mod xhci;
mod usb_net;

#[cfg(feature = "board-redfin")]
mod gcc;

#[cfg(feature = "board-redfin")]
mod usb_dwc3;

#[cfg(feature = "board-redfin")]
mod spmi;

#[cfg(feature = "board-redfin")]
mod cmd_db;

#[cfg(feature = "board-redfin")]
mod rpmh;

#[cfg(not(feature = "board-redfin"))]
mod uart;

#[cfg(feature = "board-redfin")]
mod qup_uart;

#[cfg(feature = "board-redfin")]
mod dtb;

#[cfg(feature = "board-redfin")]
mod fb;

#[cfg(feature = "board-redfin")]
use qup_uart as uart;

#[cfg(feature = "board-redfin")]
extern "C" {
    static _dtb_ptr: u64;
}

mod reboot;
mod shell;

use core::panic::PanicInfo;

// BSS skip flag: redfin defers BSS zeroing to after MMU setup
#[cfg(feature = "board-redfin")]
core::arch::global_asm!(
    ".section .rodata",
    ".balign 4",
    ".global _redfin_skip_bss",
    "_redfin_skip_bss:",
    ".long 1",
);

#[cfg(not(feature = "board-redfin"))]
core::arch::global_asm!(
    ".section .rodata",
    ".balign 4",
    ".global _redfin_skip_bss",
    "_redfin_skip_bss:",
    ".long 0",
);

// Saved SPMI LDO results (set at [10], printed at [11])
#[cfg(feature = "board-redfin")]
static mut SPMI_LDO_FOUND: u32 = 0;
#[cfg(feature = "board-redfin")]
static mut SPMI_LDO_OK: u32 = 0;

// Exception vector table for Pixel 5 MMU debugging
#[cfg(feature = "board-redfin")]
core::arch::global_asm!(
    ".section .text.excv, \"ax\", @progbits",
    ".balign 2048",
    ".global exc_vector",
    "exc_vector:",
    ".rept 16",
    "  b exc_halt",
    "  .fill 31, 4, 0",
    ".endr",
    "exc_halt:",
    "  movz x0, #0xA000, lsl #16",   // x0 = 0xA0000000
    "  mvn w1, wzr",                  // w1 = 0xFFFFFFFF (white pixel)
    "  str w1, [x0]",
    "  str w1, [x0, #4]",             // two pixels for visibility
    ".Lexc_loop:",
    "  wfi",
    "  b .Lexc_loop",
);

#[cfg(feature = "board-redfin")]
extern "C" {
    static exc_vector: u8;
}

#[cfg(not(feature = "board-redfin"))]
use alloc::vec::Vec;

// UART 基地址
use crate::platform::UART as U;
#[cfg(not(feature = "board-redfin"))]
const HEAP_SIZE: usize = 0x200_0000; // 32MB heap

// ─── Print helpers (used by UART-only commands) ─────────────────────────────

pub(crate) fn print_hex(base: usize, val: u32) {
    for i in (0..8).rev() {
        let nibble = ((val >> (i * 4)) & 0xF) as u8;
        uart::putc(base, if nibble < 10 { b'0' + nibble } else { b'a' + nibble - 10 });
    }
}

pub(crate) fn print_hex_byte(base: usize, val: u8) {
    let hi = (val >> 4) & 0xF;
    let lo = val & 0xF;
    uart::putc(base, if hi < 10 { b'0' + hi } else { b'a' + hi - 10 });
    uart::putc(base, if lo < 10 { b'0' + lo } else { b'a' + lo - 10 });
}

#[inline(never)]
pub(crate) fn print_dec_u32(base: usize, mut n: u32) {
    if n == 0 { uart::putc(base, b'0'); return; }
    let mut buf: [u8; 10] = [0; 10];
    let mut i = 0;
    while n > 0 { buf[i] = b'0' + (n % 10) as u8; n /= 10; i += 1; }
    while i > 0 { i -= 1; uart::putc(base, buf[i]); }
}

pub(crate) fn print_ip(base: usize, ip: [u8; 4]) {
    for i in 0..4 {
        if i > 0 { uart::putc(base, b'.'); }
        print_dec_u32(base, ip[i] as u32);
    }
}

// ─── EL0 User Task Test ──────────────────────────────────────────────────────

#[cfg(not(feature = "board-redfin"))]
extern "C" {
    #[allow(dead_code)]
    fn user_test_task() -> !;
}

// User task that runs in EL0. It uses SVC to write a message.
// x0 = user_stack_top (passed via frame)
// Written in pure asm to ensure x0 is captured before compiler prologue clobbers it.
//
// Debug strategy: first do direct UART write (bypass SVC) to verify EL0 execution,
// then try SVC syscall.
#[cfg(not(feature = "board-redfin"))]
core::arch::global_asm!(
    ".section .text.user_test_task, \"ax\", @progbits",
    ".global user_test_task",
    ".type user_test_task, %function",
    "user_test_task:",
    // x0 = user_stack_top (set by init_user_task_stack)
    "msr SP_EL0, x0",           // Set user stack

    // Step 1: Direct UART write to prove we're in EL0
    "ldr x0, =0x09000000",
    "adr x1, .Lstep1",
    "2:",
    "ldrb w2, [x1]",
    "cbz w2, 3f",
    "strb w2, [x0]",
    "add x1, x1, #1",
    "b 2b",
    "3:",

    // Step 2: SVC write
    "adr x1, .Lstep2",
    "mov x2, #19",
    "mov x8, #3",
    "mov x0, #1",
    "svc #0",

    // Step 3: SVC exit
    "mov x8, #10",
    "mov x0, #42",
    "svc #0",

    // Fallback loop
    "4:",
    "wfi",
    "b 4b",

    ".align 4",
    ".Lstep1:",
    ".asciz \"[EL0] Direct UART ok\\n\"",
    ".Lstep2:",
    ".ascii \"[EL0] SVC write ok!\\n\"",
    ".size user_test_task, . - user_test_task",
);

// ─── Kernel Entry Point ───────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn rust_main(kernel_end: usize, _page_table_arg: usize) -> ! {
    let page_table = (kernel_end + 4095) & !4095;

    #[cfg(not(feature = "board-redfin"))]
    let heap_start = (page_table + 0x4000 + 4095) & !4095;

    #[cfg(feature = "board-redfin")]
    let _heap_start = 0usize;

    #[cfg(not(feature = "board-redfin"))]
    {
        uart::init(U);
        uart::puts(U, "\r\nAginx OS v0.2.0\r\n");
        uart::puts(U, "[..] MMU\r\n");
        unsafe { mmu::init(page_table as *mut u64) };
        uart::puts(U, "[OK] MMU\r\n");

        unsafe { frame_alloc::init(kernel_end) };
        uart::puts(U, "[OK] Frame allocator\r\n");

        allocator::init(heap_start, HEAP_SIZE);
        uart::puts(U, "[OK] Heap (32MB) @0x");
        crate::print_hex(U, heap_start as u32);
        uart::puts(U, "\r\n");

        // Test 1KB alloc
 unsafe {
            let layout = alloc::alloc::Layout::from_size_align(1024, 8).unwrap();
            let p = alloc::alloc::alloc(layout);
            if !p.is_null() {
                for i in 0..1024 {
                    core::ptr::write_volatile(p.add(i), 42u8);
                }
                let _ok = true;
                for i in 0..1024 {
                    assert!(core::ptr::read_volatile(p.add(i)) == 42u8);
                }
                uart::puts(U, "[OK] 1KB alloc\r\n");
            } else {
                uart::puts(U, "[FAIL] 1KB alloc\r\n");
            }
        }
        {
            let mut v: Vec<u8> = Vec::new();
            for i in 0..100u8 { v.push(i); }
            if v[0] == 0 && v[99] == 99 {
                uart::puts(U, "[OK] Self-test passed\r\n");
            } else {
                uart::puts(U, "[FAIL] Self-test\r\n");
            }
        }

        task::init();
        uart::puts(U, "[OK] Task init\r\n");

        uart::puts(U, "[..] GIC\r\n");
        gic::init(U);
        uart::puts(U, "[OK] GIC\r\n");
        interrupt::timer_init(10);
        // Enable CPU IRQ (I-bit) so timer interrupts and device completions can fire
        unsafe { core::arch::asm!("msr DAIFClr, #0x4"); }
        uart::puts(U, "[OK] Timer\r\n");

        pci::init(U);

        net::init(U);

        uart::puts(U, "[..] TCP/IP\r\n");
        tcp::init();

        // Send ARP request for gateway so SLIRP learns our MAC
        uart::puts(U, "[..] ARP\r\n");
        crate::ip_stack::arp_resolve_test();
        uart::puts(U, "[OK] ARP\r\n");

        uart::puts(U, "[..] DHCP\r\n");
        if tcp::wait_for_dhcp(5000) {
            uart::puts(U, "[OK] DHCP\r\n");
            uart::puts(U, "  IP: 10.0.2.15\r\n");
        } else {
            uart::puts(U, "[WARN] DHCP timeout\r\n");
        }

        blk::init(U);

        fs::init();

        // Run startup script if it exists
        shell::run_autoexec();

        // Auto-listen on port 9091 for testing
        uart::puts(U, "[..] Auto-listen 9091\r\n");
        tcp::tcp_listen(9091);
        uart::puts(U, "[OK] Auto-listen\r\n");

        // Ping test — verify RX end-to-end (disabled for TCP testing)
        // uart::puts(U, "[..] Ping GW\r\n");
        // let replies = tcp::ping([10, 0, 2, 2], 1);
        // uart::puts(U, "[OK] Ping ");
        // crate::print_dec_u32(U, replies as u32);
        // uart::puts(U, "/1\r\n");

        uart::puts(U, "# ");
        let mut uart_editor = shell::LineEditor::new();
        let mut tcp_editor = shell::LineEditor::new();
        let mut tcp_connected = false;
        let mut tcp_buf = [0u8; 256];

        loop {
            interrupt::poll_timer();

            // Process packets through the IP stack (multiple polls to drain RX)
            for _ in 0..8 {
                tcp::poll();
            }

            // ── TCP remote shell (slot 0, port 9091) ──
            if !shell::is_net_service_active() {
                match tcp::tcp_listen_state() {
                    tcp::TcpState::Connected => {
                        if !tcp_connected {
                            // New connection
                            tcp_connected = true;
                            uart::puts(U, "[TCP] Client connected\r\n");
                            // Send banner
                            crate::ip_stack::tcp_slot_send(0, b"Aginx OS remote shell\r\n# ");
                            crate::ip_stack::tcp_slot_flush(0);
                            tcp_editor.reset();
                        }
                        // Read TCP data and feed to line editor
                        let n = tcp::tcp_recv(&mut tcp_buf);
                        for &c in &tcp_buf[..n] {
                            if tcp_editor.feed(c, shell::OutputDest::Tcp) == shell::InputAction::Complete {
                                let line = tcp_editor.line();
                                if line.len() > 0 {
                                    shell::set_output_to_slot(0);
                                    shell::execute_command(line);
                                }
                                tcp_editor.reset();
                                shell::set_output_to_slot(0);
                                shell::putc(b'#');
                                shell::putc(b' ');
                            }
                        }
                    }
                    tcp::TcpState::Closed | tcp::TcpState::None => {
                        if tcp_connected {
                            // Connection lost
                            tcp_connected = false;
                            shell::set_output(shell::OutputDest::Uart);
                            uart::puts(U, "[TCP] Client disconnected\r\n");
                        }
                        // Re-listen on port 9091
                        tcp::tcp_listen(9091);
                    }
                    _ => {} // Listening / SynSent / etc — wait
                }
            }

            // ── UART local shell ──
            let c = uart::getc_nb(U);
            if c != 0 {
                if uart_editor.feed(c, shell::OutputDest::Uart) == shell::InputAction::Complete {
                    let line = uart_editor.line();
                    if line.len() > 0 {
                        shell::set_output(shell::OutputDest::Uart);
                        shell::execute_command(line);
                    }
                    uart_editor.reset();
                    uart::puts(U, "# ");
                }
            } else {
                for _ in 0..1000 { core::hint::spin_loop(); }
            }
        }
    }

    #[cfg(feature = "board-redfin")]
    {
        // === Pixel 5 bring-up: FB-only then halt (proven 2026-04-11) ===
        // Full init after this (MMU/UART/heap/GIC) historically crashed.
        let _page_table = (kernel_end + 16383) & !16383;

        let fb = fb::Framebuffer::new(0xA000_0000u64);
        fb.clear(fb::GREEN);
        let mut con = fb::Console::new(fb);
        con.set_color(fb::WHITE);
        con.puts("Aginx OS v0.2.0\r\n");
        con.puts("[1] FB ok\r\n");
        con.flush();

        // USB Host path: read-only probe. No core reset, no PHY MMIO.
        // Pixel = host, ESP = device (next: OTG + enumerate CDC).
        con.puts("[2] USB probe (RO)\r\n");
        con.flush();

        con.puts("[2a] GCC GDSCR=");
        con.put_hex32(gcc::usb30_gdscr());
        con.puts(" MCLK=");
        con.put_hex32(gcc::usb30_master_cbcr());
        con.puts(" BCR=");
        con.put_hex32(gcc::usb30_bcr());
        con.puts("\r\n");
        con.flush();

        con.puts("[2b] DWC3 SNPSID=");
        con.put_hex32(usb_dwc3::get_snpsid());
        con.puts("\r\n");
        con.flush();

        let gctl = usb_dwc3::get_gctl();
        let prtcap = (gctl >> 12) & 3;
        con.puts("[2c] GCTL=");
        con.put_hex32(gctl);
        con.puts(" CAP=");
        con.putc(b'0' + (prtcap as u8)); // 1=HOST 2=DEV 3=OTG — no string, no DCTL
        con.puts("\r\n");
        con.flush();

        // Gadget for Mac: stay Device. Do not switch Host, do not read DCTL/DSTS.
        // Prior run: [4] looped with Mac seeing nothing — PHY analog rails were off.
        con.puts("[3] gadget\r\n");
        con.flush();

        // CMD-DB + RPMh: turn on SNPS Femto supplies (DT: pm8150 L5/L12/L2).
        con.puts("[3a] cmddb\r\n");
        con.flush();
        let mut l5: u32 = 0;
        let mut l12: u32 = 0;
        let mut l2: u32 = 0;
        if cmd_db::is_present() {
            l5 = cmd_db::find_vrm(b"ldoa5");
            l12 = cmd_db::find_vrm(b"ldoa12");
            l2 = cmd_db::find_vrm(b"ldoa2");
            con.puts("[3a] L5=");
            con.put_hex32(l5);
            con.puts("\r\nL12=");
            con.put_hex32(l12);
            con.puts("\r\nL2=");
            con.put_hex32(l2);
            con.puts("\r\n");
            con.flush();
        } else {
            let h = cmd_db::probe_header();
            con.puts("[3a] BAD ");
            con.put_hex8(h[4]);
            con.put_hex8(h[5]);
            con.put_hex8(h[6]);
            con.put_hex8(h[7]);
            con.puts("\r\n");
            con.flush();
        }

        // Do not touch GCC clocks / AHB2PHY / PIPE UTMI here:
        // that sequence reached [3e] then hung on DCTL write.
        // Keep the gadget path that previously reached the [4] loop.
        con.puts("[3c] VBUS\r\n");
        con.flush();
        let hs = usb_dwc3::qscratch_vbus_override();
        con.puts("[3c] hs=");
        con.put_hex32(hs);
        con.puts("\r\n");
        con.flush();

        let dev = (gctl & !0x3000) | 0x2000;
        usb_dwc3::set_gctl(dev);
        con.puts("[3d] GCTL=");
        con.put_hex32(usb_dwc3::get_gctl());
        con.puts("\r\n");
        con.flush();

        // Skip event buf / DEVTEN / DCFG / DCTL here — they hang some boots
        // after [3d]. Get RPMh + PHY peek on screen first.
        usb_dwc3::reset_diag_counts();
        let pin = con.row;
        con.clear_line(pin);
        con.puts("L5  ");
        con.put_hex32(l5);
        con.clear_line(pin + 1);
        con.puts("L12 ");
        con.put_hex32(l12);
        con.clear_line(pin + 2);
        con.puts("L2  ");
        con.put_hex32(l2);
        con.flush();
        // RPMh after RUN_STOP — earlier sequence hung DCTL.
        con.clear_line(pin + 3);
        con.puts("[3r] rpmh");
        con.flush();
        if l5 != 0 {
            let _ = rpmh::vrm_enable_full(l5, 880, &mut con);
        }
        if l12 != 0 {
            let _ = rpmh::vrm_enable_full(l12, 1800, &mut con);
        }
        if l2 != 0 {
            let _ = rpmh::vrm_enable_full(l2, 3072, &mut con);
        }
        con.clear_line(pin + 3);
        con.puts("RSC ");
        con.put_hex32(rpmh::rsc_id());
        con.puts(" ST ");
        con.put_hex32(rpmh::last_st());
        con.flush();
        // Rails are up (COMPL). Retry PHY CSR + QSCRATCH POR.
        con.clear_line(pin + 8);
        con.puts("[5] p=");
        con.flush();
        let p54 = usb_dwc3::peek32(0x088e_3054);
        con.put_hex32(p54);
        con.flush();
        if p54 != 0x8888_8888 && p54 != 0x0f0f_0f0f {
            let q = usb_dwc3::femto_hs_on_mmio();
            con.clear_line(pin + 9);
            con.puts("[5] q=");
            con.put_hex32(q);
            con.flush();
        } else {
            let _ = usb_dwc3::qscratch_phy_por();
            let (ok, acc) = usb_dwc3::phyacc_snps_read(0x54);
            con.clear_line(pin + 9);
            con.puts("[5] acc=");
            con.puts(if ok { "ok " } else { "TO " });
            con.put_hex32(acc);
            con.flush();
        }
        let _ = usb_dwc3::qscratch_vbus_override();
        con.clear_line(pin + 10);
        con.puts("[3e] wr");
        con.flush();
        usb_dwc3::gadget_run_stop();
        con.clear_line(pin + 10);
        con.puts("[3e] ok");
        con.flush();
        let mut tick: u32 = 0;
        loop {
            usb_dwc3::poll();
            for _ in 0..2_000_000 {
                core::hint::spin_loop();
            }
            tick = tick.wrapping_add(1);
            if tick & 0x3f == 0 {
                let evc = usb_dwc3::read_reg(0xc40c);
                con.clear_line(pin + 4);
                con.puts("EVC ");
                con.put_hex32(evc);
                con.clear_line(pin + 5);
                con.puts("N   ");
                con.put_hex32(usb_dwc3::get_event_count());
                con.clear_line(pin + 6);
                con.puts("SU  ");
                con.put_hex32(usb_dwc3::get_setup_count());
                con.clear_line(pin + 7);
                con.puts("HS  ");
                con.put_hex32(usb_dwc3::qscratch_hs_phy());
                con.flush();
            }
        }

        #[allow(unreachable_code)]
        {
        // === Pixel 5: MMU first, then BSS zero, then init ===
        // BSS zeroing is deferred to after MMU setup because ABL's
        // page tables may not cover the full BSS range.
        let page_table = (kernel_end + 16383) & !16383;

        let fb = fb::Framebuffer::new(0xA000_0000u64);
        fb.clear(fb::BLACK);
        let mut con = fb::Console::new(fb); // mut for dump_apid_table
        con.puts("Aginx OS v0.2.0\r\n");
        con.puts("[1] FB ok\r\n");
        con.flush();

        // === MMU setup ===
        con.puts("[2] MMU..\r\n"); con.flush();
        unsafe {
            let pt = page_table as *mut u64;
            for i in 0..512usize {
                core::ptr::write_volatile(pt.add(i), 0);
            }
            core::ptr::write_volatile(pt, 0x0000_0000_0000_0701u64);
            core::ptr::write_volatile(pt.add(2), 0x0000_0000_8000_0705u64);
            core::ptr::write_volatile(pt.add(4), 0x0000_0001_0000_0701u64);
            core::arch::asm!("dsb sy");
            let mair: u64 = 0x0000_0000_0044_FF04;
            core::arch::asm!("msr mair_el1, {}", in(reg) mair);
            let pt_addr = pt as u64;
            core::arch::asm!("msr ttbr0_el1, {}", in(reg) pt_addr);
            core::arch::asm!("dsb nsh");
            core::arch::asm!("tlbi vmalle1");
            core::arch::asm!("dsb nsh");
            core::arch::asm!("isb");
            let tcr: u64 = 0x0000_0005_0000_3519;
            core::arch::asm!("msr tcr_el1, {}", in(reg) tcr);
            core::arch::asm!("isb");
            let vec_addr = &exc_vector as *const u8 as u64;
            core::arch::asm!("msr vbar_el1, {}", in(reg) vec_addr);
            core::arch::asm!("isb");
            mmu::KERNEL_TTBR0 = pt_addr;
            core::arch::asm!(
                "tlbi vmalle1",
                "dsb ish",
                "isb",
                "mrs x20, sctlr_el1",
                "orr x20, x20, #1",
                "msr sctlr_el1, x20",
                "isb",
                "ic iallu",
                "dsb ish",
                "isb",
                out("x20") _,
            );
        }
        con.puts("[2] MMU ok\r\n"); con.flush();

        // Zero BSS now that our page tables cover all of RAM
        unsafe {
            extern "C" { static __bss_start: u64; static __bss_end: u64; }
            let start = &__bss_start as *const u64 as usize;
            let end = &__bss_end as *const u64 as usize;
            let mut p = start as *mut u64;
            let e = end as *mut u64;
            while p < e { core::ptr::write_volatile(p, 0); p = p.add(1); }
        }
        con.puts("[2b] BSS ok\r\n"); con.flush();

        // GCC clocks for QUP UART
        con.puts("[3] GCC..\r\n"); con.flush();
        let _step = gcc::enable_qup_uart_debug();
        con.puts("[3] GCC ok\r\n"); con.flush();

        // UART
        uart::init(U);
        uart::puts(U, "[UART] alive\r\n");
        con.puts("[4] UART ok\r\n"); con.flush();

        // Diagnostic halt — test if basic boot works
        // con.puts("[OK] Minimal boot ok — halting\r\n"); con.flush();
        // loop { unsafe { core::arch::asm!("wfi"); } }

        // Heap + Frame allocator
        con.puts("[5] Heap..\r\n"); con.flush();
        let heap_start = (page_table + 0x5000 + 4095) & !4095;
        let heap_size: usize = 0x200_0000;
        allocator::init(heap_start, heap_size);
        con.puts("[5a] alloc ok\r\n"); con.flush();

        // Test alloc
        unsafe {
            let layout = alloc::alloc::Layout::from_size_align(1024, 8).unwrap();
            let p = alloc::alloc::alloc(layout);
            if !p.is_null() {
                for i in 0..1024 {
                    core::ptr::write_volatile(p.add(i), 42u8);
                }
                con.puts("[5b] 1KB alloc ok\r\n");
            } else {
                con.puts("[5b] 1KB alloc FAIL\r\n");
            }
        }
        con.flush();

        con.puts("[OK] Heap ok\r\n"); con.flush();

        // Frame alloc — split into steps (init() crashes due to codegen issue)
        frame_alloc::zero_bitmap();
        let first_free = (kernel_end + 4095) & !4095;
        let first_idx = (first_free - 0x8000_0000) / 4096;
        frame_alloc::mark_range(0, first_idx);
        let stack_bottom_idx = (0x87FF_0000 - 0x8000_0000) / 4096;
        let stack_top_idx = (0x8800_0000 - 0x8000_0000) / 4096;
        frame_alloc::mark_range(stack_bottom_idx, stack_top_idx);
        con.puts("[OK] Frame alloc\r\n"); con.flush();

        // GIC — parse DTB for correct address, then init
        con.puts("[7] GIC..\r\n"); con.flush();
        let dtb_addr = unsafe { &_dtb_ptr as *const u64 as usize };
        let dtb_ptr_val = unsafe { core::ptr::read_volatile(dtb_addr as *const u64) };
        if dtb_ptr_val != 0 {
            con.puts("[7a] DTB=0x");
            con.put_hex32(dtb_ptr_val as u32);
            con.puts("\r\n"); con.flush();

            if let Some((gicd, gicc)) = dtb::find_gic(dtb_ptr_val as *const u8) {
                con.puts("[7b] GICD=0x");
                con.put_hex32(gicd as u32);
                con.puts(" GICC=0x");
                con.put_hex32(gicc as u32);
                con.puts("\r\n"); con.flush();

                // Try reading GICD_CTLR to verify address is valid
                let ctlr = unsafe { core::ptr::read_volatile(gicd as *const u32) };
                con.puts("[7c] CTLR=0x");
                con.put_hex32(ctlr);
                con.puts("\r\n"); con.flush();

                // Init GIC with DTB-derived address
                gic::init_with_base(gicd, gicc, U);
                con.puts("[7] GIC ok\r\n"); con.flush();
            } else {
                con.puts("[7b] GIC not found in DTB\r\n"); con.flush();
            }
        } else {
            con.puts("[7a] no DTB\r\n"); con.flush();
        }
        interrupt::timer_init(10);

        // Task scheduler
        con.puts("[8] Tasks..\r\n"); con.flush();
        task::init();
        con.puts("[8] Tasks ok\r\n"); con.flush();

        // IRQ — skip (no GIC)
        // unsafe { core::arch::asm!("msr DAIFClr, #0x4"); }
        con.puts("[9] IRQ skip\r\n"); con.flush();

        // SPMI (power button + LDOs)
        con.puts("[10] SPMI..\r\n"); con.flush();
        if spmi::is_present() {
            con.puts("[10] SPMI ok\r\n"); con.flush();
            // Diagnostic: print version, config, APID count, test read
            let ver = spmi::get_version();
            let cfg = spmi::read_diag(0x04);
            let cnt = spmi::get_apid_count();
            con.puts("[10d] ver="); con.put_hex32(ver);
            con.puts(" cfg="); con.put_hex32(cfg);
            con.puts(" cnt="); con.put_hex32(cnt);
            con.puts("\r\n"); con.flush();
            // Test read APID=1 type (power button peripheral)
            let (_, st1, rt1) = spmi::obs_cmd_read(1, 0x04, 1);
            con.puts("[10d] apid1 st="); con.put_hex32(st1);
            con.puts(" typ="); con.put_hex32(rt1 & 0xFF);
            con.puts("\r\n"); con.flush();
            // Scan SPMI bus for peripherals and enable ALL LDOs (type 0x02 classic + 0x1A LDO516)
            // Use observer channel for both reads and writes (TX channel crashes)
            let apid_cnt = spmi::get_apid_count();
            con.puts("[10e] scan apids="); con.put_hex32(apid_cnt);
            con.puts("\r\n"); con.flush();
            let mut ldo_ok = 0u32;
            let mut ldo_found = 0u32;
            let mut found_types: [u8; 16] = [0; 16]; // compact type log
            let mut ft_idx = 0usize;
            let scan_max = if apid_cnt < 512 { apid_cnt } else { 512 };
            for apid in 0..scan_max {
                let (_, status, rdata) = spmi::obs_cmd_read(apid, 0x04, 1);
                if (status & 0x01) == 0 || (status & 0x06) != 0 || rdata == 0 {
                    continue;
                }
                let typ = (rdata & 0xFF) as u8;
                // Log first 16 non-zero types for diagnostic
                if ft_idx < 16 {
                    found_types[ft_idx] = typ;
                    ft_idx += 1;
                }
                // LDO: type 0x02 (classic), 0x06 (PMIC5), or 0x1A (LDO516)
                if typ != 0x02 && typ != 0x06 && typ != 0x1A { continue; }
                ldo_found += 1;
                // Enable LDO: HPM mode (offset 0x45) + enable (offset 0x46)
                let (st1, _) = spmi::obs_cmd_write(apid, 0x45, &[0x80]); // HPM mode
                let (st2, _) = spmi::obs_cmd_write(apid, 0x46, &[0x80]); // Enable
                let ok = (st2 & 0x01) != 0 && (st2 & 0x06) == 0;
                con.puts(" L"); con.put_hex8(apid as u8);
                con.put_hex8(typ);
                con.puts(if ok { "ok" } else { "X" });
                con.flush();
                if ok { ldo_ok += 1; }
            }
            con.puts("\r\n[10e] types:");
            for i in 0..ft_idx {
                con.put_hex8(found_types[i]); con.puts(" ");
            }
            con.puts("\r\n[10e] LDO found="); con.put_hex32(ldo_found);
            con.puts(" ok="); con.put_hex32(ldo_ok);
            // Save for later display in USB init
            unsafe { SPMI_LDO_FOUND = ldo_found; SPMI_LDO_OK = ldo_ok; }
            con.puts("\r\n"); con.flush();

            // [10f] Dump APID table (compact: APID=PPID for non-zero)
            con.puts("[10f] APID:"); con.flush();
            for apid in 0..128u32 {
                let entry = spmi::apid_map(apid);
                if entry == 0 { continue; }
                let ppid = ((entry >> 8) & 0xFFF) as u16;
                con.put_hex8(apid as u8);
                con.put_hex8(ppid as u8);  // SID=upper nibble, PID=lower byte
                con.puts(" ");
                con.flush();
            }
            con.puts("\r\n"); con.flush();

            // [10f2] L18 register dump already done, skip TX channel test

            // [10g] RPMh — dump TCS commands with correct layout
            // TCS layout: CMD_ENABLE(+0x1C), CMD_WAIT(+0x20)
            // Commands start at TCS+0x30, stride 0x14: MSGID ADDR DATA STATUS RESP
            // TCS stride = 0x2A0, TCS area starts at DRV+0xC00
            {
                // Scan DRV0 and DRV2
                let drvs: [usize; 2] = [0x0AF00000, 0x0AF20000];
                for (di, &drv_base) in drvs.iter().enumerate() {
                    con.puts("[10g] D"); con.put_hex8(di as u8);
                    con.puts(":"); con.flush();
                    // Check first 4 TCS channels
                    for ti in 0..4usize {
                        let tcs = drv_base + 0xC00 + ti * 0x2A0;
                        let en = unsafe { core::ptr::read_volatile((tcs + 0x1C) as *const u32) };
                        let wait = unsafe { core::ptr::read_volatile((tcs + 0x20) as *const u32) };
                        if en == 0 && wait == 0 { continue; }
                        con.put_hex8(ti as u8);
                        con.puts("=E"); con.put_hex32(en);
                        con.puts("W"); con.put_hex32(wait);
                        // Read up to 4 command slots
                        for ci in 0..4usize {
                            let cmd_base = tcs + 0x30 + ci * 0x14;
                            let msgid = unsafe { core::ptr::read_volatile((cmd_base) as *const u32) };
                            let addr  = unsafe { core::ptr::read_volatile((cmd_base + 4) as *const u32) };
                            let data  = unsafe { core::ptr::read_volatile((cmd_base + 8) as *const u32) };
                            if msgid == 0 && addr == 0 && data == 0 { continue; }
                            con.put_hex8(ci as u8);
                            con.puts(":"); con.put_hex32(addr);
                            con.puts("="); con.put_hex32(data);
                            con.puts(" "); con.flush();
                        }
                    }
                    con.puts("\r\n"); con.flush();
                }
            }
        } else {
            con.puts("[10] SPMI fail\r\n"); con.flush();
        }

        // USB: GCC clocks → CMD-DB → RPMh regulators → PHY reset → DWC3 init
        {
            // Step 1: GCC USB30 clocks — NO PHY reset (preserve ABL's PHY config)
            con.puts("[11clk].."); con.flush();
            let clk_result = crate::gcc::enable_usb30_clocks_no_phy_reset();
            let clk_ok = clk_result;
            if clk_ok { con.puts("ok"); } else { con.puts("FAIL"); }
            con.puts("\r\n"); con.flush();

            if !clk_ok {
                con.puts("[11] USB SKIP\r\n"); con.flush();
            } else {
                // Step 2: CMD-DB search for PM8150 VRM addresses
                // Pixel 5 SNPS Femto PHY: vdd=PM8150_L5(0.72V), vdda18=PM8150_L12(1.7V), vdda33=PM8150_L2(2.7V)
                con.puts("[11db] search.."); con.flush();

                let cmdb_base: usize = 0x8086_0000;
                let mut l5_vrm: u32 = 0;
                let mut l12_vrm: u32 = 0;
                let mut l2_vrm: u32 = 0;

                let mut scanned: u32 = 0;
                let targets: [&[u8]; 3] = [b"ldoa5\0\0\0", b"ldoa12\0\0", b"ldoa2\0\0\0"];
                let mut found = [false; 3];
                'outer: for i in 0..32u32 {
                    let base = 8 + i as usize * 16;
                    let cnt = unsafe { core::ptr::read_volatile((cmdb_base + base + 6) as *const u16) };
                    if cnt == 0 { continue; }
                    let hdr_off = unsafe { core::ptr::read_volatile((cmdb_base + base + 2) as *const u16) } as usize;
                    for j in 0..cnt {
                        if scanned >= 2000 { break 'outer; }
                        scanned += 1;
                        let entry = 528 + hdr_off + j as usize * 24;
                        if entry + 24 > 0x20000 { continue; }
                        let mut name_buf = [0u8; 8];
                        for k in 0..8 {
                            name_buf[k] = unsafe { core::ptr::read_volatile((cmdb_base + entry + k) as *const u8) };
                        }
                        for (ti, &target) in targets.iter().enumerate() {
                            if found[ti] { continue; }
                            let mut matched = true;
                            for k in 0..8 {
                                if target[k] == 0 { break; }
                                if name_buf[k] != target[k] { matched = false; break; }
                            }
                            if matched {
                                let addr = unsafe { core::ptr::read_volatile((cmdb_base + entry + 16) as *const u32) };
                                match ti {
                                    0 => { l5_vrm = addr; found[0] = true; }
                                    1 => { l12_vrm = addr; found[1] = true; }
                                    2 => { l2_vrm = addr; found[2] = true; }
                                    _ => {}
                                }
                                for k in 0..8 {
                                    let c = name_buf[k];
                                    if c == 0 { break; }
                                    if c >= 0x20 && c < 0x7f {
                                        con.puts(core::str::from_utf8(&[c]).unwrap_or("."));
                                    }
                                }
                                con.puts("="); con.put_hex32(addr);
                                con.puts(" "); con.flush();
                            }
                        }
                        if found[0] && found[1] { break 'outer; }
                    }
                }
                con.puts("\r\n[11db] L5="); con.put_hex32(l5_vrm);
                con.puts(" L12="); con.put_hex32(l12_vrm);
                con.puts(" L2="); con.put_hex32(l2_vrm);
                con.puts("\r\n"); con.flush();

                // Step 3: Enable regulators via RPMh
                // Must call dump_tcs first to "wake up" RSC bus
                rpmh::dump_tcs(&mut con);
                if l5_vrm != 0 && l12_vrm != 0 {
                    con.puts("[11r] "); con.flush();
                    let r1 = rpmh::vrm_enable_full(l5_vrm, 720000, &mut con);
                    let r2 = rpmh::vrm_enable_full(l12_vrm, 1700000, &mut con);
                    if l2_vrm != 0 {
                        let _ = rpmh::vrm_enable_full(l2_vrm, 2700000, &mut con);
                    }
                    con.puts(" L5="); con.puts(if r1 { "ok" } else { "X" });
                    con.puts(" L12="); con.puts(if r2 { "ok" } else { "X" });
                    con.puts("\r\n"); con.flush();
                } else {
                    con.puts("[11r] NO ADDR\r\n"); con.flush();
                }

                // Step 3b: SPMI diagnostic — check PM8150 LDO status + direct enable
                con.puts("[11spmi] LDOs:"); con.flush();
                let mut ldo_count = 0u32;
                let mut ldo_apids: [(u32, u8, u8); 32] = [(0, 0, 0); 32]; // (apid, pid, en)
                for apid in 0..256u32 {
                    let entry = spmi::apid_map(apid);
                    if entry == 0 { continue; }
                    let ppid = ((entry >> 8) & 0xFFF) as u16;
                    let sid = (ppid >> 8) & 0xF;
                    if sid != 0 { continue; } // PM8150 only (SID=0)
                    let pid = ppid & 0xFF;
                    // Read type
                    let (_, st, rd) = spmi::obs_cmd_read(apid, 0x04, 1);
                    if (st & 0x01) == 0 || (st & 0x06) != 0 { continue; }
                    let typ = (rd & 0xFF) as u8;
                    if typ != 0x02 && typ != 0x06 && typ != 0x1A { continue; }
                    // Read enable register
                    let en_byte = spmi::obs_read_byte(apid, 0x46);
                    con.puts(" P"); con.put_hex8(pid as u8);
                    con.puts("="); con.put_hex8(en_byte);
                    con.flush();
                    if ldo_count < 32 {
                        ldo_apids[ldo_count as usize] = (apid, pid as u8, en_byte);
                    }
                    ldo_count += 1;
                }
                con.puts(" n="); con.put_hex32(ldo_count);
                con.puts("\r\n"); con.flush();

                // SPMI direct enable: enable ALL disabled PM8150 LDOs, check PLL after each
                con.puts("[11se] try all:"); con.flush();
                for i in 0..(ldo_count as usize).min(32) {
                    let (apid, pid, en) = ldo_apids[i];
                    if en == 0x80 { continue; } // already enabled
                    // Read current voltage first
                    let cur_v = spmi::obs_read_byte(apid, 0x40);
                    // Enable: HPM mode + enable bit
                    let _ = spmi::obs_cmd_write(apid, 0x45, &[0x80]); // HPM
                    let (st, _) = spmi::obs_cmd_write(apid, 0x46, &[0x80]); // Enable
                    let ok = (st & 0x01) != 0 && (st & 0x06) == 0;
                    let new_en = spmi::obs_read_byte(apid, 0x46);
                    con.puts(" P"); con.put_hex8(pid);
                    con.puts("v"); con.put_hex8(cur_v);
                    con.puts("="); con.put_hex8(new_en);
                    con.puts(if ok { "ok" } else { "X" });
                    con.flush();
                    // Check PLL after each enable
                    let pll = usb_dwc3::phy_read8(0x1A0);
                    if pll != 0 {
                        con.puts(" PLL!"); con.put_hex32(pll as u32);
                        con.puts("\r\n"); con.flush();
                        break;
                    }
                }
                con.puts("\r\n"); con.flush();

                // Step 4: DWC3 init — full warm takeover from ABL
                let snpsid = usb_dwc3::read_reg(0xc120);
                con.puts("[11a] snps="); con.put_hex32(snpsid);
                let dctl0 = usb_dwc3::read_reg(0xc704);
                let dsts0 = usb_dwc3::get_dsts();
                let gctl0 = usb_dwc3::read_reg(0xc110);
                con.puts(" dctl="); con.put_hex32(dctl0);
                con.puts(" dsts="); con.put_hex32(dsts0);
                con.puts(" gctl="); con.put_hex32(gctl0);
                con.puts("\r\n"); con.flush();

                let usb_ok = usb_dwc3::init_abl_takeover_fs(&mut con);
                con.puts("[11fs] "); con.puts(if usb_ok { "ok" } else { "FAIL" });
                con.puts("\r\n"); con.flush();

                // Invalidate cache on event buffer (DWC3 DMA bypasses cache)
                unsafe {
                    let buf_ptr = usb_dwc3::get_event_buf_ptr();
                    for off in (0..4096).step_by(64) {
                        core::arch::asm!("dc ivac, {}", in(reg) buf_ptr.add(off));
                    }
                    core::arch::asm!("dsb sy", "isb");
                }

                // Poll for events — long wait for host enumeration
                con.puts("[11ev] poll..\r\n"); con.flush();
                let mut got_event = false;
                for i in 0..100 {
                    for _ in 0..1_000_000 { core::hint::spin_loop(); }
                    // Invalidate cache each time
                    unsafe {
                        let buf_ptr = usb_dwc3::get_event_buf_ptr();
                        core::arch::asm!("dc ivac, {}", in(reg) buf_ptr);
                        core::arch::asm!("dsb sy", "isb");
                    }
                    let evc = usb_dwc3::read_reg(0xc40c);
                    let dsts = usb_dwc3::get_dsts();
                    if evc != 0 {
                        // Read first event from buffer
                        let ev0 = usb_dwc3::read_event_buf(0);
                        con.puts("[11ev] EVENT! evc="); con.put_hex32(evc);
                        con.puts(" dsts="); con.put_hex32(dsts);
                        con.puts(" ev0="); con.put_hex32(ev0);
                        con.puts("\r\n"); con.flush();
                        usb_dwc3::write_reg(0xc40c, evc);
                        got_event = true;
                        break;
                    }
                    if i % 25 == 0 {
                        let buf0 = usb_dwc3::read_event_buf(0);
                        con.puts("[11ev] t="); con.put_hex8(i as u8);
                        con.puts(" dsts="); con.put_hex32(dsts);
                        con.puts(" evc="); con.put_hex32(evc);
                        con.puts(" buf0="); con.put_hex32(buf0);
                        con.puts("\r\n"); con.flush();
                    }
                }

                con.puts(if got_event { "[11] USB EVENT\r\n" } else { "[11] USB NOEVT\r\n" });
                con.flush();
            }
        }

        // Final diagnostic — DWC3 register state
        {
            let dctl = usb_dwc3::get_dctl();
            let dsts = usb_dwc3::get_dsts();
            let evc = usb_dwc3::read_reg(0xc40c);
            con.puts(">>> dctl="); con.put_hex32(dctl);
            con.puts(" dsts="); con.put_hex32(dsts);
            con.puts(" evc="); con.put_hex32(evc);
            con.puts(" <<<\r\n"); con.flush();
        }

        // Shell
        con.puts("[OK] Boot complete\r\n"); con.flush();
        uart::puts(U, "# ");

        let mut editor = shell::LineEditor::new();
        loop {
            interrupt::poll_timer();

            // USB event polling
            usb_dwc3::poll();

            // USB diagnostic: print register state a few times only
            static mut USB_DIAG_TICK: u32 = 0;
            static mut USB_DIAG_PRINTED: u32 = 0;
            unsafe {
                USB_DIAG_TICK += 1;
                if USB_DIAG_TICK >= 20_000 && USB_DIAG_PRINTED < 3 {
                    USB_DIAG_TICK = 0;
                    USB_DIAG_PRINTED += 1;
                    let ev = usb_dwc3::get_event_count();
                    let su = usb_dwc3::get_setup_count();
                    let evcnt = usb_dwc3::read_reg(0xc40c); // GEVNTCOUNT
                    let dctl = usb_dwc3::get_dctl();
                    let dsts = usb_dwc3::get_dsts();
                    let evaddr = usb_dwc3::read_reg(0xc400); // GEVNTADRLO
                    con.puts("[usb] dctl="); con.put_hex32(dctl);
                    con.puts(" dsts="); con.put_hex32(dsts);
                    con.puts(" evc="); con.put_hex32(evcnt);
                    con.puts(" eva="); con.put_hex32(evaddr);
                    con.puts(" n="); con.put_hex32(ev);
                    con.puts(" su="); con.put_hex32(su);
                    con.puts("\r\n"); con.flush();
                }
            }

            // Power button check
            let (_, st, rt) = spmi::obs_cmd_read(1, 0x0810, 1);
            if st & 1 != 0 && (rt & 1) != 0 {
                reboot::reboot();
            }
            let c = uart::getc_nb(U);
            if c != 0 {
                if editor.feed(c, shell::OutputDest::Uart) == shell::InputAction::Complete {
                    let line = editor.line();
                    if line.len() > 0 {
                        shell::set_output(shell::OutputDest::Uart);
                        shell::execute_command(line);
                    }
                    editor.reset();
                    uart::puts(U, "# ");
                }
            } else {
                for _ in 0..1000 { core::hint::spin_loop(); }
            }
        }
        } // unreachable: full redfin init (kept for later)
    }
}

#[alloc_error_handler]
fn alloc_error(_: core::alloc::Layout) -> ! {
    // PSCI reset on alloc error → device reboots
    unsafe {
        core::arch::asm!(
            "mov x0, #0x0009",
            "movk x0, #0x8400, lsl #16",
            "smc #0",
            out("x0") _,
            out("x1") _,
            out("x2") _,
            out("x3") _,
        );
    }
    loop { unsafe { core::arch::asm!("wfi"); } }
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    // PSCI reset on panic → device reboots = Rust crashed
    unsafe {
        core::arch::asm!(
            "mov x0, #0x0009",
            "movk x0, #0x8400, lsl #16",
            "smc #0",
            out("x0") _,
            out("x1") _,
            out("x2") _,
            out("x3") _,
        );
    }
    loop { unsafe { core::arch::asm!("wfi"); } }
}
