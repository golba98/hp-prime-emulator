/* SPDX-License-Identifier: GPL-2.0-or-later */
.syntax unified
.cpu arm926ej-s
.arm
.section .text.start,"ax"
.global _start
_start:
    ldr sp, =0x3000f000
    bl guest_main
.global nand_halt
nand_halt:
    b nand_halt
