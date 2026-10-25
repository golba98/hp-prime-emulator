# M8 — Unified native and original-firmware CLI

Date: 2026-10-24. Status: **M8 COMPLETE — UNIFIED NATIVE/FIRMWARE CLI VERIFIED**.
Branch `rust-rewrite`. No commits, staging, physical-device commands, firmware
archive changes, or M9 GUI integration were performed.

## 1. Baseline and preserved behavior

The previous `ppl check` used the Rust compiler; `ppl run` used the Rust VM,
headless host, native graphics and PPL expression arguments. Commands accepted
files, inline code and stdin, plus the `ppl FILE` shorthand. Explicit entry
selection already existed. The current working tree also already preferred an
export matching the filename before falling back to the first native export.
This existing fix was preserved rather than restarted.

Before M8 changes, native HELLO/SQUARE and three Baseconv CLI tests passed.
M6 HELLO/BROKEN passed (13.71 s test time), including the authentic BROKEN line
3/column 30 assertions; evidence remained under `/tmp/.tmps5E6v3/` at the time.
M7 HELLO/SQUARE/DEMO/FACT passed (115.46 s), with artifacts under
`/tmp/.tmpeu9hNJ/`. Persistent M8 regression records below replace reliance on
these temporary baseline paths.

Legacy native invocations retain their human output, streaming PRINT, quiet
behavior, warnings, debugger/GUI paths, native limits and exit policy. Native
and explicit-native now share strict file/container decoding. Malformed binary
containers and invalid encodings are rejected instead of heuristic recovery.
UTF-16BE with BOM is additionally covered. Valid UTF-8, BOM-marked UTF-16,
source `.hpprgm` files, and genuine containers are tested. Non-finite native
execution limits now produce usage errors rather than a duration-conversion
panic. Output paths aliasing source files are refused.

## 2. Architecture and changed files

New `crates/ppl-engine` is a GUI-independent library with owned source snapshots,
typed engine/status/diagnostic/result structures, cancellation, native adaptation,
lossless argument codecs and managed firmware supervision. It depends on the
existing native compiler/VM/graphics, not GUI, USB or Unicorn.

The native adapter calls the existing compiler and VM. A forwarding host records
PRINT/clear events while retaining existing scripted inputs, keys, dialogs and
clock behavior. It does not implement another interpreter. Legacy native human
commands retain their established execution path; shared loading and legacy
argument parsing use the same reusable utilities.

Firmware uses the existing `prime-emulator check-program` and `run-program`
executables. This boundary avoids adding Unicorn/CMake/libatomic requirements
to native-only `ppl` installations and permits hard process termination without
changing the emulator's execution model. There is no shell invocation, hidden
Cargo command, persistent emulator service or native fallback.

Changed CLI integration: `main.rs`, command registration, `cmd/unified.rs`,
shared loading/arguments, help, CLI manifest, lockfile, README and new tests.
New library modules: `source`, `arguments`, `native`, `firmware`, and result types.
The helper gained optional request correlation, optional automatic entry
selection, version output, and a passive body-entry checkpoint. Existing
explicit-entry M7 APIs and standalone commands remain supported.

## 3. Commands and installation

```bash
scripts/emulator/run.sh build --release --bin ppl --bin prime-emulator
```

Install both executables together, or specify the helper explicitly. The native
CLI remains usable without the helper installed.

```bash
ppl check HELLO.ppl
ppl run HELLO.ppl
ppl debug HELLO.ppl
ppl check HELLO.ppl --engine native --json
ppl run SQUARE.ppl --engine native --entry SQUARE --args '[7]' --json

ppl check BROKEN.ppl --engine firmware --json
ppl run SQUARE.ppl --engine firmware --entry SQUARE --args '[7]' --json
ppl run DRAWTEST.ppl --engine firmware --png draw.png --report draw.json
```

Exactly reproduced firmware command shape, using the local supplied assets:

```bash
FW="$PWD/scratch/research/firmware/extracted"
export PPL_FIRMWARE="$FW/fat/armfir.elf"
export PPL_OS_ROM="$FW/PRIME_OS.ROM"
export PPL_FACTORY_A="$PWD/scratch/research/m7-private/factory/A"
export PPL_FIRMWARE_INIT_DATA="$PWD/scratch/research/m7-private/initial-ready/data"

scripts/emulator/run.sh run --release --bin ppl -- run \
  crates/prime-emulator/tests/fixtures/m7/SQUARE.ppl \
  --engine firmware --entry SQUARE --args '[7]' --json \
  --report /tmp/m8-square.json --png /tmp/m8-square.png
```

Omit `PPL_FIRMWARE_INIT_DATA` for fresh factory initialization. It is an optional
program-free template, always copied. The pinned OS ROM is mapped inert and
remains optional under the existing backend contract; a configured missing ROM
is an explicit resource error.

## 4. Resource discovery

Explicit flags override environment configuration:

| Flag | Environment |
|---|---|
| `--firmware` | `PPL_FIRMWARE` |
| `--os-rom` | `PPL_OS_ROM` |
| `--factory-a` | `PPL_FACTORY_A` |
| `--init-data` | `PPL_FIRMWARE_INIT_DATA` |
| `--firmware-runner` | `PPL_FIRMWARE_RUNNER` |

Helper discovery: explicit path/environment, sibling of the current executable,
then PATH. Explicit invalid paths never fall through to another executable.
The helper must be executable and answer `--version`; reports must support the
request-ID and version-1 contracts. Old incompatible helpers fail explicitly.
No developer-specific home directory is embedded in production code.

Missing ELF, resource trees, invalid initialized storage, missing runner, wrong
firmware hash and protocol failures have distinct outcomes. The helper retains
M6's exact firmware-SHA gate. No proprietary asset is fetched or bundled.
Native mode never resolves these resources or starts the helper.

## 5. Source and container identity

The existing verified strict `.hpprgm` parser is reused. Container magic followed
by invalid structure fails without scanning for a recoverable EXPORT substring.
Plain text with a `.hpprgm` extension remains text; it is explicitly staged as
source for M5 import and is never called a genuine container merely by extension.
Genuine container bytes are snapshotted unchanged. Text is decoded strictly and
normalized to LF before staging. UTF-8 BOM and UTF-16 LE/BE BOM are supported.

The result records original input SHA-256, LF-normalized source SHA-256, actual
engine-source SHA-256, original path/display name, encoding, representation and
transformations. For a container containing CRLF, firmware preserves its source
while the canonical identity can still describe LF text. Diagnostics refer to
the engine source; raw firmware UTF-16 positions are retained. Numeric original
input byte offsets are not invented from normalized positions.

Valid ASCII program names are preserved. `--program-name` overrides storage
naming. An otherwise invalid host stem (including the tested Unicode/spaced
basename) maps to `PPL_<source-hash-prefix>`; original path/name remain recorded.
That storage mapping does not select the entry function.

## 6. Entry selection

Explicit entry names always select that function or fail. Native selection keeps
its current filename-match/first-export/sole-function conventions, including
private functions when explicitly requested. Legacy multiple-export notes remain.

Firmware automatic selection runs after the original compiler succeeds, using
its freshly written export cache: a unique case-insensitive match to the original
filename stem, else a sole export, else an entry error listing candidates.
Source snapshots use generated filenames, so the original stem is transmitted
separately. No native AST is consulted and no second firmware boot is required.
Firmware entry invocation remains limited to exported functions.

The BST regression fixture exports BST_Init, BST_KeyProbe, then BSTVisualizer,
returning 111, 222 and 333. Both native paths select the third export by filename;
firmware does likewise. Explicit BST_KeyProbe returns 222. ARGTEST has no matching
filename and two exports: firmware requires `--entry` rather than running the
first helper. Explicit MiX_sum(-4,7) returns 3.

## 7. Arguments and precision

Confirmed compatibility contract:

| Invocation | Default argument syntax |
|---|---|
| No `--engine` | Existing PPL expressions |
| Explicit native or firmware | JSON array |
| `--args-format json` | Explicit JSON override |
| `--args-format ppl` | Native-only legacy expressions |

Existing `--args '50, {1,2}, "s"'` remains available. JSON `[7]` denotes one numeric
argument. Native JSON strings are strings, nested arrays are PPL lists. Tagged
`real`, `integer`, `complex`, `matrix`, `list`, `string`, `grob` and native-only
`ppl` expressions cover existing VM values. Matrix tags specify rows, columns,
vector flag and flattened values. Integer tags retain literal and bit width;
reports additionally expose signed/unsigned string views. Native PPL expressions
remain available for compatibility rather than being restricted to JSON literals.

Numeric JSON spelling is retained with RawValue, never routed through f64.
Exact-decimal tags use `{"type":"real","value":"1e400"}`. Native conversion uses
its existing 12-digit Real behavior; firmware receives the original decimal
spelling and applies its own parser/precision. Requested raw arguments and
resolved values are retained. Tagged native round trips include 1e400 complex
components, lists and an eight-bit integer.

Firmware accepts real numbers only. Unsupported strings/lists/matrices/booleans
and PPL-expression evaluation are rejected explicitly. Invalid JSON/decimal
requests differ from unsupported capabilities. Wrong firmware arity preserves
the actual pre-body guest diagnostic, `Error: Syntax Error`, and returns an
invocation error. No argument is converted into a placeholder zero.

## 8. Results and JSON schema

`--json` emits exactly one document on stdout; helper stdout/stderr are captured.
`--report` writes the same versioned result. Native legacy human commands keep
their prior output; explicit engine commands identify engine, status and entry.

Schema version 1 contains:

```text
schema_version, request_id, engine, requested_engine, operation, program
source {path, display_name, input_sha256, source_sha256,
        engine_source_sha256, encoding, representation, transformations}
requested_entry, entry, arguments {format, raw, resolved}
compilation_status, execution_status, status, exit_code
return_value, output[], diagnostics[], runtime_errors[], error
    diagnostic {source, severity, code, message, line, column, raw_position}
    error {kind, phase, message}
timings, artifacts[], work_retained, backend
```

Return values are tagged data with decimal strings, not forced into JSON doubles.
Firmware exact rationals/strings/errors retain their original supported forms.
Unavailable values/locations are null. Invalid engine names have no selected
engine (`engine:null`) and preserve the requested name. Native in-process requests
currently have no helper request ID. Helper reports remain nested under backend,
with original guest observation structures and backend exit code intact.

Compiler and runtime phases are separate. A completed compiler call cannot mark
execution completed. M7 still requires actual callback/body entry, evaluator
teardown, source context and decoded guest result. Firmware PRINT strings are
actual guest UTF-16, not OCR. Object-mode output remains value evidence rather
than fabricated text. Clear operations are events. Runtime positions unavailable
in M7 remain null; native spans never substitute for them.

## 9. Process supervision and protocol validation

Commands use explicit argument arrays with absolute resource/snapshot paths.
Each helper owns a new process group. Captured stdout is capped at 16 MiB and
stderr at 1 MiB while readers continue draining; reports exceeding 64 MiB fail.
A noisy synthetic helper confirms there is no pipe deadlock.

A new owned run root and request nonce prevent old report reuse. The frontend
validates schema, engine, nonce, program, source hash, exit/status agreement,
source preservation and compiler observations. Successful runs additionally
require genuine completion evidence and the selected entry. Non-executing
outcomes containing runtime data are rejected. A requested screenshot is only
listed when its bytes match this invocation's guest-capture hash; an old image
is not attributed to a rejected program.

Malformed, missing, mismatched or incomplete reports are infrastructure/protocol
failures, not source rejection. Unsupported schema/profile is explicit. A child
terminated without a report is not assumed to be an ARM guest fault.

## 10. Timeouts and cancellation

Firmware defaults: 60 s boot settlement, 30 s Check/key settlement budget,
30 s actual PPL execution (`--max-time`), 360 s outer request (`--timeout-s`),
and the existing 8B guest instruction budget. The compile budget reuses M6's
bounded Check action; it is not an invented direct compiler-call timeout.
Startup is not charged against the PPL execution budget. Native limits retain
their VM meaning; native steps are not reinterpreted as ARM instructions.
Native-only settings/inputs/keys are rejected in firmware mode; runtime key/touch
replay uses the existing `--runtime-keys` language. Firmware rejects unbounded
`--no-time-limit` requests.

Ctrl-C sets a cancellation token. The supervisor sends SIGINT, waits one second,
then SIGTERM, and after another 250 ms SIGKILL if needed. It kills remaining group
members, waits for the helper, and joins capture readers. Synthetic tests include
a helper and descendant ignoring signals. Native execution uses existing VM
cooperative interruption and retains native exit policy.

A small M7 `runtime-entry-NN.json` checkpoint is published only after passive
observation of body entry. It records source hash, function, thread and invocation;
it does not write guest memory. The genuine cancellation test waits for this
checkpoint before signalling the parent CLI, then verifies that the helper PID
is absent. Measured Ctrl-C-to-exit latency was **90.5 ms**.

Source/stdin loading and native compilation are synchronous. Native cancellation
is observed at compiler/VM checkpoints, not by forcibly terminating Rust threads.
The process supervisor and verified platform are Fedora/Linux; other platforms
have not been validated by M8.

## 11. Exit mapping

Native established exits remain: 0 success; 1 compile/policy rejection; 2 usage,
entry or argument parsing; 3 runtime failure (including pre-body arity);
4 VM limits/interruption/input exhaustion; 5 I/O. Legacy OS signal behavior is
unchanged. JSON status can be more specific than this compatibility mapping.

Firmware frontend mapping:

| Exit | Meaning |
|---:|---|
| 0 | Successful check or completed execution |
| 1 | Genuine source rejection |
| 2 | CLI usage |
| 3 | Genuine runtime error |
| 4 | Timeout |
| 5 | Source/container/artifact I/O |
| 6 | Missing or ambiguous entry |
| 7 | Invalid arguments |
| 8 | Unsupported capability/profile |
| 9 | Actual guest execution fault |
| 10 | Emulator/protocol/infrastructure failure |
| 11 | Missing firmware |
| 12 | Missing resource/template |
| 13 | Missing runner |
| 130 | Cancellation |

The original helper exit code remains available separately. This mapping does
not reinterpret M7 exit 3 (timeout) as frontend exit 3 (runtime error).

## 12. Real results and differential comparison

| Case | Native | Original firmware |
|---|---|---|
| HELLO check | Accepted | Accepted |
| HELLO PRINT | Hello from HP Prime! | Same exact recovered text |
| HELLO implicit return | real 0 | returned PRINT string |
| SQUARE 7/3/0/-4 | 49/9/0/16 | 49/9/0/16 with guest argument evidence |
| BROKEN | Native syntax diagnostics | Error: Syntax Error; line 3, column 30 |
| BROKEN run | Compile rejection | Compile rejection, no runtime invocation |
| DRAWTEST | Native graphics; implicit 0 | Original blue LCD/near-white text; implicit 104 |
| ERRLIST | Invalid dimension, index detail | Error: Invalid input |
| Other runtime errors | Native diagnostics/spans | Genuine Bad argument type / Invalid dimension / X/0 |
| Ambiguous default entry | Existing first-export convention | Requires explicit entry |
| Numeric runtime location | Native span | Unavailable; guest context preserved |

No native compiler/VM changes were made to force agreement. Final comparisons
ran both backends through `ppl`, outside the repository root, with resource
configuration from environment and explicit-flag override checks.

## 13. Regression and reliability evidence

Evidence root: `scratch/research/m8-evidence/`. Exact commands, environment,
exit codes and timings are in `validation/`, `final/commands.json` and each
integration directory's command/result files. All firmware tests supplied the
local assets explicitly. Ordinary workspace firmware skips are not genuine
verification.

M4–M6: all 13 distinct tests passed across the recorded runs. M7: all 8 distinct
tests passed. M8: all 5 integration groups passed across the initial run and
corrected cancellation rerun. The reliability group completed **12/12 independent
boots**, three each of HELLO, SQUARE, DRAWTEST and ERRTYPE, with unique request IDs.
The final separate matrix completed ten expected firmware outcomes and six native
comparisons, including actual source rejection, arity rejection and ambiguous
entry outcomes—not ten successful PPL executions.

Retained failures are explicit:

* During three simultaneous guest suites, two M4 boot-settlement timeouts and
  one M7 editor-settlement timeout occurred before checking DEMO. Failure reports were
  classified as timeouts, not syntax errors. The unchanged M4/M5 tests then
  passed 7/7 with one guest at a time; the unchanged M7 HELLO/SQUARE/DEMO/FACT
  group also passed. This correlation does not prove a complete root cause;
  heavily concurrent UI/tracing workloads remain a reliability limitation.
* The original cancellation test waited for an input screenshot directory that
  is never created while LOOP keeps the main thread busy. The command correctly
  reached its execution timeout; the test checkpoint assumption was wrong.
  The corrected test waits on the guest-observed body-entry checkpoint, then
  confirms exit 130 and no helper orphan. Timeout limits were not increased.
* An early synthetic native-error fixture used reserved builtin name F; it was
  replaced by FUNCASE. No compiler rule was weakened.

All initial logs remain in validation. The isolated reruns are recorded in
`isolated.json`, with M4/M5 exit 0, M7 failed-group rerun exit 0 and M8 cancellation
rerun exit 0. These are aggregate unique-test results, not a claim that the first
parallel suite was green.

## 14. Measurements

Final comparison data is retained in `performance.json` and `final/commands.json`.
Representative isolated measurements:

| Operation | Native compiler / execution | Firmware compiler / execution | Firmware complete request |
|---|---|---|---:|
| HELLO | 0.041 ms / 0.122 ms | 2.105 ms / 24.015 ms | 15.779 s |
| SQUARE(7) | 0.038 ms / 0.120 ms | 2.471 ms / 0.809 ms | 17.923 s |
| DRAWTEST | 0.047 ms / 18.819 ms | 4.251 ms / 22.192 ms | 18.770 s |
| ERRLIST | 0.046 ms / 0.081 ms | 2.245 ms / 5.595 ms | 17.680 s |

Firmware totals include boot, verified key settlement, compilation, invocation,
collection and teardown. A fresh Unicode-named genuine-container HELLO run took
21.998 s, including 6.818 s factory initialization, 0.698 s checking-boot settlement
and 4.610 ms import. Program-free template copies are much cheaper; backend
initialize/import timings remain available. No native runtime figure is compared
as though it included a firmware boot.

Parent helper-wall minus backend total measures process/report collection costs;
per-run values are retained rather than claiming a universal startup constant.
Serialization probes measured roughly 10–15 microseconds for native reports and
25–34 microseconds for the sampled firmware reports. The field measures a first
serialization pass; actual final output is serialized again with that measurement.
Input reading, UI key delivery and source-name length can materially affect total
time (BSTVisualizer's mixed-case input took 26.340 s end to end).

## 15. Reproduction and required checks

```bash
cargo fmt --all -- --check
scripts/emulator/run.sh check --workspace --all-targets
scripts/emulator/run.sh clippy --workspace --all-targets -- -D warnings
scripts/emulator/run.sh test --workspace --all-targets
```

All four required commands returned exit 0. The final workspace test run reported
**1,131 passed, 0 failed, 3 ignored**. Exact command wall times and exit codes are
in `validation/workspace.json`; firmware asset-unset skips in that run are not
counted as genuine guest tests.

```bash
FW="$PWD/scratch/research/firmware/extracted"
export PRIME_TEST_FIRMWARE="$FW/fat/armfir.elf"
export PRIME_TEST_OS_ROM="$FW/PRIME_OS.ROM"
export PRIME_TEST_FACTORY_A="$PWD/scratch/research/m7-private/factory/A"
export PRIME_TEST_INIT_DATA="$PWD/scratch/research/m7-private/initial-ready/data"
export PRIME_TEST_RUNNER="$PWD/../.cache/prime-emulator-target/release/prime-emulator"
export PRIME_TEST_UNIFIED_OUT="$PWD/scratch/research/m8-private/reproduction"

scripts/emulator/run.sh build --release --bin ppl --bin prime-emulator
scripts/emulator/run.sh test --release -p ppl-cli --test firmware_engines \
  -- --test-threads=1 --nocapture
scripts/emulator/run.sh test --release -p prime-emulator \
  --test firmware_boot --test firmware_display --test firmware_programs \
  --test firmware_compiler --test firmware_runtime \
  -- --test-threads=1 --nocapture
```

Run the guest suites sequentially for reference acceptance; do not silently count
an asset-unset test return as firmware success. M8 tests fail when explicitly
supplied assets/helper are invalid. The existing preparation script remains:

```bash
scripts/emulator/prepare_factory_a.sh \
  scratch/research/firmware/extracted/PRIME_APP.DAT /tmp/prime-factory-m8
```

## 16. Safety, artifacts and persistence

Firmware SHA-256 remains
`ad9779ff72db39fe4e9977ce7c4324f39aad1bb00c849c0aab067dc5bfc2fb39`;
OS ROM remains
`f6aeba161189a5aa1acb57be428dc5730cfa5ff7cd534494e59225debd5a8999`.
Original ADVPMT and Leasing20 hashes match the M5/M7 records. Source files are
snapshotted/read only; input-alias report/screenshot tests prove non-overwrite.
Only owned temporary roots are cleaned. Explicit reports and screenshots survive.
Retained work uses a new directory even when its parent already exists.

**The report and research tree are Git-ignored.** The ignore policy was unchanged;
authorized later persistence requires deliberately adding reviewed files.
No commit/stage was performed. The integration test's explicitly retained work
folders include firmware copies and potentially stale guest padding. The curated
index excludes those folders and saved binary programs. Never indiscriminately
publish the entire research tree. Nested backend paths marked ephemeral describe
historical guest storage; the frontend artifact list only advertises retained
outputs. Screenshots and JSON/raw header bytes also require publication review.

## 17. Remaining limits and next milestone

One pinned G1 image; firmware numeric arguments only; returned list/matrix/complex
representations and runtime numeric source positions remain unsupported in M7.
Firmware entry selection uses exports from one isolated imported program; it does
not add cross-program linking. Input replay uses M7 controls, not native INPUT/
CHOOSE simulation. No semantic fallback, GUI engine selector, warm-session cache
or daemon was introduced. Concurrent traced UI workloads can time out as noted.

All M8-A through M8-F primary behaviors have genuine execution evidence. Next:
M9 can reuse the backend library and cancellation/results without moving process
orchestration into widgets, while preserving the native GUI's revision tracking.

## Public repository snapshot

The engineering reports are explicitly tracked in the publication commit; the
existing Markdown ignore pattern is unchanged. Earlier warnings about ignored
reports describe their pre-publication state. Proprietary firmware, raw guest
drives remain uncommitted. A subsequent user-requested save tracks the existing
hashed-device ownership receipts and transfer ledger without altering their data.
The firmware process backend is Linux-only. Other platforms retain the native
engine and receive an explicit unsupported-platform result for firmware requests.
