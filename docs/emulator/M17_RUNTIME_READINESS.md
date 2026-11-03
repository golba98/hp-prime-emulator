# M17 runtime readiness and blocked-run deadlines

The M16 debug post-CAS failure was reproduced without changing its preserved
calculator profile. That profile stayed in the session's Booting state for
20 seconds while delivering 16 real LCD frames. A fresh initialized profile
using the same debug helper reached Ready in 3.897 seconds.

The preserved helper's SHA-256 was
`d82399bf032b817845d90f465f1fcd7dc1a16070a55e32e24bb6b867d2675cea`.
The failed profile tree fingerprint was
`9e89aa6dd3e064bed0de6019cc4782a9beb217fea443249f04311a333c4d778b`.
All experiments copied disposable data into the owner's home cache; the source
profile and proprietary resources were not modified.

## Finding and implementation

This was not evidence of a host deadlock. A diagnostic run executed 82.8 million
main-thread instructions in 15.015 seconds without faults. Owner-thread traces
showed continued OSWaitForEvent, OSSetEvent, GetEvent and GetSysTime activity.
The frequently sampled PC `0x306A9474` belongs to a glyph alpha-blending loop;
the observed bounds were small and valid. Disabling UI timers as a diagnostic
reduced rendering work, but normal production timer delivery remains unchanged.

Booting had reused the sustained idle-share gate intended for interruption and
persistence. Rendering-heavy but responsive calculator state could therefore
remain Booting despite completing normal event-loop round trips. Exact idle
duration was not inferred from sampled PC activity.

Boot readiness is now a separate proof: a validated LCD capture must exist,
then two genuine normal timer deliveries and pump reentries must occur, followed
by a fresh main wait at an empty input boundary. It injects no desktop keys,
does not manufacture timer bursts and has no machine-specific sleep. A frame,
timer counter or stale wait alone cannot declare Ready. Checkpoint and interrupt
IdleMeter policies, including their 80% sustained-idle threshold, are unchanged.

The runner also enforces its wall-clock deadline from the existing device
producer thread. Previously a main engine blocked inside a native GetEvent or
OSWaitForEvent could prevent the outer run loop from checking its deadline.
Stopping now wakes cancellable host waits; the result remains a time-limit
failure, never successful boot or execution.

## Validation

Three deterministic readiness tests cover busy renderers, incomplete evidence,
invalid frames and restarted counters. A deadline regression blocks in an
infinite host wait without input/timers and verifies bounded cancellation.

The unchanged normal-OFF/CAS/list/matrix/fresh-process recovery test passed in
debug after this change (97.70 seconds on the first comparison), where its M16
comparison had failed at fresh boot. A private copy of the retained failed
profile reached Ready in 5.953 seconds after the fix. Dedicated-cache validation passed all 100 emulator library tests, the unchanged
normal-OFF post-CAS multi-process regression in debug (133.50 s), and emulator
all-target Clippy with warnings denied. Early shared-cache results are retained
for investigation but are not final cross-branch verification.

These changes do not implement live CPU restoration, weaken checkpoint
admission or claim physical hardware verification. Additional safe-point and
component restoration work must continue to distinguish guest state from native
callback continuations and host resources.

## Final integration findings

Later validation classified a separate post-CAS debug checkpoint refusal:
72.73% idle samples, below the unchanged 80% rule, with no OFF input sent.
This is distinct from both the boot-readiness predicate and the reproduced
prelatched-Shift OFF bug. Phase/idle/save diagnostics and opt-in failed-profile
preservation now retain enough evidence to distinguish these conditions.

The native Unicorn dev dependency now uses opt-level 2 / RelWithDebInfo while
project Rust debugging remains intact. Matched-source recovery median improved
from 91.22 to 80.51 seconds; the final coordinator debug regression passed in
71.91 seconds. Guest timers, instruction limits, the idle rule and checkpoint
deadline were not changed. Busy firmware can still refuse strict admission.
See the [final M17 report](M17_RUNTIME_FIDELITY_FULL_SYSTEM.md) and retained
build/refusal evidence for exact commands, flags, trials and limitations.
