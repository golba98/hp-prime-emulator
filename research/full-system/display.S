/* Self-authored ARM926 entry; no calculator firmware. */
.syntax unified
.cpu arm926ej-s
.arm
.section .text
.global _start
_start:
    msr cpsr_c, #0xd3
    ldr sp, =0x3000f000
    bl display_main
.global display_halt
display_halt:
    b display_halt
