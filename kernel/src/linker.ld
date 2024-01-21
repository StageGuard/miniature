OUTPUT_ARCH(riscv)
ENTRY(kernel_entry)
BASE_ADDRESS = 0x10000000;

SECTIONS
{
    . = BASE_ADDRESS;
    __kernel_start = .;

    stext = .;
    .text : {
        *(.text.entry)
        . = ALIGN(4K);
        *(.text .text.*)
    }

    . = ALIGN(4K);
    etext = .;

    srodata = .;
    .rodata : {
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
    }

    . = ALIGN(4K);
    erodata = .;

    sdata = .;
    .data : {
        *(.data .data.*)
        *(.sdata .sdata.*)
    }

    . = ALIGN(4K);
    edata = .;

    sbss_with_stack = .;
    .bss : {
        *(.bss.stack)
        sbss = .;
        *(.bss.memory)
        *(.bss .bss.*)
        *(.sbss .sbss.*)
    }

    . = ALIGN(4K);
    ebss = .;
    
    __kernel_start = .;

    /DISCARD/ : {
        *(.eh_frame)
    }
}
