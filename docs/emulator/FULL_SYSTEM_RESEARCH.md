# Full-system G1 research

Status: source audit, not kernel boot. The production backend executes armfir
under Unicorn with host SVC services; PRIME_OS is mapped inert. This research
track does not replace that backend or claim a full-system implementation.

## Evidence and artifacts

The local reference is `scratch/research/qemuPrime`, upstream
https://github.com/opcod3/qemuPrime, commit
`32e891a9fc568de3110343c3e35b9ec0d23f6f44` (2021-08-17), VERSION 2.10.1.
The old Gigi1237 URL redirects to this repository. Inspect source rather than
assuming an advertised device is complete. The pinned board source is
https://github.com/opcod3/qemuPrime/blob/32e891a9fc568de3110343c3e35b9ec0d23f6f44/hw/arm/s3c2416.c.

| Component | Source evidence | Research gap |
|---|---|---|
| ARM926 | Board creates arm926 CPU | Kernel CP15/MMU behavior unverified locally |
| RAM | One 32 MiB host buffer at 0x30000000, aliased at 0x32000000/0x34000000/0x36000000 | Verify aliases against hardware documentation |
| Boot | Copies first 8 KiB of MTD into boot region at zero; optional ELF kernel loading | Matching authorized boot/NAND image missing |
| INTC | S3C2416 model wired to IRQ/FIQ | Priority, pending/ack and nested interrupt tests |
| NAND | Model at 0x4E000000, Samsung EC/DA identification | ECC paths incomplete; ready/busy IRQ TODO; geometry/OOB validation |
| LCD | Exynos FIMD reused at 0x4C800000 | Actual G1 register compatibility and RGB565 output |
| GPIO/keys | S3C2416 GPIO at 0x56000000 | Matrix scanning, debounce and power key validation |
| Touch | NT11002 I2C slave, GPIO interrupt | Packet/timing accuracy and calibration |
| Timers/RTC | Exynos PWM and RTC reused | Clock/divisor, alarm and interrupt correctness |
| MEMC/watchdog/ADC | Source files present, board creation commented out | Models are not verified merely by existing |
| Power | No local boot/power-cycle evidence | Reset causes, sleep, wake and persistent NAND |

Existing supplied assets include PRIME_OS.ROM, PRIME_APP.DAT, PRIME_MASTER.DAT
and extracted armfir.elf. No BXCBOOT/BESTAARM boot image or complete physical
NAND/OOB backup was found in the inspected configured research assets. These
update components must not be assumed to form a bootable NAND image. Their
presence does not establish redistribution rights. No proprietary image was
changed, fetched or committed during this continuation.

## Milestones and acceptance

1. Reproduce the pinned fork build in a separate build directory; record compiler,
   Python, GLib/pixman and configure versions and every compatibility patch.
   The pinned configure script explicitly requires Python >=2.6 and <3; modern
   Python-only hosts cannot configure it unchanged. QEMU 2.10-era APIs/toolchains
   are a maintenance risk. No build success is
   claimed here, and a modern-QEMU port is not a prerequisite for the Rust work.
2. Run self-authored ARM926 smoke code: reset entry, RAM aliases, UART, interrupt
   acknowledgement and timer delivery. Add device tests using QEMU's testing
   facilities: https://www.qemu.org/docs/master/devel/testing/main.html.
3. Inventory explicitly supplied boot artifacts with hashes, source, geometry and
   OOB requirements. Refuse incomplete images; keep source media read-only and
   confine simulated writes to disposable overlays.
4. Minimum kernel milestone: execute the matching boot chain into PRIME_OS,
   demonstrate guest kernel instruction/exception progress and interrupts, and
   retain UART/CPU/MMIO evidence. A direct ELF or synthetic smoke boot is a
   separate result, not proof of the calculator's real boot chain.
5. Only then attempt storage mounting, armfir launch, Home/input, OFF/restart
   and the shared differential scenarios. Report each stage separately.

Use QMP/GDB/artifact files across a process boundary for any future adapter.
The differential runner currently reports full-system unavailable. Reuse source
identities, fixture steps, value/error observations and pixel comparisons;
never substitute HLE results for a missing full-system result.

QEMU's LICENSE/COPYING and individual device-file notices must be reviewed and
retained with any derivative distribution. Keep QEMU changes in their licensed
research component; do not copy GPL device implementations into MIT Rust crates.
Also record Unicorn and binding licenses separately. This is a provenance and
packaging requirement, not a conclusion about firmware distribution permission.
