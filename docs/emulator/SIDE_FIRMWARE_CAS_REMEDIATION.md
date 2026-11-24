# Side emulator: CAS and firmware implementation evidence

This report records actual implemented behavior and the remaining parts of
findings 27–32. It does not claim complete firmware emulation or physical
calculator verification. Local firmware tests use copied disposable profiles.

## Native algebra service

`ppl-cas` supervises an independently compiled Giac executable. The Rust
application does not link Giac. Each request carries an explicit environment,
precision, angle mode and exact/approximate mode. Successful replies export
typed exact integers/rationals, decimal values, complex values, strings,
symbolic expressions, lists or matrices and the updated environment. The
environment is committed only after validating protocol version, request ID,
engine identity, result structure and successful evaluation.

Operations use separate process groups. Cancellation and timeout kill and reap
the helper and descendants, including helpers that do not read stdin. Output
is drained concurrently with enforced bounds. Syntax/evaluation errors preserve
the preceding environment. Session bindings serialize independently of helper
processes. Giac secure mode prevents filesystem/process operations from CAS
expressions. Setup, protocol and GPL helper licensing are documented in
`tools/giac-helper/README.md`; the helper carries its license separately.

Actual Giac 2.0.0 tests cover polynomial factorization, differentiation,
integration, solving, exact fractions, arbitrary-precision integers, complex
values, matrices, strings, assignments, purge, fresh-process context restoration,
independent sessions, angle modes, approximate mode and syntax errors. Six
integration tests and eight process-supervision tests pass, with no skips.

## Firmware desktop text (finding27: implemented for installed G1)

Desktop Text/Paste now executes the original firmware Clipboard/FindView/text
methods through a digest-admitted, serializable guest-call plan. Literal editors
retain Unicode, newlines and unmatched delimiters; text never becomes an Enter
key. Actual firmware math source/caret queries preserve quoted scalars while
established Unicode math aliases retain operator behavior. Other firmware builds
keep the explicit restricted legacy translator until their ABI is verified.

Genuine controls replace/save/execute a Program Editor source (return17 and exact
Unicode source), create/read back a multiline Unicode note, insert30 into a SIN
template, preserveⅈ×😀 at a physically moved quoted caret, and enter12.5 into a
running PPL INPUT (return25). An actual desktop GUI note Paste exceeds the old2048
limit and reads back exactly through PPL. GUI/backend share a UTF16 budget and
refuse NUL or oversized packets atomically.

Complete caret phases of three expressions match actual firmware Copy/Paste with
zero differing pixels. The physical-key fraction caret differs by51pixels and
is retained rather than masked. A saved producer is required to contain queued
text; after its helper is killed, fresh-process resume and genuine Copy return
314159265 exactly once. Owned source/event/argument buffers and borrowed guest
clipboard/editor buffers have separate restoration validation.

## OS services (30–32 implemented for their recorded scoped criteria)

Deferred guest-thread creation now preserves launch arguments and allocates an
identity. Resume starts that exact invocation only once. Suspend/resume are
observed at CPU slice boundaries; termination stops further execution and wakes
blocked event/semaphore/critical-section waits. Shutdown can end a suspended
owner without requiring a resume. Six tests execute actual synthetic ARM SVC
calls/CPU code for deferred launch, suspension, resumption, termination and ISR
behavior. These tests do not constitute firmware ABI parity for every caller.

The original audit's statement that registered ISRs were never dispatched was
incorrect: `runner::run_with_session` already called `CpuThread::run_isr` between
slices. The implementation now masks an IRQ without deleting its registration,
allows Enable to restore delivery, validates Done against registered IRQs and
sorts IRQ delivery order. ISR faults now propagate failure and are not counted
as successful deliveries. ARM tests prove callback memory writes and CPU
register/PC preservation, mask/reenable behavior and failure on an invalid ISR.

`WriteComDebugMsg` now decodes the ARM variadic ABI, including aligned64-bit and
floating values, integer lengths, width/precision, dynamic fields, narrow/wide
text and explicit guest count writes. Formats/output are bounded and unknown
conversions report failure. Host libc receives only generated single conversions
and host-owned typed values.

Finding31 is now implemented for its functional HLE criteria. Executable controls
from the installed OS ROM establish idempotent Suspend, flag-clearing Resume,
timed-sleep Wake, unique priority slots and auto-power mode/return semantics.
The scheduler applies those priorities to actual queued CPU owners and serializes
execution slices. Auto-power pauses those owners and ON resumes their current
instructions. A common guest clock freezes service/UI/IRQ timing during power and
bounded inspection pauses; clock/timed-wait components restore after host downtime.
Thread/ISR behavior remains cooperative at CPU slice boundaries, without a claim
of cycle-accurate hardware preemption. The existing registry's conservative
per-service granularity labels are distinct from this functional acceptance.

Application metadata now implements original-ROM ANSI/wide132-byte records for
PE, ELF and FM/FI modules, missing/malformed files, actual named BIN/ROMSPC
resources, checksums and raw section sizes. Executing original ROM compares all
record bytes and canaries. Its five legacy resource exports really return
not-found; those exports are distinct from the implemented internal parser.

Live sessions stop on unhandled OS calls. Unresolved PE imports fault with their
DLL/symbol instead of silently returning0; late dependencies bind their actual
ARM/Thumb export while preserving arguments and return state. The read-only
`prime-emulator service-inventory` reports742slots:55 implemented,31 partial and
656 unhandled. The [current inventory](side-remediation/current-service-inventory.json)
retains each slot; unused slots remain coverage gaps and fault if called. This
closes30's used-ABI/inventory criteria, not every possible OS API.

Finding32 now implements the originally reported virtual battery, USB/USBHOST,
indicator, unknown-device and disk-room gaps. Typed configuration changes real
firmware-visible battery records/events. USB uses bounded data queues and a real
guest callback worker; `Prime::open_virtual` and `ppl device virtual` exchange the
original firmware protocol without opening physical USB. Genuine information,
PNG, guarded upload/readback and fresh-boot execution passed. BACK PWM ramps,
RTC-driven indicator patterns and ON cancellation have executable original-ROM
controls. Disk reporting fills the complete60-byte structure and enforces its
logical C-volume budget. Fourteen ROM controls and genuine firmware/GUI tests,
including a separate GUI-process reopen, passed. Electrical hardware and a
physical calculator link are not claimed.

Unknown devices no longer return FF-filled success. The inherited fabricated
FACTORY blob/success handler is removed; that device explicitly reports absence.
Executing the initialized original-ROM registry and OPEN_EXISTING confirms that
FACTORY/FTL are absent and return failure. No invented partition contents are
supplied. Broader stream dispositions and optional drivers remain explicitly
outside established OS coverage; no hardware-only blocker is asserted. Complete
live persistence of these working virtual models is verified under29.

## Firmware text clipboard (finding 28: implemented for the verified G1 profile)

The normal firmware Copy/Cut path calls an internal setter rather than the
named clipboard OS stubs. `prime-emulator/src/clipboard.rs` observes its committed
size/type/UTF-16 record, gated by the setter code digest. The observer is read-only;
it does not infer text from pixels or from previously supplied desktop input.
The session's Clipboard event reaches egui's real system-clipboard integration.
Unknown setter code reports this capability unavailable. Binary calculator
objects are not exported as text; private HP glyph code points remain unchanged.

Genuine local firmware controls cover full and partial Home selections, repeated
Copy, and multiline Unicode program Copy/Cut. An isolated GUI scenario uses real
LCD mouse clicks and retrieves the result through OS Ctrl+V. Its seven assertions
passed. The normal GUI pointer path now streams stationary MOVE samples, required
by the firmware's touch debounce. Sanitized controls and rerunnable tests are in
`tests/fixtures/firmware-clipboard-controls.json`,
`scripts/emulator/verify_firmware_clipboard.py` and
`tests/ide_xvfb_smoke/scripts/script_firmware_clipboard.json`.

## Live calculator snapshots (finding29: implemented)

The firmware session controls now provide explicit **Save live state** and
**Resume live state**. Saving captures execution without OFF; resuming replaces
the owned helper on its exact resolved profile. Missing or incompatible images
are refused. Ordinary close retains the separate existing OFF recovery workflow.
There is no automatic live save on close or automatic resume.

The session producer coordinates an expiring CPU pause and one memory exclusion
interval. The complete image includes banked ARM926/FPU/CP15 state, every mapped
RAM block and allocator, scheduler/clocks/RTC, threads/stacks/queued launches,
virtual peripherals and input queues, complete drive trees and open VFS/sync
resources, active jobs/calls and compiler/runtime observations, typing and frames.
Immutable RAM compression and atomic current/previous publication happen after
CPUs resume. Every part is bounded, checksummed and resource-bound.

Restoration validates staged data and constructs engines on their owning threads
behind a startup barrier. VFS handles retain their inodes while staged drives are
promoted. A journal preserves old trees and rolls back interrupted installation.
Managed result channels bind the saved source/request, and transport attachment
restores pending operations/counters/environment before Ready initialization.
Guest and producer deadlines exclude archive loading and host downtime.

`firmware_live_resume` executes genuine PPL across SIGKILL and fresh helper
reconstruction, returning{113,37,150} after WAIT(0). Local37 and the single entry
side effect are preserved. A second timed WAIT(5) survives ordinary Shutdown and
returns{1114,73} after its remaining guest delay. Nondefault virtual environment
state survives; corrupt current data selects an intact previous active image;
a different ROM is refused without changing A/C files. The real desktop pointer
Save/Restart/Resume/Enter scenario additionally verifies the result and identical
program graphics across three helper sessions. These are whole-calculator
results, beyond the retained independent ARM component controls.

`session/archive.rs`, `live_image.rs`, `promotion.rs` and `runner/restore.rs`
implement publication/admission/orchestration. Existing CPU, RAM, storage, VFS,
sync, call/job/observer and kernel image tests remain. A pre-ISR queue regression
verifies an accepted interrupt runs exactly once after reconstruction; worker
registration/stack exclusion prevents an allocator race and distinguishes
completed owners from queued launches. Inspection reports a portable boundary
only when owner/resource/producer admission succeeds; SaveLive performs final
RAM/storage admission and returns any failure explicitly.

Current and previous archives, older generations and pre-restore drive trees
remain local rather than being automatically pruned. Images require matching
resources and supported backend schemas; physical USB-peer/hardware state is not
part of the virtual machine. Broader OS coverage remains finding30.

## Executed validation

With the worktree's source and configured proprietary assets:

```sh
cargo test -p ppl-cas --features giac-integration
cargo test -p prime-emulator --test thread_control --test svc --test sync_lifecycle --lib
cargo test -p prime-emulator --test firmware_session -- --test-threads=1 --nocapture
cargo clippy -p ppl-cas --all-targets --all-features -- -D warnings
cargo clippy -p prime-emulator --all-targets -- -D warnings
```

Counts at handoff: CAS 14 pass; emulator library 157 pass (including two new
debug-format tests), SVC 16 pass, synchronization lifecycle 6 pass, ARM thread/IRQ
6 pass; genuine local-firmware session 2 pass. Final coordinator integration
must rerun changed-source checks and relevant desktop scenarios. Firmware
profiles and the Giac dependency prefix stay outside Git. Nothing was committed,
pushed or sent to physical hardware.
## Home measurement boundary

`ppl_cas::units::translate_home` detects attached measurements and the four HP
unit tools, without routing ordinary Home input, comments, strings or identifiers
such as `speed_m` into CAS. It translates the documented HP compound suffix
`5_kg*m^2/s^2` to Giac's `5_(kg*m^2/s^2)` and routes `CONVERT`, `MKSA`, `UFACTOR`
and `USIMPLIFY` to their genuine Giac implementations. No Rust numerical unit
conversion table or successful-looking placeholder is used.

The Home/CAS variable bridge preserves symbols inside grouped unit suffixes, so
Home variables named `m`, `s`, `A` and `K` cannot change measurement dimensions.
Actual Giac tests cover the guide's 20 cm + 5 inches calculation, division by
4 seconds, conversion to km/h, all four unit tools, dimension-preserving
multiplication and powers, and actual incompatible-dimension errors. Inch and
minute input use Giac's equivalent `inch` and `mn` spellings because this Giac
version's parser treats `in`/`min` as operators/functions.

Supported units comprise SI bases m/g/s/A/K/mol, the explicitly listed common SI
derived units in `units.rs`, their documented prefixes, and the tested common
length, mass, time and pressure units listed there. Unknown units, affine
Celsius/Fahrenheit temperatures, candela (`cd` collides with Giac's secure-mode
filesystem command), gallon/torr (unresolved unit names in this backend) and
`fm` (Giac means fathom, conflicting with HP's femto prefix) fail explicitly.
A numeric expression may be suffixed as `(2+3)_m`; arbitrary names such as
`speed_m` retain their identity. Giac unit conversion coefficients are floating
values; this implementation does not claim exact rational SI conversion factors.

Reference: HP Prime User Guide, chapter 24, pages 490–496,
https://h10032.www1.hp.com/ctg/Manual/c05332710.pdf. Integration tests use real
Giac 2.0.0 and compare dimensional differences within 1e-12 in SI base units for
the documented decimal examples. The exact 2 m × 3 m result is also checked.


## Final canonical verification

The final integrated command counts and all 33 outcomes are maintained in
[the remediation report](side-remediation/REMEDIATION.md). The final workspace
passed 2,219 tests (6 existing opt-in ignores), real-Giac GUI integration passed
552 tests (2 existing opt-in ignores), and the separate CAS/VM feature run passed
256 tests without ignores. These overlapping commands are not a combined unique
count. Both genuine local firmware lifecycle tests passed. Selected-text copy-out is now verified as described above. Live firmware restore
and the documented broader OS gaps remain open. The original
virtual environment finding32 has since passed its scoped acceptance above.
