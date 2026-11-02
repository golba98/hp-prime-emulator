# M16 stateful execution implementation

Implemented on the existing Rust HLE, preserving the IDE's optimized ZZT_IDE path.

Schema 1 fixtures remain supported. Schema 2 adds named source programs and
import, run_program, probe, edit, start and await actions. A program name is a
disposable ZZT_ identifier; reserved helpers and case-ambiguous names are refused.
Native uses its existing SessionContext module identity and persistent state.
External function linkage, physical input and firmware OFF persistence remain
unsupported Native capabilities.

Each initial source and each edited revision has an exact normalized UTF-8
SHA-256. LoadedScenario::source_manifest() exposes the sorted name-to-hash map.
Schema 2's source_sha256 is the SHA-256 of the JSON-serialized BTreeMap manifest,
rather than the hash of one source file. Initial default source uses key
"default"; edited sources use "<program>@<step-id>". Provenance includes the map
and comparison verifies it. Schema 1 keeps its existing single-source hash.

The engine's headless LiveSession supervises a version-6 framed helper process,
drains frames and bounded stderr, enforces deadlines/cancellation and kills/reaps
its own process group. Each scenario copies initialized A/C resources into a
disposable profile. Sequential operations share a genuine firmware process;
OFF/restart creates a fresh process using that profile.

StoreProgram uses a separate exact-source-validated named loader. It assigns
Programs(name) directly from the existing serialized AFiles source attachment.
It does not compile an alias in the IDE scratch module. Replacements require the
previous exact source hash; reserved helpers are protected.

RunStored never reassigns source or invokes Check. At actual function dispatch
the runtime observer checks the requested entry and reads the actual program
context's single source item. Exact source hashing binds that invocation to the
fixture before observing its body. A mismatch stops the disposable process
before body entry. This also rebinds after fresh boot without restoring host
pointers, rerunning initializers or recompiling through host intervention.
Evidence is labelled stored_source_dispatch, independently of an observed
compiler verdict. No compile_verified claim is invented after restart.

Authored zero-argument probes return one typed list element per declared name.
Both engines execute the same probe. Matrix tests currently compare explicit
dimensions/cell projections; direct matrix/complex/integer decoding is not
implemented and unsupported return representations remain unobserved.

Structured Execution events retain actual runtime status, typed values, output,
diagnostics, source binding and program-output pixels. They are emitted for
RunStored, avoiding large JSON payloads on the existing IDE Run fast path.
Framebuffer captures are the program-return display, not a reconstructed Native
image or a claim about physical hardware.

Start waits for genuine body entry, then allows request-bound physical key/touch
actions before Await collects terminal evidence. Input acknowledgments explicitly
say delivery_level=queued; the terminal fixture value proves actual consumption.
Late new inputs are refused when their invocation has ended; cleanup releases
remain allowed. A regression found that ReleaseAll discarded pending key-up
events after they left the desktop queue. It now preserves releases across both
queues. Correlated ReleaseInput waits for queue consumption and a fresh firmware
idle boundary before another invocation. No machine-specific sleep is used.

Session output reserves correlated replies: optional notifications can be
discarded at the ordinary bound; mandatory overflow emits Error/Closed and
terminates instead of silently losing a result.

## Local firmware verification

No physical calculator was accessed. Firmware ELF SHA-256:
ad9779ff72db39fe4e9977ce7c4324f39aad1bb00c849c0aab067dc5bfc2fb39.

- stateful.toml: two named programs alternate; one local counter reaches 2;
  shared A reaches 19; list/matrix projections agree exactly with Native.
- An edited revision initializes that counter to 40 and returns 41 on its next
  execution, while the same calculator session remains active.
- firmware_lifecycle.toml: cross-program call, retained locals, CAS calculation,
  divide-by-zero and subsequent execution, red pixel at (12,34), normal OFF,
  fresh helper boot, persisted scalar/list/matrix/angle and counter reaching 2.
- async_input.toml: Start/ENTER down/up/Await returns 7; a second Start and touch
  down/up at (100,50) returns the typed list {0,100,50}. This failed before the
  pending-release fix and passed after it.

Tests are gated on PRIME_TEST_FIRMWARE, PRIME_TEST_INIT_DATA,
PRIME_TEST_RUNNER, and optional PRIME_TEST_OS_ROM. Normal CI never requires
proprietary files or USB. PRIME_M16_EVIDENCE selects an optional raw JSON report.

## Limits

Check remains available through the existing one-shot adapter. Stateful Check
requires an explicit import verdict; arbitrary in-place checking can reset
module state, so unsupported paths fail explicitly. Source arguments remain
firmware real-number literals. Nondefault initial.seed is explicitly unsupported
because no verified firmware RNG seeding mapping exists. Initial supported
settings run through generated PPL, and initial setup_entry uses authored PPL.

Full live CPU snapshots, hardware comparison capture and full-system boot are
separate workstreams. These tests establish local-firmware behavior and
Native-versus-firmware comparisons, not physical hardware equivalence.

The pre-fix failed asynchronous report and successful reports/screenshots are
retained in the implementation checkout's evidence/m16 directory. Final
integrated workspace/GUI/performance checks belong to the coordinator.

## Verification checkpoint

Focused regression command: cargo test -p prime-emulator -p ppl-compat -p
ppl-engine --lib --test runtime --test differential: 195 passed, 0 failed.
The three firmware-gated tests in that command were additionally executed with
the configured proprietary resources: stateful/edit 18.63 s; lifecycle 20.77 s;
combined keyboard/touch 11.30 s. These are complete fixture durations including
boot/import/shutdown, not launch-latency benchmarks.

Clippy for ppl-engine, ppl-compat and prime-emulator all targets passed with
-D warnings. The coordinator owns integrated GUI and performance verification.
