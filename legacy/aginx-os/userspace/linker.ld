/* User-space linker script for Aginx OS */
/* Entry point at VA 0x10000 */

ENTRY(_start)

SECTIONS
{
    . = 0x10000;

    .text : {
        *(.text._start)
        *(.text .text.*)
    }

    .rodata : {
        *(.rodata .rodata.*)
    }

    .data : {
        *(.data .data.*)
    }

    .bss : {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        __bss_end = .;
    }

    /DISCARD/ : {
        *(.ARM.attributes)
        *(.comment)
    }
}
