ENTRY(_start)
SECTIONS {
    /DISCARD/ : { *(.ARM.exidx*) *(.ARM.extab*) }
    . = 0x30000000;
    .text : { *(.text*) *(.rodata*) }
    .data : { *(.data*) }
    .bss : { *(.bss*) *(COMMON) }
    ASSERT(. < 0x30001000, "display guest overlaps mailbox")
}
