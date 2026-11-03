ENTRY(_start)
SECTIONS { /DISCARD/ : { *(.ARM.exidx*) *(.ARM.extab*) }
 . = DEFINED(BOOT_STAGE) ? 0 : 0x30000000;
 .text : { *(.text.start) . = DEFINED(BOOT_STAGE) ? 0x1000 : .; *(.text*) *(.rodata*) }
 .data : { *(.data*) }
 .bss : { *(.bss*) *(COMMON) }
 ASSERT(SIZEOF(.text) + SIZEOF(.data) + SIZEOF(.bss) < 8192, "NAND guest exceeds bootstrap")
}
