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

## Firmware keyboard translation (finding 27: improved, incomplete)

Paste now parses the full expression when it contains powers. AST exponent
boundaries cause actual Right-key transitions to leave each power template
before the next outer operator. This handles nested/right-associative and
parenthesized powers. Incomplete expressions fail atomically before queuing.
Explicit single-character keyboard power entry retains physical-key behavior.

Supported Unicode now includes π, ⅈ, ×, ·, ÷ and −, translated to their actual
calculator key sequences. Unknown characters and pasted newlines are still
rejected; newlines must not silently execute a calculation. Exact Alpha/Shift
frame recognition remains necessary; unknown firmware screens stop automatic
text delivery rather than guessing modifier state.

The repeatable `scripts/emulator/verify_desktop_typing.py` harness compares
desktop paste with equivalent real firmware key input. On local G1 firmware,
all pixels in LCD rows 20–239 match exactly for:

- `2^3+4`
- `2^(3+4)^5+6`
- `π×2−ⅈ÷3`

Evidence is `/tmp/prime-side-firmware-typing-evidence-2/report.json` and its six
PNG captures. An initial attempt identified a test-settling error: inspection
did not include pending raw `Device.keys`. Inspection now reports `queued_keys`,
and snapshot readiness also rejects queued raw input. The final comparisons
use actual empty queues and firmware idle boundaries, with no pixel tolerance
or cursor masks.

## OS services (findings 30–32: improved, incomplete)

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

`WriteComDebugMsg` now marshals the common integer/pointer/string variadic
arguments from R1–R3 followed by stack arguments. Signed/unsigned, hex/octal,
pointer, character, narrow/wide string, flags and bounded width are supported.
Unsupported formats (including float/64-bit/precision formats) produce an
explicit diagnostic rather than a falsely formatted message. Its status
remains Partial.

The registry still has 742 slots: 48 Implemented, 31 Partial and 663 without a
handler. This work does not turn unused slots into verified behavior. Actual
boot still reports `_FindResourceW` and `GetApplicationHeadInfoA` as unsupported;
their resource/metadata ABI has not been implemented here. Thread priority is
stored but does not implement a guest-priority scheduler; wake semantics beyond
resuming a suspended owner require additional ABI evidence.

Virtual battery/USB/indicator device behavior, unknown-device IOCTL fallback,
auto-power-off and the partially observed disk-room structure remain incomplete.
These are unresolved implementation requirements, not newly verified services.

## Firmware text clipboard (finding 28: unresolved)

The firmware transport exposes framebuffer pixels, physical events and managed
program operations; it does not expose selected editor text. `typing.rs`
observes only modifier glyphs. Correct copy-out requires a verified selected-text
accessor/observer for each relevant firmware build and a correlated selection
transaction in the session protocol. OCR or inferring text from pixels would
not meet the requested semantics. No clipboard success is claimed.

## Live calculator snapshots (finding 29: unresolved)

The existing live inspection explicitly returns `live_restore_ready=false`.
`session.rs::inspect` reports incomplete complete-CPU export and host-resource
reconstruction. This patch preserves that admission refusal.

Concrete remaining implementation boundaries:

- `CpuThread` owns a live Unicorn engine. Its observations export general
  registers for diagnostics, not a complete versioned restorable CPU context.
- Blocking SVC handlers retain native Rust call stacks, borrows, mutex/condvar
  state and `SvcCtx` references. `safepoint.rs` explicitly treats parked native
  callback continuations as blockers. They must become resumable service state
  machines before a live snapshot can restore them in another process.
- VFS and heap component images deliberately carry `live_resume=false` and
  refuse whole-calculator resume claims. Writable descriptor identity, sync
  waiters, scheduler/IRQ state, in-flight guest calls, pending input/timers and
  active managed operations need coordinated capture/reconstruction.
- Synthetic ARM/Thumb snapshot/resumable-SVC tests use a separately bounded
  machine contract. They cannot be promoted into live HP calculator evidence.

These are architectural implementation blockers, not evidence that a screenshot
or OFF recovery is a live snapshot. Existing durable OFF recovery remains in
place and passed genuine local-firmware session regression tests.

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
count. Both genuine local firmware lifecycle tests passed. Selected-text copy-out,
live firmware restore and the documented OS/device gaps remain unresolved.
