/*
 * Linker script for Aginx OS on Pixel 5 (redfin)
 *
 * ABL loads kernel at DRAM_BASE + text_offset = 0x80000000 + 0x80000 = 0x80080000.
 */
OUTPUT_FORMAT("elf64-littleaarch64")
OUTPUT_ARCH(aarch64)
ENTRY(_start)

SECTIONS
{
    . = 0x80080000;
    _kernel_start = .;

    /* Entry code MUST be first — contains ARM64 Linux Image header */
    .init : {
        *(.text._start)
    } = 0

    /* Vector table at 2KB alignment */
    . = ALIGN(0x800);
    .init.vector : {
        *(.text.vector_table)
    } = 0

    /* All other code */
    .text : {
        *(.text .text.*)
    } = 0

    .rodata : {
        *(.rodata .rodata.*)
    }

    .data : {
        *(.data .data.*)
    }

    .bss (NOLOAD) : {
        *(.bss .bss.*)
        *(COMMON)
    }

    /* rust-lld doesn't advance . after NOLOAD — compute manually */
    __bss_start = ADDR(.bss);
    __bss_end = ADDR(.bss) + SIZEOF(.bss);
    _kernel_end = __bss_end;

    __stack_top = 0x88000000;

    /DISCARD/ : {
        *(.comment)
        *(.note.*)
        *(.eh_frame*)
        *(.ARM*)
    }
}
