ENTRY(_start)
SECTIONS {
    . = 0x30000000;
    .text : { *(.text*) *(.rodata*) }
    .data : { *(.data*) }
    .bss : { *(.bss*) *(COMMON) }
    ASSERT(. < 0x30001000, "hardware guest overlaps result mailbox")
}
