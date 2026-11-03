# M18 full-system hardware execution

This research component executes self-authored ARM926 programs on the pinned
`hp-prime` S3C2416 machine. It is separate from production Rust HLE. No proprietary
bootloader/kernel was executed and no physical calculator was accessed.

## Implemented and executed

The board now instantiates a dedicated `s3c2416-pwm` model instead of the borrowed
Exynos PWM. It implements five 16-bit countdown channels, shared prescalers,
internal /2,/4,/8,/16 dividers, manual load, one-shot/auto-reload, observation,
stop/reset cancellation and pulsed requests to the existing INTC. Clock input is
an explicit `pclk-hz` property, defaulting to the previous model's 66 MHz. This
is a research clock input, not validated G1 PLL/clock-tree emulation. Invalid
clock configurations stop counting with an unsupported diagnostic. There is no
Exynos TINT_CSTAT. External clocks, DMA, output waveform/dead-zone behavior and
whole-machine migration are explicitly unsupported; migration is blocked rather
than silently omitting live timer state.

The exact S3C2416 upstream Linux DT includes s3c24xx.dtsi, inherits the
`samsung,s3c2410-pwm` compatible and only overrides PCLK_PWM. The driver selects
16-bit counters, divisor base1 and no TINT_CSTAT. These are primary-source
contracts rather than assumptions from a similarly named QEMU file:

- https://raw.githubusercontent.com/torvalds/linux/v5.15/arch/arm/boot/dts/s3c2416.dtsi
- https://raw.githubusercontent.com/torvalds/linux/v5.15/arch/arm/boot/dts/s3c24xx.dtsi
- https://raw.githubusercontent.com/torvalds/linux/v5.15/drivers/clocksource/samsung_pwm_timer.c

The original unused S3C timer skeleton merely stored register values. It remains
untouched; the build now selects the new dedicated timer object. This remains GPL research code. Production Rust code contains none of it.

Four bare-metal payload cases execute actual ARM instructions. They install low
vectors and banked stacks through guest instructions; the host debugger observes
only entry/step, terminal breakpoint, registers and RAM. It never supplies an IRQ,
advances the PC to a handler or edits guest test results.

| Case | Executed observations |
| --- | --- |
| `timer_irq` | Nine actual timer IRQ entries/returns: five periodic timer0 interrupts, one pending while controller-masked, one while CPU-I-masked, then two already-pending timer sources under a fixed arbitration setting. Observed offsets10 eight times then11; controller-masked count5, CPU-masked count6. Handler CPSR is IRQ, saved SPSR is SVC, LR lies in payload text and IRQ stack is banked. UART is exactly `IIIIIIIIIE`; final E and mainline marker follow handler return. |
| `timer_fiq` | Three timer0 FIQ entries/returns with zero IRQ handler entries. FIQ/SVC saved state and banked FIQ stack verified, UART exactly `FFFE`. This is single-source FIQ evidence, not simultaneous-FIQ priority or arbitrary IRQ-preemption proof. |
| `uart_gpio` | Guest configures UART and emits exact bytes `M18 UART\n`. Guest writes/reads GPBCON=0x5555 and GPBDAT=0xa55a through MMIO. This verifies modeled register readback, not electrical pins, baud timing or physical key scanning. |
| `mmu` | ARM926 first-level sections enable MMU and alias VA0x10002000 to RAM0x30002000 (0x12345678). Real data-abort records: translation status5/address0x20000000, alignment status1/address0x30002001 and user-permission status13/address0x10002000. Unmapped instruction fetch enters prefetch-abort vector with IFSR5; guest handler returns to a declared recovery label. Same LDR block runs with SCTLR.A clear/set/clear; cache bits remain disabled. |

Every case passed in three separate fresh QEMU processes. Raw mailboxes, GDB
output, instruction-translation logs, UART bytes, QMP shutdown and binary/image/
source hashes are retained. `evidence/m18-hardware.json` records these real
captures; replaying them in normal CI does not itself count as a new execution.

## Root causes and negative evidence

The pre-M18 borrowed Exynos model accepted counter buffer0x123456 unchanged. The
unchanged qtest width regression now fails that M17 binary and passes the new
16-bit model (readback0x3456). Virtual-clock qtest separately covers every timer:
manual load, one-shot, three reload cycles, clear, stop and reset. It also proves
the research 66MHz/66/16 period, stops unsupported external-clock selection,
rejects the extra Exynos register/read-only counter writes and checks reset
while periodic counting is active. These device tests do not count as guest ARM
execution; the independent guest payloads establish that.

The first MMU control produced only translation and permission aborts; the old
QEMU2.10 translator ignored SCTLR.A on ordinary A32 memory operations. The GPL
ARMv5 overlay adds alignment to the 32-bit translator load/store helpers and
includes the control in translated-block identity. The existing QEMU unaligned
access exception path generates the genuine abort. Flagbit22 was unused in the
pinned A32 TB map (handlerbit21, FP exception bits24–25); only the non-AArch64,
pre-ARMv6 system branch sets it. ARMv6+ and user-mode translation are unchanged.
The same-PC clear/set/clear control verifies that cached code cannot retain stale
alignment policy. Multiword LDM/LDRD corner cases and cache timing/coherency are
not exhaustively validated and remain explicitly outside this proof.

An initial priority experiment unmasked the controller before both timers had
expired, so whichever request arrived first was already selected in INTPND. The
control now masks the controller until both sources are pending before selecting
them. This corrects the experiment, not the firmware/model priority algorithm.
Simultaneous FIQ priority remains explicitly unsupported as in M17. Other
mask/preemption/rotation anomalies are research questions until exact semantics
are established; they have not been changed speculatively.

QEMU qtest must run with its virtual clock enabled (`accel=qtest` without `-S`).
Pausing the machine had prevented timer dispatch; removing that harness mistake
made deterministic clock-step checks meaningful. Guest tests continue starting
with `-S` for observed entry and GDB single stepping. Read-only shared build
mounts use `:ro,z`; independent output/socket mounts retain private labels. Two
parallel early runs with private labels on the same build exited before the
debugger opened; serial controlled runs and all final executions pass.

## Coverage / boot readiness

| Component | Status at M18 | Evidence and remaining requirement |
| --- | --- | --- |
| ARM926 | Implemented, selected operations tested | Entry, ARM execution, banked IRQ/FIQ/abort, exception return, MMU sections and ordinary-load alignment; not complete ISA validation. |
| RAM | Implemented and tested | Original four32MiB aliases pass original smoke; SRAM/boot RAM present in board source. Physical alias identity still requires hardware documentation/capture. |
| INTC | Partial, tested subset | Reset/clear, single FIQ and fixed timer IRQ ordering pass. Most declared IDs have no lookup entries; arbitrary priority/rotation/preemption and simultaneous FIQ are not validated. |
| PWM timers | Implemented and tested subset | Five16-bit channels and guest IRQ/FIQ execution. Clock tree, external clock, waveform and DMA unsupported. |
| UART0 | Implemented and tested TX | Actual guest and qtest bytes. RX, framing/baud and physical wiring remain unverified. Additional UARTs are not instantiated. |
| GPIO/keyboard | Partial | PortB register readback verified. Keyboard matrix source exists; pin masks/electrical behavior, interrupts, debounce and reset remain unverified (GPIO has no reset callback). |
| SYSC/clocks/reset | Register storage only/partial | Board source stores PLL/divider/power/reset fields; it does not propagate a real clock graph or prove physical reset/power transitions. |
| RTC | Implemented but unverified | Borrowed Exynos RTC wired at0x57000000; G1/S3C2416 compatibility not established. |
| NAND | Partial, boot-blocking | EC/DA chip model at0x4e000000, incomplete ECC paths, fixed/questioned NFSTAT behavior, ready/busy IRQ TODO. Need actual geometry, OOB/ECC, bad-block layout and matching authorized image. |
| LCD | Partial/unverified | Borrowed Exynos FIMD at0x4c800000; original S3C LCD file is not instantiated. No new display-controller execution/physical fidelity claim. |
| I2C/touch | Implemented but unverified | Exynos I2C plus NT11002 slave ataddress1, wired GPIO interrupt. Protocol/calibration/timing not tested here. |
| DMA | Missing for this board | Declared interrupt IDs do not instantiate a usable DMA controller. |
| MEMC/watchdog/ADC | Not instantiated | Files exist, but board creation is commented out. |
| Boot | Synthetic ELF only | Board raw-copies first8KiB of MTD into RAM0. This is not proof of real NAND/ECC boot. Matching BXCBOOT/BESTAARM and full NAND/OOB remain absent from configured authorized assets. |

The existing PRIME_OS.ROM/PRIME_APP.DAT/PRIME_MASTER.DAT/armfir assets are update/
application components, not a demonstrated complete bootable NAND image. No
image was constructed from them and no proprietary byte was modified. The next
executable hardware tasks are NAND ID/read-page/OOB controls using a self-authored
image, supported clock-tree transitions, and GPIO interrupt/reset controls. A
real boot attempt requires explicitly supplied matching boot artifacts first.

## Reproduction and isolation

Build into a **new** private directory (the pinned reference is never edited):

```sh
python3 scripts/emulator/full_system_build.py --reference /path/to/qemuPrime --dtc-reference /path/to/dtc-reference --output /path/to/new-build --overlay research/full-system/patches/0001-intc-reset-fiq.patch --overlay research/full-system/patches/0002-s3c2416-timer.patch --overlay research/full-system/patches/0003-armv5-alignment.patch
python3 scripts/emulator/full_system_preflight.py --build-root /path/to/new-build
python3 scripts/emulator/full_system_hardware.py --build-root /path/to/new-build --case timer_irq --output /path/to/new-run --timeout 30
python3 scripts/emulator/full_system_timer.py --build-root /path/to/new-build --output /path/to/new-timer-model-run
python3 -m unittest discover -s scripts/emulator -p 'test_full_system*.py' -v
```

Repeat guest runner for timer_fiq, uart_gpio and mmu. Original full_system_run.py
and full_system_intc.py reset/FIQ commands remain regressions. `--overlay` is now
repeatable and ordered; the old single-overlay Python interface still works.
Optional preflight `--build-root` validates exact binary and runtime image hashes
without requiring host Python2/QEMU or an existing reference checkout; compiler,
linker, Podman and GDB must still be available for a guest experiment.

The hardware CLI's timeout bounds guest debugger execution1–120seconds; startup,
image validation and owned cleanup have separate finite bounds. Every runtime
container has no network, a read-only root/build,1GiB RAM,8MiB file limits and
owned-ID cleanup. Logs are bounded translated blocks, not an exact per-executed-
instruction count. This component advertises synthetic diagnostics only: PPL,
CAS, calculator UI, whole-machine restore, physical verification and real kernel
boot are unsupported.


Final reproducible build: `/home/k9-vortex/.cache/prime-m18-hardware-published-build-02`,
SHA-256 `6312c95a5e628798913cbe40cfd8a053801d6637ae44fb76c9e27ed8f0c71c5a`.
It was built from a fresh pinned-source export with all three published overlays,
not by mutating a previously built source tree. Final execution totals:12 fresh
ARM guest runs (four cases × three),3 full timer qtest runs (five channels each),
M17 INTC reset/FIQ regressions (three cycles each), original ARM smoke, and20
ordinary Python tooling tests all pass. Raw directories use the
`/home/k9-vortex/.cache/prime-m18-release-*` prefix. Failed earlier controls are
also retained; no failure was reclassified as a success.
