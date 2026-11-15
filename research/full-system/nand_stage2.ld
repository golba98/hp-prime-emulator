ENTRY(_stage2)
SECTIONS { /DISCARD/ : { *(.ARM.exidx*) *(.ARM.extab*) } . = 0x30010000; .text : { *(.text*) *(.rodata*) } }
