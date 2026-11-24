# Live inspection and safe-point prerequisites

Session protocol7 adds request-correlated `inspect` / `inspection` packets.
The engine client exposes `LiveSession::inspect`; using the ordinary Result
waiter for this command is rejected immediately. The reply binds version1,
request identity and the exact composite firmware/resource identity.

Runnable headless example (the initialized input directory is copied):

```sh
TMPDIR="$HOME/.cache/prime-inspection-tmp" python3 scripts/emulator/inspect_session.py \
  --helper /absolute/path/to/prime-emulator \
  --firmware /absolute/path/to/armfir.elf \
  --os-rom /absolute/path/to/PRIME_OS.ROM \
  --initial-data /absolute/path/to/initial-data
```

Create TMPDIR first if selecting a custom location. The tool writes only its
disposable copy, drains frames, issues the query and shuts down its owned helper.
If boot readiness times out, it still attempts inspection and reports
`session_ready:false`; inspection success is not boot success. No USB is opened.
The Rust client also attempts bounded inspection before reporting a failed boot.

The reply contains real owner phases (boundary, executing, SVC enter/blocked/
exit, exited), service identity, caller and blocked handle, entry/completion
times, resource counts, current input/main-wait state and bounded diagnostics.
Readiness/inspection uses nonblocking resource locks. Register copies remain
owner-thread observations; opt-in PRIME_SESSION_TRACE enables their existing
SVC/boundary recording. Missing/stale register observations are not fabricated.

Safe-point requests have an epoch and deadline. Registered owners acknowledge
only outside emu_start/native callbacks. New registrations wait for the pause
lease; dropping, cancellation or expiry releases owners. Registry overflow or
native callback continuations refuse pause. The memory gate independently
reports whether coherent RAM inspection was acquired.

An actual idle calculator inspection observed two blocked native callbacks:
OSWaitForEvent on thread0 and GetEvent on thread1. Its memory gate was acquired,
but owners were not resumably paused. `live_restore_ready` is always false;
complete CPU export and host resource reconstruction remain explicit blockers.
This is not a machine snapshot and no raw host resource is serialized.

## Validation

- Deterministic tests cover genuine owner acknowledgement, deadline refusal,
  lease expiry, explicit cancellation, registration fencing and SVC/handle state.
- Client tests reject wrong request/resource identity, unsupported versions,
  oversized owner metadata and fabricated live-restore capability.
- The real firmware inspection test queries idle and actively waiting firmware,
  then delivers input and verifies the program returns7 (19.89 s).
- The headless Python tool was executed against actual local firmware and
  returned the two real blocked native callbacks and explicit non-resumability.
- Emulator103 and engine28 library tests passed before two further cancellation/
  registration tests; all-target Clippy for engine/emulator/GUI passed.

These are local HLE observations, not newly captured physical-calculator results.
The M16 persistent checkpoint mechanism and its strict admission are preserved.
