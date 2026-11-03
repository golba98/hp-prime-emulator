# Full-system G1 research

Status: M19 executes synthetic NAND/OOB/MAIN-512 ECC, clock/reset, LCD/input and a NAND stage handoff, preserving the M18 IRQ/FIQ/MMU controls; no proprietary kernel boot. The production backend executes armfir
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
| ARM926 | Board creates arm926 CPU | Actual kernel execution and complete CPU/cache timing remain unverified |
| RAM | One 32 MiB host buffer at 0x30000000, aliased at 0x32000000/0x34000000/0x36000000 | Verify aliases against hardware documentation |
| Boot | Copies first 8 KiB NAND main data into boot region at zero, including reset; optional ELF loading | Matching authorized boot/NAND image and physical ROM/strap contract missing |
| INTC | S3C2416 model wired to IRQ/FIQ | Priority, pending/ack and nested interrupt tests |
| NAND | Model at 0x4E000000, Samsung EC/DA identification | Synthetic main/OOB and 512-byte 1-bit ECC guest tested; spare/4/8-bit ECC, IRQ/DMA and physical geometry remain unsupported/unverified |
| LCD | Exynos FIMD reused at 0x4C800000 | S3C register subset and RGB565 guest capture tested; physical panel, palette and timing remain unverified |
| GPIO/keys | S3C2416 GPIO at 0x56000000 | Guest matrix scanning tested; physical mapping/debounce/power key remain unverified |
| Touch | NT11002 I2C slave, GPIO interrupt | Packet/timing accuracy and calibration |
| Timers/RTC | Dedicated S3C2416 PWM and inherited RTC | Selectable clock tree tested as research input; physical clock rates and RTC alarms remain unverified |
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

## M16 runnable prerequisite inventory

Run `python3 scripts/emulator/full_system_preflight.py` for a read-only JSON
report (exit 1 when prerequisites are missing). Repeated `--artifact PATH`
options hash explicitly supplied artifacts without interpreting them as bootable.
The script does not install tools, start containers, download images or open USB.

Self-authored `research/full-system/smoke.S` and `smoke.ld` define the first ELF
experiment. Compile with `clang --target=armv5-none-eabi -c`, then link with an ARM
linker and the supplied script. The assembly was compiled locally; no ELF link
or QEMU execution is claimed. Inspect `0x30001000` through GDB: `0x600d0001`
means all three additional RAM aliases matched; `0xbad00001` means a mismatch.
The pinned board defaults UART to a null chardev, so the first experiment uses
a mailbox rather than assuming visible serial output. Timer/IRQ and UART tests
remain subsequent experiments after the reset/RAM smoke actually runs.

M16 follow-up: the preflight now discovers Rust's bundled `rust-lld` as an ARM
linker. `--build-smoke NEW_DIRECTORY` compiles and links the self-authored smoke,
validating ELF32/ARM/entry metadata and refusing an existing output directory.
With TMPDIR on home storage, the local build succeeded with ELF entry
`0x30000000`, SHA-256
`617bcd9865c392f24d145f96d5c0d7cfb97a4654c98adfc9e0a6b05032218d76`.
Execution remains unverified; Python 2 and QEMU are still missing. Three Python
tests pass, including preservation of existing output directories. This
supersedes the earlier missing-linker/assembly-only observation above.


## M17 actual ARM execution

The pinned qemuPrime fork now builds and executes the self-authored smoke on its
**hp-prime S3C2416 machine model**, not a generic ARM fallback. The PPL FullSystem
adapter remains unavailable: no PRIME_OS/kernel/boot-chain execution is claimed.

Rootless Podman uses the pinned official Ubuntu16.04 amd64 image specified in
research/full-system/Containerfile.xenial. Python2.7, GCC5 and build dependencies
exist only in that isolated image. Xenial libfdt1.4.0 proved insufficient: its
header declares fdt_first_subnode but its library lacks it. The build therefore
exports qemuPrime's exact DTC submodule commit558cd81 into the owned source stage.
The canonical scratch reference remains unchanged. The correct Make target is
subdir-arm-softmmu. No compatibility patch was needed for the initial fork build.

Reproduce with:

    python3 scripts/emulator/full_system_build.py --reference /path/to/pinned/qemuPrime --output /home/USER/.cache/prime-research-build
    python3 scripts/emulator/full_system_run.py --build-root /home/USER/.cache/prime-research-build --output /home/USER/.cache/prime-research-run

Output directories must be new. Build logs, exact package inventory, immutable
image/binary/source identities, and runtime GDB/QMP/translations are retained.
Only self-authored code is compiled/executed. Runtime containers have no network,
1GiB memory and8MiB per-file limits; failure cleanup removes the recorded owned
container ID, never another container or the reference checkout.

Actual host GDB evidence across three fresh processes:

- Initial PC0x30000000 and initially zero mailbox.
- Single step advances PC to0x30000004.
- Hardware breakpoint stops at smoke_halt0x3000005c.
- Guest mailbox0x30001000 contains0x600d0001.
- All four RAM alias readbacks contain0x12345678.
- QMP stop/quit and owned-container cleanup complete normally.

The manual and scripted builds produced the same executable SHA256
(dfda20a5b7f308bf42ba66dd21060af898f13fcc3c76a73b9ec039b1ea10e851).
Evidence summary: research/full-system/evidence/m17-smoke.json. Raw logs/builds:
/home/k9-vortex/.cache/prime-m17-fullsystem-build. Eight Python tests pass,
including wrong-source/binary identity, existing-output preservation and generic
ARM observations that cannot satisfy the hp-prime alias test.

The old fork emits a disassembler/translator boundary warning while single
stepping; the retained trace is not an exact instruction-count transcript. GDB
independently verifies execution and mailbox/alias results. No message was sent
to upstream. UART capture is configured, but this first payload does not transmit.
INTC reset/FIQ model regressions were subsequently reproduced and fixed below.


## M17 interrupt-controller increment

Actual baseline qtest runs reproduced two source-demonstrated bugs. A UART0 TX
interrupt raises the CPU IRQ and sets source/pending/subsource/offset. The old
system_reset left those fields and the IRQ line asserted. Selecting UART0 and
timer0 in INTMOD while only UART0 was pending aborted on the invalid-lookup
assertion. Both failed baseline results and raw assertion text are retained.

The GPL research overlay patches/0001-intc-reset-fiq.patch clears pending/mode/
offset/update state and lowers IRQ/FIQ at reset; it binds FIQ lookup to the
actual single pending source and bounds-checks unknown IDs. Simultaneous pending
FIQ priority is explicitly unsupported rather than guessed. No physical G1
interrupt priority or kernel compatibility is claimed.

Both unchanged qtest cases now pass three consecutive cycles each, observing
actual CPU IRQ/FIQ input lines. Pending bits clear after reset/acknowledgement.
UART writes produce ABC in the capture file. A clean full build with the exact
published overlay matched the incremental patched executable, and its ARM/GDB
RAM smoke also passed. This separates device-model qtest evidence (CPU paused)
from genuine ARM instruction execution (the independent smoke).

Reproduce after building a NEW directory with the --overlay option:

    python3 scripts/emulator/full_system_build.py --reference /path/to/pinned/qemuPrime --overlay research/full-system/patches/0001-intc-reset-fiq.patch --output /home/USER/.cache/prime-patched-build
    python3 scripts/emulator/full_system_intc.py --build-root /home/USER/.cache/prime-patched-build --case reset --output /home/USER/.cache/prime-intc-reset
    python3 scripts/emulator/full_system_intc.py --build-root /home/USER/.cache/prime-patched-build --case fiq --output /home/USER/.cache/prime-intc-fiq

Final published-overlay executable SHA256:
011b39176130685aba44be06f96b1e63c6b41d59efba4e48ce82bc26542a7b46.
Exact overlay SHA256:
4221c9856d889941eb89ccbff14474465a6692b9fabdf727e30974ad1a10074c.
Evidence: research/full-system/evidence/m17-intc.json. Raw artifacts and the clean
published-overlay-build are retained under the home-cache build root above.
Ten ordinary Python tests pass without QEMU, proprietary firmware or USB.
No owned containers or background builds remain after validation.

Remaining full-system prerequisites are the matching authorized boot/NAND-OOB
artifacts, kernel/MMU execution proof, and further validated timers/storage/LCD/
input hardware behavior. PPL FullSystem stays unavailable until that independent
software stack runs. Neither a linked ELF nor successful device-model tests are
counted as PRIME_OS boot or physical calculator parity.

## M18 executed hardware foundations

The dedicated five-channel 16-bit S3C2416 timer replaces the borrowed Exynos
device. Published GPL overlays also correct ARMv5 alignment checks in this old
QEMU. Four self-authored guest cases pass three fresh processes each; the
coordinator independently runs all four plus the original smoke through the
new `ppl full-system` interface. IRQ handlers execute9 times and return;
FIQ handlers execute3 times and return. UART output and GPIO register readbacks
are exact. MMU translation, permission, alignment and prefetch aborts enter real
vectors and return to guest instructions. Cache timing, simultaneous FIQ
priority, multiword alignment edge cases and electrical GPIO behavior remain
unverified. The timer blocks migration rather than omitting active device state.

Final published-overlay build SHA-256:
`6312c95a5e628798913cbe40cfd8a053801d6637ae44fb76c9e27ed8f0c71c5a`.
The pinned source/DTC/Xenial toolchain remain unchanged. Build overlays are
ordered and reproducible. `full_system_preflight.py --build-root PATH` validates
the isolated runtime without requiring host Python2/QEMU installation.

See [hardware execution and complete coverage/boot matrices](../../research/full-system/M18_HARDWARE_EXECUTION.md)
and [M18 integration](M18_FULL_SYSTEM_HARDWARE_FOUNDATIONS.md). Local resource
hashes remain inventory evidence: update components and HLE application ELFs do
not establish a bootable NAND/OOB image. No BXCBOOT or matching complete boot
chain was found in the inspected assets. The next executable boot-foundation
experiment is a self-authored NAND page/OOB/ECC control, followed by matching
explicitly supplied boot artifacts when available. Never construct proprietary
boot images from guessed update-component placement.

## M19 boot hardware and storage

M19 adds executed NAND/OOB/MAIN-512 ECC, opt-in clock/reset controls, LCD/input and a self-authored NAND stage handoff. The final combined QEMU build runs all eleven guest cases; kernel boot remains unattempted because matching authorized boot artifacts are missing. See the [M19 report](M19_BOOT_HARDWARE_AND_STORAGE.md) for evidence categories, limits and final verification.
