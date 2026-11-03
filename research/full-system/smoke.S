/* Self-authored ARM926 smoke, no proprietary code. Not PRIME_OS boot.
 * Inspect mailbox at 0x30001000 through GDB after loading the linked ELF.
 * 0x600d0001 = alias tests passed; 0xbad00001 = alias mismatch.
 * UART0 is wired to a null chardev by the pinned board by default, so this
 * experiment uses a RAM mailbox rather than promising visible serial output.
 */
.syntax unified
.cpu arm926ej-s
.arm
.section .text
.global _start
_start:
    mrs r0, cpsr
    orr r0, r0, #0xc0
    msr cpsr_c, r0
    ldr r0, =0x30002000
    ldr r1, =0x12345678
    str r1, [r0]
    ldr r2, =0x32002000
    ldr r3, [r2]
    cmp r1, r3
    bne failed
    ldr r2, =0x34002000
    ldr r3, [r2]
    cmp r1, r3
    bne failed
    ldr r2, =0x36002000
    ldr r3, [r2]
    cmp r1, r3
    bne failed
    ldr r1, =0x600d0001
    b report
failed:
    ldr r1, =0xbad00001
report:
    ldr r0, =0x30001000
    str r1, [r0]
1:  b 1b
