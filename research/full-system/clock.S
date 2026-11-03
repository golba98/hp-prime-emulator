/* Self-authored ARM926 clock/reset control. GPL-2.0-or-later.
 * QEMU -icount shift=3 supplies 8 virtual ns per guest instruction.
 * Two identical 6000-iteration SUBS/BNE loops measure independent timer deltas.
 * No host clock writes, guest result writes or PC changes are used.
 */
.syntax unified
.cpu arm926ej-s
.arm
.section .text
.global _start
.global hardware_halt
_start:
    msr cpsr_c, #0xd3
    ldr sp, =0x3000c000
    ldr r4, =0x30001000
    ldr r5, =0x4c000000
    ldr r6, =0x51000000
    ldr r7, =0x50000000
    ldr r0, [r4, #252]
    ldr r1, =0x19b00719
    cmp r0, r1
    beq after_reset
    /* Cold register observations before any reconfiguration. */
    ldr r0, [r5, #0x24]
    str r0, [r4, #40]
    ldr r0, [r5, #0x20]
    str r0, [r4, #44]
    mov r0, #0
    str r0, [r5, #0x24]  /* XTI12MHz -> HCLK/PCLK12MHz. */
    mov r0, #3
    str r0, [r7]         /* UART8N1. */
    mov r0, #5
    str r0, [r7, #4]
    mov r0, #'C'
    str r0, [r7, #0x20]
    /* PCLK12MHz / prescaler6 / divider2 = 1MHz timer. */
    mov r0, #5
    str r0, [r6]
    mov r0, #0
    str r0, [r6, #4]
    ldr r0, =60000
    str r0, [r6, #0xc]
    mov r0, #2
    str r0, [r6, #8]
    mov r0, #1
    str r0, [r6, #8]
    bl measure
    str r0, [r4, #4]
    mov r0, #'S'
    str r0, [r7, #0x20]
    mov r0, #4
    str r0, [r5, #0x24]  /* PCLK6MHz, active timer must preserve its count. */
    bl measure
    str r0, [r4, #8]
    ldr r0, [r5, #0x34]
    bic r0, r0, #0x400
    str r0, [r5, #0x34]  /* PWM gate must not gate UART. */
    bl measure
    str r0, [r4, #12]
    mov r0, #'R'
    str r0, [r7, #0x20]
    ldr r0, [r5, #0x34]
    orr r0, r0, #0x400
    bic r0, r0, #1
    str r0, [r5, #0x34]
    mov r0, #'X'         /* UART gated: this character must be absent. */
    str r0, [r7, #0x20]
    ldr r0, [r5, #0x34]
    orr r0, r0, #1
    str r0, [r5, #0x34]
    mov r0, #'Y'
    str r0, [r7, #0x20]
    ldr r0, =0x19abcdef
    str r0, [r5, #0x70]
    ldr r0, =0x19b00719
    str r0, [r4, #252]
    ldr r0, =0x533c2416
    str r0, [r5, #0x44]
1:  b 1b
measure:
    ldr r2, [r6, #0x14]
    ldr r3, =6000
1:  subs r3, r3, #1
    bne 1b
    ldr r0, [r6, #0x14]
    sub r0, r2, r0
    bx lr
after_reset:
    ldr r0, [r5, #0x70]
    str r0, [r4, #20]
    ldr r0, [r5, #0x24]
    str r0, [r4, #24]
    ldr r0, [r5, #0x20]
    str r0, [r4, #28]
    ldr r0, [r6, #8]
    str r0, [r4, #32]
    ldr r0, [r5, #0x68]
    str r0, [r4, #36]
    mov r0, #'Z'
    str r0, [r7, #0x20]
    ldr r0, =0xfeed0019
    str r0, [r4, #16]
    ldr r0, =0x600d0019
    str r0, [r4]
hardware_halt:
    b hardware_halt
