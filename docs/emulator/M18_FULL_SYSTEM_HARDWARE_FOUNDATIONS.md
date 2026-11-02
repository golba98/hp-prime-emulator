# M18 — Full-system hardware foundations

Implementation in progress from M17 reporting checkpoint `31cfa64` on
`feat/g1-usability-overhaul`. This report records verified work only; final
integration, workspace/GUI regressions and performance comparison are pending.

## Integrated coordinator implementation

The GUI-independent engine now exposes a FullSystem backend descriptor and
bounded regular-file build validation. Discovery checks pinned source revision,
manifest version/status and the actual executable SHA-256. Configured resources
are explicitly distinct from runtime readiness and executed evidence. PPL Run,
Check, calculator UI, CAS, graphing and Native source debugging are unsupported
FullSystem capabilities.

`ppl full-system info --build-root PATH --json` inspects the build.
`ppl full-system run --build-root PATH --case smoke --output NEW_DIR --json`
executes the existing self-authored ARM smoke. Hardware cases use the separate
research runner. This process boundary does not link GPL device code into Rust.
A cancelled/deadline-limited launcher interrupts its owned process group and
attempts bounded cleanup of only its recorded container ID. Results must bind
case, machine, binary, image and revision; synthetic success requires actual
execution observations and cannot claim kernel/physical verification.

The coordinator executed the smoke through the new CLI against the M17 patched
build. GDB verified entry `0x30000000`, single step `0x30000004`, halt
`0x3000005c`, mailbox `0x600d0001` and four alias values `0x12345678`.
Evidence: `evidence/m18/coordinator-smoke.json`. This is actual ARM execution,
not a booted calculator kernel.

## Boot resources

The read-only inventory of existing local research assets found PRIME_OS.ROM
(1 MiB), PRIME_APP.DAT (32 MiB) and PRIME_MASTER.DAT (4 MiB). Their hashes and
paths are recorded in `evidence/m18/authorized-resource-inventory.json`. No
BXCBOOT/BESTAARM name was found in that inspected tree; no complete NAND/OOB
image or matching boot chain is verified. Update components were neither
modified nor combined into an image. Source identity does not establish
bootability or redistribution rights.

## Work being integrated

- Dedicated timer model and executing IRQ/FIQ/UART/GPIO/MMU controls.
- Fresh-process synthetic CPU continuation, excluding calculator resume.
- Source/revision-aware linked Native debugger.

The final report will distinguish implementation handoffs from integrated
verification. Firmware OFF/checkpoint recovery stays authoritative. Arbitrary
calculator live restoration remains unsupported. No USB access, physical writes,
proprietary-image modification or remote publication is authorized or performed.
