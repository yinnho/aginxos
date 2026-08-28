/*
 * MMU enable for Pixel 5 (redfin)
 *
 * x0 = page table base (16KB aligned, after kernel_end)
 *
 * Builds identity-mapped page tables and enables MMU.
 * Uses T0SZ=16 (48-bit VA) to match ABL's TCR.
 *
 * Page table layout:
 *   [base + 0x0000] L0 (512 entries, 4KB)
 *   [base + 0x1000] L1 (512 entries, 4KB)
 *   [base + 0x2000] L2 (512 entries, 4KB)
 *
 * Mappings:
 *   L2_MMIO[0..127]  = Device  0x00000000-0x0FFFFFFF (256MB)
 *   L2_RAM[0..15]    = Normal  0x80000000-0x81FFFFFF (32MB kernel)
 *   L2_RAM[256..263] = Normal  0xA0000000-0xA0FFFFFF (16MB framebuffer)
 */

.global enable_mmu_redfin
.type enable_mmu_redfin, @function
.align 4
enable_mmu_redfin:
    /* Save page table base */
    mov x8, x0

    /* Zero all 3 tables (12KB = 1536 entries) */
    mov x9, #0
    mov x10, #1536
1:
    str x9, [x8], #8
    subs x10, x10, #1
    b.ne 1b
    mov x8, x0          /* restore base */

    /* L1 table at base + 0x1000 */
    add x1, x8, #0x1000
    /* L2 table at base + 0x2000 */
    add x2, x8, #0x2000

    /* L0[0] → L1 table (covers VA 0x00000000-0x7FFFFFFFFF) */
    orr x3, x1, #3      /* table descriptor: valid + table */
    str x3, [x8]

    /* L1[0] → L2 table (covers VA 0x00000000-0x3FFFFFFF) */
    orr x3, x2, #3
    str x3, [x1]

    /* L1[2] → same L2 table (covers VA 0x80000000-0xBFFFFFFF) */
    /* So both MMIO and RAM go through the same L2 */
    str x3, [x1, #16]   /* L1[2] */

    /* Fill L2: 0x00000000-0x0FFFFFFF as Device (128 entries) */
    /* Block: valid=1, AttrIndx=001(Device), SH=11, AF=1 */
    /* = 0x000000000000070D */
    mov x3, #0x70D
    mov x4, #0           /* PA counter */
    mov x5, #128
2:
    orr x6, x4, x3
    str x6, [x2], #8
    add x4, x4, #0x200000    /* next 2MB */
    subs x5, x5, #1
    b.ne 2b

    /* L2[256..271] = Normal NC for framebuffer 0xA0000000+ */
    /* Block: valid=1, AttrIndx=010(NC), SH=11, AF=1 = 0x0000000000000709 */
    /* Using AttrIndx=1 (MAIR[1]=Normal NC) = bits[4:2]=001 = 0x004 | 0x301 = 0x705 */
    mov x3, #0x705
    mov x4, #0xA0000000
    add x2, x8, #0x2000   /* reset L2 pointer */
    mov x5, #256
3:
    add x2, x2, #8        /* skip to index 256... this is wrong, need to compute */
    subs x5, x5, #1
    b.ne 3b

    /* Actually, let me just index directly */
    add x2, x8, #0x2000
    mov x3, #0x705         /* Normal NC: AttrIndx=1, SH=11, AF=1 */
    mov x4, #0xA0000000
    mov x5, #8             /* 8 entries = 16MB */
    add x2, x2, #256*8     /* x2 = &L2[256] */
4:
    str x4, [x2], #8
    orr x6, x4, x3
    str x6, [x2, #-8]      /* oops, overwrite */
    add x4, x4, #0x200000
    subs x5, x5, #1
    b.ne 4b

    /* L2[0..15] = Normal for kernel 0x80000000+ */
    /* Using AttrIndx=3 (MAIR[3]=0xFF Normal WB) = bits[4:2]=011 = 0x00C | ... = 0x70D */
    /* Wait, let me be more careful:
     * bit 0 = 1 (valid)
     * bit 1 = 0 (block, not table)
     * bits[4:2] = AttrIndx = 3 → 0b011 → (3 << 2) = 0x0C
     * bits[9:8] = SH = 3 → (3 << 8) = 0x300
     * bit 10 = AF = 1 → (1 << 10) = 0x400
     * Total lower bits = 0x001 | 0x00C | 0x300 | 0x400 = 0x70D
     */
    add x2, x8, #0x2000   /* L2 base */
    mov x3, #0x70D         /* Normal WB (AttrIndx=3), SH=11, AF=1 */
    mov x4, #0x80000000    /* PA start */
    mov x5, #16            /* 16 entries = 32MB */
5:
    orr x6, x4, x3
    str x6, [x2], #8
    add x4, x4, #0x200000
    subs x5, x5, #1
    b.ne 5b

    /* Ensure all writes are visible */
    dsb sy

    /* Set up system registers */
    /* MAIR: [0]=0xFF Normal WB, [1]=0x04 Device nGnRE, [2]=0x44 Normal NC */
    ldr x1, =0x004404FF
    msr mair_el1, x1

    /* TTBR0 = our page table */
    msr ttbr0_el1, x8
    dsb nsh
    tlbi vmalle1
    dsb nsh
    isb

    /* TCR: T0SZ=16, IPS=5 (48-bit PA), SH0=3, ORGN0=1, IRGN0=1, TG0=0 */
    /* T0SZ=16: bits[5:0] = 0x10 */
    /* IPS=5: bits[34:32] = 0x5 → value = 5<<32 = 0x500000000 */
    /* SH0=3: bits[13:12] = 0x3 → 0x3000 */
    /* ORGN0=1: bits[11:10] = 0x1 → 0x400 */
    /* IRGN0=1: bits[9:8] = 0x1 → 0x100 */
    /* Total = 0x0000000500003510 */
    ldr x1, =0x0000000500003510
    msr tcr_el1, x1
    isb

    /* Invalidate caches */
    ic iallu
    dsb ish
    isb

    /* Enable MMU only (M=1, C=0, I=0) */
    mrs x1, sctlr_el1
    orr x1, x1, #1
    msr sctlr_el1, x1
    isb

    /* If we get here, MMU is on! */
    /* Write a green pixel at (300, 200) to confirm */
    ldr x1, =0xA0000000
    mov x2, #300
    mov x3, #1080
    mul x2, x2, x3
    add x2, x2, #200
    lsl x2, x2, #2        /* * 4 (bytes per pixel) */
    add x1, x1, x2
    mov w3, #0x0000FF00   /* green (BGRA) */
    str w3, [x1]

    /* Halt */
6:
    wfi
    b 6b
