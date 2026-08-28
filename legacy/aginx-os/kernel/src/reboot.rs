//! Qualcomm PSHOLD-based reboot driver for redfin (Pixel 5 / SM7225)
//!
//! The `qcom,pshold` hardware at physical address 0xC2640000 triggers a
//! system-level power reset when written. This returns the device to the
//! bootloader (fastboot mode after a normal reboot).

/// Reboot the system by writing to the PSHOLD register.
/// This never returns — the CPU resets immediately.
#[inline(never)]
#[allow(dead_code)]
pub fn reboot() -> ! {
    // MOVK x8, #0xc264, lsl #16 → x8 = 0xC2640000 (PSHOLD on SM7225)
    // STR wzr, [x8] × 2 → write 0 to PSHOLD → triggers reset
    // ISB + B #-4 → loop in case reset is delayed
    //
    // NOTE: inline asm avoids LLVM miscompiling ptr::write_volatile
    // with MOVN (sign-extends upper bits, giving 0xFFFFFFFF3D9BFFFF).
    unsafe {
        core::arch::asm!(
            // MOVK X8, #0xC264, LSL #16  (encoding: d2 80 4c f2)
            ".byte 0xd2, 0x80, 0x4c, 0xf2",
            // STR wzr, [X8]  (encoding: 3f 00 00 b9)
            ".byte 0x3f, 0x00, 0x00, 0xb9",
            // STR wzr, [X8]
            ".byte 0x3f, 0x00, 0x00, 0xb9",
            // ISB  (encoding: df 3f 03 d5)
            ".byte 0xdf, 0x3f, 0x03, 0xd5",
            // B #-4  (encoding: fd ff ff 17)
            ".byte 0xfd, 0xff, 0xff, 0x17",
            options(noreturn, nostack, preserves_flags, raw)
        );
    }
    #[allow(unreachable_code)]
    loop {
        core::hint::spin_loop();
    }
}
