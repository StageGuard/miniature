SECTIONS
{
    . = 0x1000;
    __kernel_start = .;

    __stext = .;
    .text : {
        *(.text.entry)
        . = ALIGN(4K);
        *(.text .text.*)
    }
    . = ALIGN(4K);
    __etext = .;

    __srodata = .;
    .rodata : {
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
    }
    . = ALIGN(4K);
    __erodata = .;

    __sdata = .;
    .data : {
        *(.data.heap)
        *(.data .data.*)
        *(.sdata .sdata.*)
    }
    . = ALIGN(4K);
    __edata = .;

    __sbss = .;
    .bss : {
        *(.bss .bss.*)
        *(.sbss .sbss.*)
    }
    . = ALIGN(4K);
    __ebss = .;
    
    __kernel_end = .;

    /DISCARD/ : {
        *(.eh_frame)
    }
}