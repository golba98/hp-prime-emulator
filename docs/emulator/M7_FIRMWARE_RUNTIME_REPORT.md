# M7 — Original HP Prime G1 PPL runtime

Date: 2026-10-24. **M7 COMPLETE — ORIGINAL FIRMWARE PPL RUNTIME VERIFIED**.
Branch: `rust-rewrite`. All six primary stages are verified below.

## Assets, baseline and safety

This work uses the same G1 2.4.2 image as M6:
`armfir.elf` SHA-256
`ad9779ff72db39fe4e9977ce7c4324f39aad1bb00c849c0aab067dc5bfc2fb39`;
OS ROM SHA-256
`f6aeba161189a5aa1acb57be428dc5730cfa5ff7cd534494e59225debd5a8999`;
factory `armfir.dat` SHA-256
`cf50aa599e236a10d24761f15217c5c97473603f208fb4ba14b4cd84c36b8edf`.
Other firmware images are refused by the existing M6 hash gate.

M6's HELLO/BROKEN baseline was reproduced before runtime implementation
(exit 0, 20.44 s). Its `/tmp` artifacts were lost during an environment
restart. It was reproduced again into persistent research storage:
`validation/m6-baseline.log` and `validation/firmware-checks.json` below.
The second reproduction passed (exit 0, 13.49 s test time; 20.17 s command wall time), including the
exact BROKEN line 3, UTF-16 offset 51, column 30 assertions.

All generated guest storage is isolated beneath
`scratch/research/m7-private/`. The factory tree and initial program-free
storage are copied, never cleaned in place. No physical calculator commands,
USB communication, device transfer records, original samples or original
firmware archives were modified. Existing unrelated native IDE/compiler/VM
changes were preserved. No commit was made.

Evidence paths in this report are relative to
`scratch/research/m7-evidence/`, unless stated otherwise. It contains logs,
JSON reports and PNGs, not copies of the firmware/factory drive. Private guest
storage and firmware-saved `.hpprgm` files may contain stale memory padding;
they remain separate and require review before any publication.

**Persistence:** `docs/**/*.md` and `scratch/` are ignored by this repository.
This report and research evidence exist on disk but are not protected by Git.
The ignore policy was not changed. A later authorized commit must deliberately
include reviewed source/report files; do not indiscriminately add private data.

## Genuine UI reference and invocation

The initial discovery used Program Catalog **Run**, after Editor **Check**.
A persistent reproduction is `ui-reference-command.json`, exit 0:

```text
SHIFT,1,TAP:26:229,TAP:293:229,ENTER,ESC,TAP:293:229
```

`ui-reference.png` and `ui-reference/07_TAP_293_229.png` show the original
Terminal displaying **Hello from HP Prime!**. The raw report includes the
actual compiler observation. This is calculator rendering, not OCR or a
host-drawn terminal.

The general headless interface uses the same boot/import/Check pipeline as
M6, then the original **Home evaluator** resolves an exported function. It
enters the function name with real calculator key events. Numeric arguments
use the calculator's parenthesis/comma/sign/EEX input. No guest registers,
source/value buffers, runtime stacks or framebuffer pixels are injected.

A zero-argument function is entered as its **bare name**. Initial exploration
entered literal `HELLO()` and observed a genuine Home Syntax Error before
runtime entry. That exploratory `/tmp` screenshot was lost at restart; the
failure is recorded here, not presented as retained evidence. Bare-name
invocation was independently verified against Catalog Run. Wrong argument
counts also produce **Error: Syntax Error** on this firmware, before the PPL
body. They are invocation failures, not errors from an executing body.

## Runtime architecture and observation ABI

All following addresses are ARM instructions in the pinned image. They are
passive observation anchors, not a supported external-call ABI:

| Address | Observed operation |
|---|---|
| `0x306D2B48` | User PPL function callback; r0 definition, r1 argument vector, r2 argc |
| `0x306D2B90` | Argument check passed, body frame setup; r6 definition |
| `0x306D171C` | Evaluator return after frame cleanup; r5 evaluator context, r4 result object |
| `0x30978C08` | String PRINT terminal append; r1 actual UTF-16 text |
| `0x30978C40` | Non-string PRINT converted by firmware to text; r1 UTF-16 text |
| `0x30978B84` | PRINT() terminal clear |
| `0x306DDDC4` | Runtime error resource resolved; r4 error object, r0 UTF-16 message |
| `0x306D0B6C` | Composed error/statement/function context; r1 text, r5 evaluator |

Disassembly also exposes an object-rendering PRINT branch at `0x30978BE0`.
The observer retains a typed guest value if this branch is encountered. The
verified default terminal path for PRINT(49) is the text-formatting branch,
which yields actual guest text `49`; object-rendering mode is not claimed as
independently tested.

At function entry, definition+0x10 is the program context already observed
by M6, definition+0x20 points to its compiled function node, and node+8 is
the UTF-16 function name. `(node_word >> 24) & 31` is declared arity. The
argument vector contains argc value pointers followed by an evaluator-context
pointer at `argv[argc]`. These were verified for zero, one and two arguments,
nested calls and recursion. The controller requires the selected function and
checked program context to match before correlating execution.

The callback schedules evaluation. Its own return is **not completion**.
The observer requires the captured root evaluator context at `0x306D171C`
and its frame-stack pointer at context+8 to be zero. Nested returns with a
nonempty stack cannot finish the invocation. A synthetic ARM hook test
specifically checks this distinction. Observed executions use main thread 0;
thread identity is retained rather than assumed by the completion observer.

Boot, checking and key delivery retain M4's event-pump/main-thread/framebuffer
synchronization. Runtime completion comes from evaluator teardown, not UI
idleness. WAIT(0), looping programs, cancellation and timeouts are tested.
The optional input replay starts only after function entry. Cooperative stop
checks now stop engines between slices and after SVC return, and allow a
critical-section waiter to stop without acquiring another thread's lock.

## Guest values and argument encoding

Numbers are not assumed to be ARM integers. The implemented checked codec
covers these observed layouts (little endian):

| Kind (`byte2 & 15`) | Layout |
|---|---|
| 0 | 16-byte decimal: signed byte3 (-1/0/1), i32 exponent at +4, twelve BCD digits in u64 bits 12..59 at +8 |
| 15, subtype byte3=0 | Embedded 16-byte decimal at object+8 |
| 15, subtype byte3=1 | Exact rational: negative flag in byte4, u64 numerator +8, u64 denominator +16 |
| 2 | u32 UTF-16 unit length +4, NUL-terminated text +8 |
| 5 | Error enum in byte3; not an ordinary number |

Unreadable tags/subtypes are null; readable header tags remain available even
when the payload representation is unsupported (LISTRET: raw tag 0x16, subtype
0x20). No fabricated zero tag is reported for an unreadable object.
Value objects have additional reference/flag bits which the codec does not
interpret. It validates readable bounds, supported tags, decimal nibbles,
signs/exponents, denominator and UTF-16. Unknown representations return an
explicit unavailable reason and an unsupported outcome; they do not become
zero. Bounded representation bytes accompany decoded facts. This is a codec,
not a PPL evaluator.

Verified SQUARE(7) input is rational 7/1; its result is rational 49/1. Terminating
rationals are reported as exact decimal strings; other u64 fractions retain
numerator/denominator. Both embedded BCD results (scientific notation) and
compact rational results were exercised. Strings are strictly decoded without
lossy surrogate replacement. No host floating-point conversion is used.

Input is a JSON numeric array or tagged decimals, e.g. `[7]` or
`[{"type":"real","value":"1e400"}]`. JSON numeric spelling is retained by
`RawValue`, then canonicalized as decimal text. The firmware owns its precision
and rounding. The report preserves both requested arguments and actual guest
argument objects. Supported inputs are finite decimal numbers in the verified
G1 exponent range. String/list/matrix/complex arguments are explicitly refused.

Export selection uses the firmware-saved M6 cache and its export bit. Cache
flags 9/25/41 correspond to observed 0/1/2-argument exports, confirmed against
function-node arity. Raw cache flags are retained. They support classification
of a pre-body argument-count failure only when actual guest error text was
also observed. Other pre-body indeterminate outcomes remain unsupported.

## Verified behavior

| Program | Genuine firmware result |
|---|---|
| HELLO | PRINT text `Hello from HP Prime!`; final returned object is the same string |
| SQUARE(7), (0), (3), (-4) | 49, 0, 9, 16; guest arguments and returned objects captured |
| SQUARE(0.1), (1e20), (1e-20) | 0.01, 1e40, 1e-40 |
| DEMO | Unchanged forward/helper arrangement accepted; local result 49, PRINT `SQUARE(7) = 49`, return 49 |
| FACT(5) | Recursive callbacks and final result 120 |
| ADDTWO(2,3), MiX_sum(-4,7) | 5 and 3; mixed case, underscore and two arguments verified |
| DRAWTEST | Blue guest LCD with antialiased near-white text at (30,50); final expression returns 104 |
| GLOBALTEST / INC | Same-guest sequential results 1, 2, 3 |
| WAITING | WAIT(0), scripted ENTER, return 7 |
| LOOP | Deadline/cancellation; never reported completed |
| BROKEN | M6 compile rejection prevents runtime arming |
| Missing export | Invalid entry, no runtime invocation |
| SQUARE with 0 or 2 arguments | Home Syntax Error, no PPL body entry |

DRAWTEST framebuffer capture happens **at evaluator return**, before Home
redraws over it. In each repeated boot the 320×240 capture contains 76,459
exact blue pixels, 134 near-white pixels, and the same antialiased text region. Near-white statistics
and PNG hashes are retained in `reliability/DRAWTEST-*/`. A pure-white pixel
assertion would be wrong: the firmware renderer uses antialiasing with channel
values near 254. Native renderer bytes are not used as a golden image.

## Genuine runtime errors

All error fixtures first compiled successfully. Their final returned error
objects and resolved resource text were captured independently of source text:

| Fixture | Raw guest enum | Actual firmware text |
|---|---:|---|
| ERRLIST: access element 5 of {1,2} | 24 | `Error: Invalid input` |
| ERRTYPE: SIN("x") | 47 | `Error: Bad argument type` |
| ERRDIM: incompatible matrix product | 57 | `Error: Invalid dimension` |
| ERRZERO: 1/0 | 4 | `Error: X/0` |

For example the list-error context is exactly
`Error: Invalid input\nRETURN L(5)\nERRLIST`. It is preserved as `raw_context`.
No numeric runtime line, column or source-offset representation was established;
those fields are null. Raw enums belong to this pinned guest, not an invented
SDK namespace. Matching the final returned error enum selects its diagnostic;
intermediate or caught errors are not automatically reported as terminal errors.
SEQUENCE runs FAIL, GOOD, CATCHED, GOOD in one guest; caught failure returns 17,
and each GOOD returns 7 with a new clear/49/Fresh output event sequence.

## Headless interface, reports and exit status

```bash
FW="$PWD/scratch/research/firmware/extracted"
A="$PWD/scratch/research/m7-private/factory/A"
scripts/emulator/run.sh run --release --bin prime-emulator -- run-program \
  --program crates/prime-emulator/tests/fixtures/m7/SQUARE.ppl \
  --entry SQUARE --args '[7]' \
  --firmware "$FW/fat/armfir.elf" --os-rom "$FW/PRIME_OS.ROM" \
  --factory-a "$A" --out-dir /tmp/prime-m7-runs \
  --report /tmp/prime-m7-square.json --screenshot /tmp/prime-m7-square.png
```

Real `.hpprgm` containers use the same option. A plain source file carrying
that extension requires M5's existing `--as-source`. Optional `--init-data`
accepts initialized **program-free** storage, copied before use. Omit it for
fresh factory setup. `--execution-timeout-s` bounds the actual PPL phase;
`--max-seconds`/`--max-instructions` bound the guest session.
`--runtime-keys ENTER` resumes the WAIT fixture. Ctrl-C requests cancellation.
`--trace-runtime` adds framebuffer writer counters, not instruction tracing.

The public `runprog::run` API also accepts up to 32 sequential requests in the
same checked guest; the CLI issues one request. This API underlies sequential,
global-state and recovery tests. No M8 CLI or M9 IDE integration was added.

Reports contain engine, program/source hash, complete M6 result, selected entry,
requested/observed arguments, callback frames, invocation indices, thread and
context pointers, precise guest result evidence, output events, diagnostics,
status, snapshots, timing and heap measurements. The unique work directory
identifies the guest execution instance; invocation indices are local to it.
A fresh invocation starts with no inherited output/result/error. Sources are
checked again after firmware save. Unknown services outside M4's two known
baseline stubs invalidate otherwise conclusive outcomes.

Text output is actual guest UTF-16. Numeric PRINT can use firmware-formatted
text. Object-mode events explicitly carry a typed object, not guessed text.
A terminal clear is an event rather than silently deleting earlier events.
A PNG is independent framebuffer evidence; no OCR is used for structured output.

| Exit | Meaning |
|---:|---|
| 0 | Completed, root evaluator teardown and supported result observed |
| 11 | Genuine PPL runtime error |
| 7 | Genuine compiler rejection; no execution |
| 9 | No matching exported entry |
| 10 | Invalid arguments/input request or verified pre-body arity failure |
| 3 | Deadline/instruction budget, not source failure |
| 1 | Guest ARM execution fault |
| 4 | Infrastructure/load/report/capture failure |
| 8 | Unsupported or indeterminate observation |
| 130 | Caller cancellation |

## Validation, reliability and failures

All four required workspace commands returned exit **0**:

```bash
cargo fmt --all -- --check
scripts/emulator/run.sh check --workspace --all-targets
scripts/emulator/run.sh clippy --workspace --all-targets -- -D warnings
scripts/emulator/run.sh test --workspace --all-targets
```

`validation/checks.json` records their exact commands, exit codes and wall times.
The final workspace run reported **1,105 passed, 0 failed, 3 ignored**. Firmware
integration tests deliberately print SKIP when assets are unset during that
workspace command. Those returns are not counted as real-firmware evidence.
Nine synthetic runtime tests and two CLI error tests passed without firmware.

With assets supplied, **M4–M6: 13/13** passed: boot 1, display/calculation 4,
program storage/editor/persistence 3, compiler 5. The latter includes exact-source
HELLO/BROKEN checks, in-editor correction, back-to-back/navigation/Home arithmetic,
23 reference/diagnostic cases, and independent repeated boots. Exact command:

```bash
scripts/emulator/run.sh test -p prime-emulator --release \
  --test firmware_boot --test firmware_display \
  --test firmware_programs --test firmware_compiler \
  -- --test-threads=1 --nocapture
```

It returned exit 0 in 436.04 s including compilation/process overhead; test
binaries reported 0.79/352.39/31.81/44.30 s in execution order (boot/compiler/
display/programs). `validation/m4-m6.log` retains all outcomes.

The final M7 run of the seven main groups passed **7/7**, exit 0, 461.28 s test
time (`validation/m7-current.log`, command/environment/wall time in its JSON).
The additional unsupported-return test passed **1/1**, exit 0, 17.46 s
(`validation/m7-unsupported-final.log`). Together these establish **8/8 distinct
M7 firmware tests**. Strengthened exact requested-versus-observed argument checks
were then run for both numeric/function groups: **2/2**, exit 0, 199.86 s
(`validation/m7-arguments.log`). A prior corrected seven-group run also passed
7/7; the earlier assertion failure remains retained separately.

Reproduction environment and supported full M7 command:

```bash
FW="$PWD/scratch/research/firmware/extracted"
export PRIME_TEST_FIRMWARE="$FW/fat/armfir.elf"
export PRIME_TEST_OS_ROM="$FW/PRIME_OS.ROM"
export PRIME_TEST_FACTORY_A="$PWD/scratch/research/m7-private/factory/A"
# Optional initialized program-free copy; omit for fresh factory setup.
export PRIME_TEST_INIT_DATA="$PWD/scratch/research/m7-private/initial-ready/data"
export PRIME_TEST_RUNTIME_OUT="$PWD/scratch/research/m7-private/runtime"
export PRIME_TEST_COMPILER_OUT="$PWD/scratch/research/m7-private/compiler"
scripts/emulator/run.sh test -p prime-emulator --release \
  --test firmware_runtime -- --test-threads=1 --nocapture
```

The recorded M7 commands (including test filters) and exact environment values
are in `validation/m7-{current,arguments,unsupported-final}.json`. Factory
extraction uses the existing command:

```bash
scripts/emulator/prepare_factory_a.sh \
  scratch/research/firmware/extracted/PRIME_APP.DAT \
  scratch/research/m7-private/factory
```

The successful setup script and 8B instruction budget are recorded in
`validation/initial-command.json`. Inputs with explicitly supplied missing assets
fail; missing environment is the only intended M7 skip path. No success counts
are inferred from skipped runs.

`reliability/summary.json` records **20/20 independent boots**, five each of
HELLO, SQUARE, DRAWTEST and ERRTYPE. All five SQUARE inputs have independent
argument/result checks. Every error run requires successful compilation and
actual guest diagnostic text. Per-run commands, exit codes, JSONs, screenshots
and pixel statistics are retained. These were run with two workers and a
concurrent regression suite; timing is not an isolated benchmark.

Retained failures are not hidden:

* `validation/initial.json`: raw setup stopped at the raw CLI's default 20M
  instruction limit (exit 3). Retrying with the documented 8B budget passed.
* The first persistent M7 suite contains one test assertion failure: PRINT(49)
  was assumed to produce an object event. The guest actually formatted text
  `49`, captured at `0x30978C40`. Assertions now require that exact guest text;
  the execution, output and recovery checks were otherwise successful.
* Early research `/tmp` evidence, including the literal HELLO() failure, was
  lost at environment restart. Persistent reproductions replace the claimed
  final evidence; lost files are not linked as available artifacts.

## Performance

`performance.json` contains per-boot measurements and min/mean/max, with the
exact concurrent workload preserved in each command log. Means over five boots:

| Case | Total with initialized-storage copy | Compiler | Function entry to root return | LCD copy at return |
|---|---:|---:|---:|---:|
| HELLO | 15.793 s | 2.296 ms | 24.117 ms | 0.167 ms |
| SQUARE | 18.696 s | 3.100 ms | 0.826 ms | 0.184 ms |
| DRAWTEST | 18.424 s | 4.312 ms | 22.532 ms | 0.176 ms |
| ERRTYPE | 17.372 s | 1.835 ms | 4.707 ms | 0.196 ms |

These include host scheduling, guest callbacks and observer costs. Root-return
latency is not an isolated CPU execution benchmark. The UI invocation takes
about 6.8–9.2 s on average, dominated by observed settlement of name/argument
key events. Boot settlement is 0.697–0.743 s; discovery about 0.2 s. PNG output
adds roughly 1–2 ms. Main-thread throughput over the full session is about
6.5–6.9 million guest instructions/s; spawned-thread attribution is retained in
the raw report. This includes boot/evaluator idle work, not just PPL statements.

The final fresh-factory **container** HELLO CLI run took 22.614 s, including
factory setup, import, checking and runtime. Its exact string-capture work was
0.001132 ms; the LCD copy was 0.045130 ms. The traced SQUARE(7) run took 17.848 s
and returned in 0.831820 ms. DRAWTEST took 19.519 s, with 27.735844 ms from
function entry to root return under framebuffer write tracing. These individual
measurements are in `cli/*/result.json`, including import/init/compiler timings.

Guest heap tuples are `(allocated bytes, live blocks, largest free block)`.
In these final runs HELLO went from 5,322,160 to 5,322,592 allocated bytes,
SQUARE from 5,321,904 to 5,321,872, and DRAWTEST from 5,323,888 to 5,326,608.
They describe firmware allocations/cache lifetimes, not proof of leak freedom.
The independent post-presentation screenshot `cli/SQUARE/home-after-return.png`
shows the firmware Home answer 49. Its structured result was already read before
that redraw. Runtime errors use a settled error-presentation PNG; `capture_ms` records the
initial LCD copy at evaluator return, before that presentation capture.
M6's inherited `compilation.timings.guest_session_s` in a runtime report spans
the shared checking-and-execution session; `compiler_ms` isolates its actual
compiler call. The top-level total is the entire headless operation.

## Native versus firmware

Independent native commands and exit codes are in `native/commands.json`.
No native compiler/VM is linked into the firmware emulator.

| Property | Native | Pinned firmware |
|---|---|---|
| HELLO output | Same PRINT text | Same PRINT text |
| HELLO implicit return | 0 | Final PRINT string |
| SQUARE(7), DEMO | 49 | 49 |
| Invalid list subscript | Invalid dimension (with index detail) | Invalid input |
| SIN string | Bad argument type, host detail | Bad argument type, guest text/context |
| Matrix dimensions | Invalid dimension, host detail | Invalid dimension, guest context |
| Division by zero | Division by 0 | X/0 |
| Graphics | Native font/rasterizer | Original firmware rendering |
| Error positions | Native source spans | Runtime numeric positions unavailable |

These discrepancies were recorded without altering the native engine.

## Stage evidence and artifact index

| Stage | Verified evidence |
|---|---|
| M7-A | Catalog Run, Home export resolution, callback/body/evaluator anchors |
| M7-B | HELLO guest UTF-16 PRINT events and original Terminal PNG |
| M7-C | Exact argument objects and results for SQUARE, helper calls, recursion, two-argument functions |
| M7-D | Guest framebuffer writes and DRAWTEST pixel/image checks |
| M7-E | Four successfully compiled runtime-error categories, actual text/context and guest error objects |
| M7-F | Generic headless CLI/API, source identity, independent boots, bounded waiting/cancellation, structured distinct outcomes |

`index.json` lists evidence paths, SHA-256 hashes and sizes. `source-manifest.json`
hashes implementation/test sources, fixtures and lockfile. `runtime/` contains
curated reports and return snapshots from isolated guest runs; `compiler/`
contains M6 reports. `cli/` contains exact container/fresh-boot, SQUARE, graphics,
arity, compiler-rejection and timeout command results (all expected exits).
`native/` contains independent comparison outputs. `ui-reference.png` is the
original Catalog Run terminal; `reliability/DRAWTEST-0/screen.png` is the guest
graphics result. `sample-hashes.txt` matches the original M5 sample hashes.

Raw report/header bytes still require review before publication; the ignored
bundle is research evidence, not an approved distributable firmware artifact.

## Implementation and remaining scope

Added `runtime.rs` (passive lifecycle/output/error hooks), `runtime_value.rs`
(checked codecs and lossless numeric input), `runprog.rs` (controller/report),
`tests/runtime.rs`, `tests/run_program_cli.rs`, `tests/firmware_runtime.rs` and
public source fixtures under `tests/fixtures/m7/`. Modified the emulator's
`main`, `lib`, `runner`, `engine`, `trace`, `checkprog`, `calcverify`, and
critical-section shutdown handling in `services/{sync,threading}.rs`.
M6 preparation/session/finalization is shared rather than duplicated.
Cargo adds ctrlc and serde_json's raw_value feature; existing lockfile changes
were preserved. Native crates and the native IDE were not modified by M7.

Limitations: one pinned firmware; numeric arguments only; supported returned
real/rational/string/error representations only; no runtime numeric source
locations; object-rendering PRINT branch not independently exercised; arbitrary
modal workflows may require caller key/touch replay and remain bounded by
cancellation/deadlines. One container is imported per isolated session; cross-program
namespace/linking behavior is not established. Error-text extraction overhead is
not separately timed, though guest event timestamps/context are retained.
Input/execution phase timing includes emulator scheduling
and observer overhead. Main guest instruction counts do not claim instruction
attribution to individual PPL statements. Initialization uses the existing M6
pipeline; cancellation during that preparation is honored at the next phase,
not asynchronously inside factory copying/setup.

Next milestone: M8, exposing this verified firmware engine through the unified
`ppl` CLI while retaining engine-specific values and diagnostics. M9 IDE work
remains separate.
