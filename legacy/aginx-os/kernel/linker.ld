/* Linker script for Aginx OS kernel (aarch64, QEMU virt) */

OUTPUT_FORMAT("elf64-littleaarch64")
OUTPUT_ARCH(aarch64)
ENTRY(_start)

SECTIONS
{
    . = 0x40000000;

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

    .bss (NOLOAD) : {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        __bss_end = .;
    }

    .page_table (NOLOAD) : {
        __page_table_start = .;
        . = . + 0x4000;
        __page_table_end = .;
    }

    /* Stack at fixed address */
    __stack_top = 0x48000000;

    /DISCARD/ : {
        *(.comment)
        *(.note.*)
        *(.eh_frame*)
        *(.ARM*)
    }

    .symtab 0 : { *(.symtab) }
    .strtab 0 : { *(.strtab) }
    .shstrtab 0 : { *(.shstrtab) }
}

/* Export symbols */
_kernel_end = __bss_end;
