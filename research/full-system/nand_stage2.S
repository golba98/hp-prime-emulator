/* SPDX-License-Identifier: GPL-2.0-or-later */
.syntax unified
.cpu arm926ej-s
.arm
.global _stage2
_stage2:
 ldr r0, =0x30001000
 ldr r1, =0x600d0019
 str r1, [r0]
 ldr r1, =0x5a190002
 str r1, [r0,#20]
 ldr r0, =0x50000000
 mov r1, #3
 str r1,[r0]
 ldr r1, =0x245
 str r1,[r0,#4]
 adr r2,message
1: ldrb r1,[r2],#1
 cmp r1,#0
 beq 2f
 str r1,[r0,#32]
 b 1b
.global stage2_halt
2:
stage2_halt: b stage2_halt
message: .asciz "M19 BOOT2\n"
