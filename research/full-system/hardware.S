/* Self-authored ARMv5/ARM926 synthetic hardware tests. No calculator code.
 * Build CASE=1 IRQ, 2 FIQ, 3 UART/GPIO, 4 MMU/abort.
 * Mailbox30001000: magic, irq_count, fiq_count, abort_count, main_return,
 * masked_count, cpu_masked_count, gpio readbacks, fault_status/address,
 * fault_count, then sixteen handler records starting word16.
 */
.syntax unified
.cpu arm926ej-s
.arm
.section .text
.global _start
_start:
    msr cpsr_c, #0xd2
    ldr sp, =0x3000f000
    msr cpsr_c, #0xd1
    ldr sp, =0x3000e000
    msr cpsr_c, #0xd7
    ldr sp, =0x3000d000
    msr cpsr_c, #0xd3
    ldr sp, =0x3000c000
    /* Low vectors: ldr pc,[pc,#24], addresses in adjacent literal table. */
    ldr r0, =vectors
    mov r1, #0
    mov r2, #16
1:  ldr r3, [r0], #4
    str r3, [r1], #4
    subs r2, r2, #1
    bne 1b
    ldr r4, =0x30001000
    mov r0, #0
    mov r1, #64
    mov r2, r4
1:  str r0, [r2], #4
    subs r1, r1, #1
    bne 1b
    ldr r5, =0x4a000000
    mvn r0, #0
    str r0, [r5, #8]
    str r0, [r5, #0x1c]
    str r0, [r5]
    str r0, [r5, #0x10]
    mov r0, #0
    str r0, [r5, #4]
    str r0, [r5, #0x30]
    str r0, [r5, #0x34]
    ldr r6, =0x51000000
#if CASE == 1
    mov r0, #65
    str r0, [r6]
    mov r0, #0
    str r0, [r6, #4]
    ldr r0, =999
    str r0, [r6, #0xc]
    mov r0, #2
    str r0, [r6, #8]
    mvn r0, #0x400
    str r0, [r5, #8]
    msr cpsr_c, #0x53
    mov r0, #9
    str r0, [r6, #8]
irq_wait5:
    ldr r0, [r4, #4]
    cmp r0, #5
    beq masked_test
    mcr p15, 0, r0, c7, c0, 4
    b irq_wait5
masked_test:
    mvn r0, #0
    str r0, [r5, #8]
    bl arm_oneshot
1:  ldr r0, [r5]
    tst r0, #0x400
    beq 1b
    ldr r0, [r4, #4]
    str r0, [r4, #20]
    mvn r0, #0x400
    str r0, [r5, #8]
1:  ldr r0, [r4, #4]
    cmp r0, #6
    bne 1b
    msr cpsr_c, #0xd3
    bl arm_oneshot
1:  ldr r0, [r5, #0x10]
    tst r0, #0x400
    beq 1b
    ldr r0, [r4, #4]
    str r0, [r4, #24]
    msr cpsr_c, #0x53
1:  ldr r0, [r4, #4]
    cmp r0, #7
    bne 1b
    /* Two already pending IRQs: fixed arbitration setting, no guessed FIQ priority. */
    msr cpsr_c, #0xd3
    mvn r0, #0
    str r0, [r5, #8]
    mov r0, #0
    str r0, [r6, #8]
    mov r0, #99
    str r0, [r6, #0xc]
    str r0, [r6, #0x18]
    ldr r0, =0x202
    str r0, [r6, #8]
    ldr r0, =0x101
    str r0, [r6, #8]
1:  ldr r0, [r5]
    and r0, r0, #0xc00
    cmp r0, #0xc00
    bne 1b
    mvn r0, #0xc00
    str r0, [r5, #8]
    msr cpsr_c, #0x53
1:  ldr r0, [r4, #4]
    cmp r0, #9
    bne 1b
    b passed
arm_oneshot:
    mov r0, #0
    str r0, [r6, #8]
    mov r0, #99
    str r0, [r6, #0xc]
    mov r0, #2
    str r0, [r6, #8]
    mov r0, #1
    str r0, [r6, #8]
    bx lr
#elif CASE == 2
    mov r0, #65
    str r0, [r6]
    mov r0, #0
    str r0, [r6, #4]
    ldr r0, =999
    str r0, [r6, #0xc]
    mov r0, #2
    str r0, [r6, #8]
    mov r0, #0x400
    str r0, [r5, #4]
    mvn r0, #0x400
    str r0, [r5, #8]
    msr cpsr_c, #0x13
    mov r0, #9
    str r0, [r6, #8]
1:  ldr r0, [r4, #8]
    cmp r0, #3
    beq passed
    mcr p15, 0, r0, c7, c0, 4
    b 1b
#elif CASE == 3
    ldr r5, =0x50000000
    mov r0, #3
    str r0, [r5]
    mov r0, #5
    str r0, [r5, #4]
    ldr r1, =uart_text
1:  ldrb r0, [r1], #1
    cmp r0, #0
    beq 2f
    strb r0, [r5, #0x20]
    b 1b
2:  ldr r5, =0x56000000
    ldr r0, =0x5555
    str r0, [r5, #0x10]
    ldr r0, =0xa55a
    str r0, [r5, #0x14]
    ldr r0, [r5, #0x10]
    str r0, [r4, #28]
    ldr r0, [r5, #0x14]
    str r0, [r4, #32]
    b passed
#elif CASE == 4
    /* Empty 4096-entry first-level table, aligned16KiB. */
    ldr r5, =0x30004000
    mov r0, #0
    ldr r1, =4096
    mov r2, r5
1:  str r0, [r2], #4
    subs r1, r1, #1
    bne 1b
    /* Domain0 client; AP=3 RW sections except alias AP=1 (privileged only). */
    ldr r0, =0x30000c02
    str r0, [r5, #0xc00]
    ldr r0, =0x00000c02
    str r0, [r5]
    ldr r0, =0x30000402
    str r0, [r5, #0x400]
    mcr p15, 0, r5, c2, c0, 0
    mov r0, #1
    mcr p15, 0, r0, c3, c0, 0
    mov r0, #0
    mcr p15, 0, r0, c8, c7, 0
    mrc p15, 0, r0, c1, c0, 0
    bic r0, r0, #0x1000
    bic r0, r0, #4
    orr r0, r0, #1
    mcr p15, 0, r0, c1, c0, 0
    mov r0, r0
    mov r0, r0
    ldr r1, =0x10002000
    ldr r0, =0x12345678
    str r0, [r1]
    ldr r2, =0x30002000
    ldr r2, [r2]
    str r2, [r4, #28]
    /* Unmapped section causes a real data abort, handler skips faulting LDR. */
    ldr r1, =0x20000000
    ldr r0, [r1]
    /* Execute the same LDR block with A clear/set/clear (TB identity control). */
    ldr r1, =0x30002001
    bl alignment_probe
    str r0, [r4, #44]
    /* Alignment check produces DFSR alignment fault with MMU still active. */
    mrc p15, 0, r0, c1, c0, 0
    orr r0, r0, #2
    mcr p15, 0, r0, c1, c0, 0
    ldr r1, =0x30002001
    bl alignment_probe
    mrc p15, 0, r0, c1, c0, 0
    str r0, [r4, #32]
    bic r0, r0, #2
    mcr p15, 0, r0, c1, c0, 0
    bl alignment_probe
    str r0, [r4, #48]
    /* An unmapped instruction fetch enters the actual prefetch vector. */
    ldr r0, =0x20000000
    bx r0
after_prefetch:
    /* User access to privileged-only alias; abort restores supervisor mode. */
    ldr r1, =0x10002000
    msr cpsr_c, #0xd0
    ldr r0, [r1]
    b passed
alignment_probe:
    ldr r0, [r1]
    bx lr
#endif
passed:
#if CASE == 1 || CASE == 2
    ldr r1, =0x50000020
    mov r0, #69
    strb r0, [r1]
#endif
    ldr r0, =0x600d0018
    str r0, [r4]
    ldr r0, =0xfeed0018
    str r0, [r4, #16]
.global hardware_halt
hardware_halt:
    b hardware_halt
bad:
    ldr r0, =0xbad00018
    str r0, [r4]
    b hardware_halt

irq_handler:
    stmdb sp!, {r0-r12,lr}
    ldr r4, =0x30001000
    ldr r0, [r4, #4]
    add r0, r0, #1
    str r0, [r4, #4]
    ldr r1, =0x50000020
    mov r2, #73
    strb r2, [r1]
    ldr r5, =0x4a000000
    ldr r1, [r5, #0x14]
    add r2, r4, #64
    str r1, [r2, r0, lsl #2]
    mrs r1, cpsr
    str r1, [r4, #36]
    mrs r1, spsr
    str r1, [r4, #44]
    str lr, [r4, #48]
    str sp, [r4, #52]
    mov r1, #0
    mcr p15, 0, r1, c7, c10, 4
    ldr r1, [r5, #0x10]
    str r1, [r5]
    str r1, [r5, #0x10]
    cmp r0, #5
    bne 1f
    ldr r2, =0x51000000
    mov r1, #0
    str r1, [r2, #8]
1:  ldmia sp!, {r0-r12,lr}
    subs pc, lr, #4
fiq_handler:
    stmdb sp!, {r0-r12,lr}
    ldr r4, =0x30001000
    ldr r0, [r4, #8]
    add r0, r0, #1
    str r0, [r4, #8]
    ldr r1, =0x50000020
    mov r2, #70
    strb r2, [r1]
    mrs r1, cpsr
    str r1, [r4, #40]
    mrs r1, spsr
    str r1, [r4, #44]
    str lr, [r4, #48]
    str sp, [r4, #52]
    ldr r5, =0x4a000000
    mov r1, #0x400
    str r1, [r5]
    str r1, [r5, #0x10]
    cmp r0, #3
    bne 1f
    ldr r2, =0x51000000
    mov r1, #0
    str r1, [r2, #8]
1:  ldmia sp!, {r0-r12,lr}
    subs pc, lr, #4
prefetch_handler:
    stmdb sp!, {r0-r1}
    ldr r0, =0x30001000
    mov r1, #1
    str r1, [r0, #56]
    mrc p15, 0, r1, c5, c0, 1
    str r1, [r0, #60]
    ldmia sp!, {r0-r1}
#if CASE == 4
    ldr lr, =after_prefetch
    movs pc, lr
#else
    b bad
#endif
abort_handler:
    stmdb sp!, {r0-r3}
    ldr r2, =0x30001000
    ldr r0, [r2, #12]
    add r0, r0, #1
    str r0, [r2, #12]
    add r3, r2, #128
    mrc p15, 0, r1, c5, c0, 0
    str r1, [r3, r0, lsl #3]
    add r3, r3, #4
    mrc p15, 0, r1, c6, c0, 0
    str r1, [r3, r0, lsl #3]
    /* Return to SVC if fault interrupted user mode, retaining masked IRQ/FIQ. */
    mrs r0, spsr
    bic r0, r0, #0x1f
    orr r0, r0, #0x13
    msr spsr_cxsf, r0
    ldmia sp!, {r0-r3}
    subs pc, lr, #4
.align 2
vectors:
    .rept 8
    ldr pc, [pc, #24]
    .endr
    .word bad, bad, bad, prefetch_handler, abort_handler, bad, irq_handler, fiq_handler
uart_text: .asciz "M18 UART\n"
