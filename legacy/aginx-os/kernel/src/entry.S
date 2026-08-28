/*
 * Aginx OS - aarch64 entry point and exception vector table
 *
 * QEMU virt: loaded as ELF at 0x40000000, no Linux Image header needed
 *             but we include it for consistency — QEMU -kernel skips it.
 * Pixel 5:    loaded as flat binary with Linux Image header.
 *
 * Sections use .text.* names so the QEMU linker.ld .text catch-all picks them up.
 * The redfin linker script uses .init.* sections explicitly.
 */

    .global NEW_TASK_SP
    .global OLD_TASK_SP_PTR

    /* ── Boot / Entry code ── */
    .section .text._start, "ax", %progbits
    .global _start
    .type _start, %function
_start:
    /* ── ARM64 Linux Image Header (64 bytes) ── */
    b .Lkernel_entry            /* 0x00: branch to real entry */
    .long 0                     /* 0x04: code1 */
    .quad 0x80000               /* 0x08: text_offset */
    .quad _kernel_end - _start  /* 0x10: image_size (patched post-build for redfin) */
    .quad 0                     /* 0x18: flags */
    .quad 0                     /* 0x20: res2 */
    .quad 0                     /* 0x28: res3 */
    .quad 0                     /* 0x30: res4 */
    .long 0x644d5241            /* 0x38: magic "ARM\x64" */
    .long 0                     /* 0x3c: PE header offset */

.Lkernel_entry:
    /* Save DTB address (x0) from bootloader */
    adrp x6, _dtb_ptr
    str x0, [x6, #:lo12:_dtb_ptr]

    /* Set up stack — __stack_top is a linker symbol (value, not pointer) */
    ldr x2, =__stack_top
    mov sp, x2

    /* Set VBAR_EL1 to vector table */
    ldr x3, =_vector_table
    msr VBAR_EL1, x3

    /* Enable FP/SIMD (CPACR_EL1 FPEN = 0b11, bits [21:20]) */
    movz x9, #0x3
    movk x9, #0x0, lsl #16
    lsl x9, x9, #20
    msr CPACR_EL1, x9
    isb

    /* Zero BSS: from __bss_start to _kernel_end
     * Skip if _redfin_skip_bss == 1 (redfin defers to after MMU setup) */
    adrp x11, _redfin_skip_bss
    ldr w11, [x11, #:lo12:_redfin_skip_bss]
    cbnz w11, .Lno_bss

    adrp x4, __bss_start
    add x4, x4, #:lo12:__bss_start
    adrp x5, _kernel_end
    add x5, x5, #:lo12:_kernel_end
    sub x5, x5, x4
    cbz x5, .Lno_bss
    and x5, x5, #~7
.Lbss_loop:
    stp xzr, xzr, [x4], #16
    subs x5, x5, #16
    b.gt .Lbss_loop
.Lno_bss:

    /* Pass kernel_end address to rust_main */
    adrp x0, _kernel_end
    add x0, x0, #:lo12:_kernel_end
    mov x1, xzr               /* page_table_arg */
    mov x2, xzr               /* unused */
    bl rust_main

    /* rust_main never returns */
.Lhalt:
    wfi
    b .Lhalt

    /* ── Exception Vector Table ── */
    .section .text.vector_table, "ax", %progbits
    .balign 0x800

    .global _vector_table
    .type _vector_table, %function
_vector_table:

    /* Current EL with SPx */
    .balign 0x80
    b hang
    .balign 0x80
    b hang
    .balign 0x80
    b hang
    .balign 0x80
    b hang

    /* Current EL with SPx — sync + IRQ handlers */
    .balign 0x80
    b exc_sync_spx
    .balign 0x80
    b exc_irq_spx
    .balign 0x80
    b hang
    .balign 0x80
    b hang

    /* Lower EL using AArch64 — EL0 → EL1 exception handlers */
    .balign 0x80
    b exc_sync_spx         /* 0x400: sync exception from EL0 (reuse EL1 handler) */
    .balign 0x80
    b exc_irq_spx          /* 0x480: IRQ from EL0 (reuse EL1 handler) */
    .balign 0x80
    b hang
    .balign 0x80
    b hang

    /* Lower EL using AArch32 */
    .balign 0x80
    b hang
    .balign 0x80
    b hang
    .balign 0x80
    b hang
    .balign 0x80
    b hang

    /* ── Hang → PSCI reset (only works on real hardware) ── */
hang:
    movz x0, #0x0009
    movk x0, #0x8400, lsl #16
    smc #0
    wfe
    b hang

    /* ── Synchronous Exception (Current EL SPx) ── */
exc_sync_spx:
    /* Save all registers */
    sub sp, sp, #272

    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    stp x30, xzr, [sp, #240]

    mrs x0, ELR_EL1
    mrs x1, SPSR_EL1
    stp x0, x1, [sp, #256]

    /* Read ESR_EL1 to determine exception class */
    mrs x19, ESR_EL1
    lsr x20, x19, #26          /* EC = ESR[31:26] */
    cmp x20, #0x15             /* EC=0x15 = SVC from Lower EL (AArch64) */
    b.eq .Lsvc_handler_el1

    /* Not SVC — treat as fatal */
    bl hang

.Lsvc_handler_el1:
    /* Read saved x8 (syscall number) from exception frame */
    ldr x0, [sp, #64]          /* x0 = syscall number (from saved x8) */
    ldr x1, [sp, #0]           /* x1 = orig x0 (arg0) */
    ldr x2, [sp, #8]           /* x2 = orig x1 (arg1) */
    ldr x3, [sp, #16]          /* x3 = orig x2 (arg2) */

    /* Call Rust syscall dispatcher: handle_syscall(nr, x0, x1, x2, ...) */
    bl handle_syscall

    /* Store return value in x0 position */
    str x0, [sp, #0]

    /* Check if syscall 99 (SYS_RETURN_TO_KERNEL) */
    ldr x8, [sp, #64]          /* reload x8 = syscall number */
    cmp x8, #99
    b.ne .Lsvc_normal_el1

    /* Special return: use saved ELR/SPSR globals */
    adrp x0, SAVED_ELR_FOR_EL0
    ldr x0, [x0, #:lo12:SAVED_ELR_FOR_EL0]
    adrp x1, SAVED_SPSR_FOR_EL0
    ldr x1, [x1, #:lo12:SAVED_SPSR_FOR_EL0]
    msr ELR_EL1, x0
    msr SPSR_EL1, x1
    b .Lsvc_restore_el1

.Lsvc_normal_el1:
    /* For SVC, advance ELR by 4 to skip the SVC instruction */
    ldp x0, x1, [sp, #256]
    add x0, x0, #4
    stp x0, x1, [sp, #256]

    /* Restore and return */
    ldp x0, x1, [sp, #256]
    msr ELR_EL1, x0
    msr SPSR_EL1, x1

.Lsvc_restore_el1:

    /* If returning to EL0, set SP_EL0 from CURRENT_USER_SP */
    ldp x4, x5, [sp, #256]     /* x4=ELR, x5=SPSR */
    tbnz x5, #0, 1f            /* M[0]=1 → not EL0t */
    tbnz x5, #1, 1f            /* M[1]=1 → not EL0t */
    adrp x6, CURRENT_USER_SP
    ldr x6, [x6, #:lo12:CURRENT_USER_SP]
    msr SP_EL0, x6
1:

    ldp x30, xzr, [sp, #240]
    ldp x28, x29, [sp, #224]
    ldp x26, x27, [sp, #208]
    ldp x24, x25, [sp, #192]
    ldp x22, x23, [sp, #176]
    ldp x20, x21, [sp, #160]
    ldp x18, x19, [sp, #144]
    ldp x16, x17, [sp, #128]
    ldp x14, x15, [sp, #112]
    ldp x12, x13, [sp, #96]
    ldp x10, x11, [sp, #80]
    ldp x8, x9, [sp, #64]
    ldp x6, x7, [sp, #48]
    ldp x4, x5, [sp, #32]
    ldp x2, x3, [sp, #16]
    ldp x0, x1, [sp, #0]
    add sp, sp, #272
    eret

    /* ── Sync Exception from EL0 (0x400) ── */
    /* Handles both SVC (syscall) and faults (segfault, etc.) */
exc_sync_el0:
    sub sp, sp, #272

    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    stp x30, xzr, [sp, #240]

    mrs x0, ELR_EL1
    mrs x1, SPSR_EL1
    stp x0, x1, [sp, #256]

    /* ── TTBR0 switch deferred to when user page tables are active ── */

    /* Read ESR_EL1 to determine exception class */
    mrs x19, ESR_EL1
    lsr x20, x19, #26          /* EC = ESR[31:26] */
    cmp x20, #0x15             /* EC=0x15 = SVC from AArch64 */
    b.eq .Lsvc_handler_el0

    /* Not SVC → EL0 fault (segfault, alignment, etc.) */
    /* Call Rust fault handler: handle_el0_fault(esr, elr, far) */
    mrs x0, ESR_EL1
    ldr x1, [sp, #256]         /* ELR (fault address context) */
    mrs x2, FAR_EL1            /* Faulting virtual address */
    bl handle_el0_fault

    /* Fault handler marks task as Dead and does not return to EL0 */
    /* Fall through to restore — scheduler will pick a new task */
    b .Lsvc_restore_el0

.Lsvc_handler_el0:
    /* SVC (syscall) path */
    ldr x0, [sp, #64]          /* x0 = syscall number (from saved x8) */
    ldr x1, [sp, #0]           /* x1 = orig x0 (arg0) */
    ldr x2, [sp, #8]           /* x2 = orig x1 (arg1) */
    ldr x3, [sp, #16]          /* x3 = orig x2 (arg2) */

    bl handle_syscall

    /* Store return value in x0 position */
    str x0, [sp, #0]

    /* Check if syscall 99 (SYS_RETURN_TO_KERNEL) */
    ldr x8, [sp, #64]
    cmp x8, #99
    b.ne .Lsvc_normal_el0

    /* Return to kernel — use saved globals */
    adrp x0, SAVED_ELR_FOR_EL0
    ldr x0, [x0, #:lo12:SAVED_ELR_FOR_EL0]
    adrp x1, SAVED_SPSR_FOR_EL0
    ldr x1, [x1, #:lo12:SAVED_SPSR_FOR_EL0]
    msr ELR_EL1, x0
    msr SPSR_EL1, x1
    b .Lsvc_restore_el0

.Lsvc_normal_el0:
    /* Advance ELR by 4 to skip SVC instruction */
    ldp x0, x1, [sp, #256]
    add x0, x0, #4
    stp x0, x1, [sp, #256]

.Lsvc_restore_el0:
    /* Restore ELR/SPSR */
    ldp x0, x1, [sp, #256]
    msr ELR_EL1, x0
    msr SPSR_EL1, x1

.Lel0_restore_regs:
    ldp x30, xzr, [sp, #240]
    ldp x28, x29, [sp, #224]
    ldp x26, x27, [sp, #208]
    ldp x24, x25, [sp, #192]
    ldp x22, x23, [sp, #176]
    ldp x20, x21, [sp, #160]
    ldp x18, x19, [sp, #144]
    ldp x16, x17, [sp, #128]
    ldp x14, x15, [sp, #112]
    ldp x12, x13, [sp, #96]
    ldp x10, x11, [sp, #80]
    ldp x8, x9, [sp, #64]
    ldp x6, x7, [sp, #48]
    ldp x4, x5, [sp, #32]
    ldp x2, x3, [sp, #16]
    ldp x0, x1, [sp, #0]
    add sp, sp, #272
    eret

    /* ── IRQ Handler (Current EL SPx) ── */
exc_irq_spx:
    sub sp, sp, #272

    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    stp x30, xzr, [sp, #240]

    mrs x0, ELR_EL1
    mrs x1, SPSR_EL1
    stp x0, x1, [sp, #256]

    bl handle_irq

    /* Context switch */
    adrp x2, NEW_TASK_SP
    ldr x2, [x2, #:lo12:NEW_TASK_SP]
    cbz x2, .Lno_switch

    adrp x3, OLD_TASK_SP_PTR
    ldr x3, [x3, #:lo12:OLD_TASK_SP_PTR]
    mov x4, sp
    str x4, [x3]
    mov sp, x2
    adrp x2, NEW_TASK_SP
    str xzr, [x2, #:lo12:NEW_TASK_SP]

.Lno_switch:
    ldp x0, x1, [sp, #256]
    msr ELR_EL1, x0
    msr SPSR_EL1, x1

    /* If returning to EL0, set SP_EL0 from CURRENT_USER_SP */
    tbnz x1, #0, .Lel1_irq_restore_regs   /* M[0]=1 → not EL0t, skip */
    tbnz x1, #1, .Lel1_irq_restore_regs   /* M[1]=1 → not EL0t, skip */
    adrp x4, CURRENT_USER_SP
    ldr x4, [x4, #:lo12:CURRENT_USER_SP]
    msr SP_EL0, x4

.Lel1_irq_restore_regs:
    ldp x30, xzr, [sp, #240]
    ldp x28, x29, [sp, #224]
    ldp x26, x27, [sp, #208]
    ldp x24, x25, [sp, #192]
    ldp x22, x23, [sp, #176]
    ldp x20, x21, [sp, #160]
    ldp x18, x19, [sp, #144]
    ldp x16, x17, [sp, #128]
    ldp x14, x15, [sp, #112]
    ldp x12, x13, [sp, #96]
    ldp x10, x11, [sp, #80]
    ldp x8, x9, [sp, #64]
    ldp x6, x7, [sp, #48]
    ldp x4, x5, [sp, #32]
    ldp x2, x3, [sp, #16]
    ldp x0, x1, [sp, #0]
    add sp, sp, #272
    eret

    /* ── IRQ from EL0 ── */
exc_irq_el0:
    sub sp, sp, #272

    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    stp x30, xzr, [sp, #240]

    mrs x0, ELR_EL1
    mrs x1, SPSR_EL1
    stp x0, x1, [sp, #256]

    /* Switch back to kernel page table */
    adrp x0, KERNEL_TTBR0
    ldr x0, [x0, #:lo12:KERNEL_TTBR0]
    msr TTBR0_EL1, x0
    dsb nsh
    isb

    bl handle_irq

    /* Context switch */
    adrp x2, NEW_TASK_SP
    ldr x2, [x2, #:lo12:NEW_TASK_SP]
    cbz x2, .Lno_switch_el0_irq

    adrp x3, OLD_TASK_SP_PTR
    ldr x3, [x3, #:lo12:OLD_TASK_SP_PTR]
    mov x4, sp
    str x4, [x3]
    mov sp, x2
    adrp x2, NEW_TASK_SP
    str xzr, [x2, #:lo12:NEW_TASK_SP]

.Lno_switch_el0_irq:
    ldp x0, x1, [sp, #256]
    msr ELR_EL1, x0
    msr SPSR_EL1, x1

.Lel0_irq_restore_regs:
    ldp x30, xzr, [sp, #240]
    ldp x28, x29, [sp, #224]
    ldp x26, x27, [sp, #208]
    ldp x24, x25, [sp, #192]
    ldp x22, x23, [sp, #176]
    ldp x20, x21, [sp, #160]
    ldp x18, x19, [sp, #144]
    ldp x16, x17, [sp, #128]
    ldp x14, x15, [sp, #112]
    ldp x12, x13, [sp, #96]
    ldp x10, x11, [sp, #80]
    ldp x8, x9, [sp, #64]
    ldp x6, x7, [sp, #48]
    ldp x4, x5, [sp, #32]
    ldp x2, x3, [sp, #16]
    ldp x0, x1, [sp, #0]
    add sp, sp, #272
    eret

    /* ── Global data ── */
    .section .data
    .global _dtb_ptr
    .balign 8
_dtb_ptr:
    .quad 0

    /* ── TTBR0 switching globals ── */
    .global KERNEL_TTBR0
    .global CURRENT_USER_TTBR0
    .global CURRENT_USER_SP
    .balign 8
KERNEL_TTBR0:
    .quad 0
CURRENT_USER_TTBR0:
    .quad 0
CURRENT_USER_SP:
    .quad 0
