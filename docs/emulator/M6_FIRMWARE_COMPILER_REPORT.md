# M6 — Authentic G1 firmware PPL compiler and diagnostics

Date: 2026-10-24. Status: **M6 COMPLETE — GENUINE FIRMWARE PPL COMPILER VERIFIED**.

## Baseline, assets and safety

Branch `rust-rewrite`. Existing 32 modified tracked files and 61 untracked
files were preserved. Initial inspection included all 105,540 ignored files
(summarized by directory rather than printing build artifacts).

The supplied G1 2.4.2 firmware is `scratch/research/firmware/extracted/fat/armfir.elf`,
SHA-256 `ad9779ff72db39fe4e9977ce7c4324f39aad1bb00c849c0aab067dc5bfc2fb39`.
`PRIME_OS.ROM` SHA-256 is
`f6aeba161189a5aa1acb57be428dc5730cfa5ff7cd534494e59225debd5a8999`.
Factory A: `/tmp/prime-factory/A`; `programs/misc/armfir.dat` SHA-256
`cf50aa599e236a10d24761f15217c5c97473603f208fb4ba14b4cd84c36b8edf`.
No physical calculator, firmware archive, or original sample was modified.

Before implementation, genuine boot 1/1, M4 4/4 and M5 3/3 passed (exit 0).
A fresh six-program M5 reproduction passed for HELLO, SQUARE, TEST, MULTI,
ADVPMT and Leasing20 through both boots. Evidence:
`/tmp/prime-m6-plan-m5-2b3m98x3/report.json`, with screenshots beneath
`session1/shots` and `session2/shots`; exact command is in `command.log`.
Session wall times: initialization 6.426 s, control 2.934 s, first program
session 20.811 s, reboot session 20.277 s.

Initial workspace testing exited 101 on the VFS path tests: both used the
same PID/line-number temporary path and removed each other's directory.
Serial testing passed. Fixed only their fixture ownership, using independent
`TempDir`s. The boot integration test now also defaults to a unique temporary
directory, retaining its explicit `PRIME_TEST_DATA_DIR` override.

## Verified invocation and ABI

The reference workflow is the original UI, not a host-invoked internal call:

1. Copy and initialize factory storage through the M4 unlock/setup script.
2. Import with the unchanged M5 importer before the checking boot.
3. Boot; `SHIFT,1` opens Program Catalog; `TAP:26:229` opens Edit.
4. `TAP:293:229` activates the editor's **Check** soft key.
5. Observe the guest compiler, its exact source, return and resolved dialog.
6. `ENTER,HOME,ESC,IDLE` dismisses the result and any exit warning.

Leaving invalid source opens an additional genuine warning, "Syntax error in
program line 3" (displayed across lines). Its dismissal is required before
catalog navigation. An initial navigation test omitted this and reopened
BROKEN; the exact-source guard caught the mismatch. The failing log was
retained in `/tmp/prime-m6-discovery/firmware-tests.log` (exit 101). Corrected
navigation explicitly acknowledges the warning. No acceptance check was weakened.

All following instruction addresses are ARM, observed on the pinned image:

| Address | Verified role |
|---|---|
| `0x308F2C60` | Program Editor Check handler |
| `0x308F2C90` | Call to the program compiler |
| `0x306BB740` | Program compiler entry called by Check |
| `0x306BB8B8` | Call to PPL source parser `0x306D4BC8` |
| `0x306BB8E0` | Parser return/status consumer |
| `0x308F2C94` | Check's compiler return consumer |
| `0x30685B28` | Dialog creation; Check caller return address `0x308F2CF4` |
| `0x30685B54` | Resource text resolved into a guest UTF-16 string in r0 |
| `0x306BB978`–`0x306BB990` | Failure location copied into Check's result record |
| `0x30B68CC0` | Editor cursor positioning using the failed source offset |

At the program compiler entry observed from Check:

- r0: initialized guest program context pointer, not a source string.
- r1 = 1, r2 = 0: observed Check arguments; their complete semantic names
  are not established.
- r3: guest pointer to three result words on the Check handler's stack.
- lr = `0x308F2C94`.
- Return r0 = 1 on acceptance, 0 on rejection in the observed cases.

The parser call receives the complete NUL-terminated UTF-16 source in r0,
parser result/context pointers in r1/r2, and source index in r3. The observer
strictly decodes UTF-16 (including surrogate pairs) and hashes its UTF-8
representation. That hash and UTF-16 length must match the imported source.
Neither parser arguments nor guest registers/memory are changed by observation.
Guest allocation, stacks, error handling and source-buffer cleanup remain
entirely under the original UI's ownership. This is not a reusable direct-call
ABI: allocator lifecycle and arbitrary external-call preconditions are not claimed.

Check's failure record is three signed 32-bit words:
`source_index, one_based_line, zero_based_utf16_offset`. Disassembly counts
newlines starting at 1 and stores `(error_pointer - source_pointer) >> 1`.
On success only the first word is initialized to -1; the other words are
stale stack data and are deliberately not read/reported.

The dialog resources observed are `0x410` for success and `0x3416` for the
error presentation. Production code validates those identities but reads the
actual text from guest memory; it contains no hardcoded success/error message
and makes no decision by matching program names, source text or OCR.

## HELLO and BROKEN: independent guest evidence

HELLO's parser input was exactly:

```text
EXPORT HELLO()
BEGIN
  PRINT("Hello from HP Prime!");
END;
```

Its parser returned 0, its program compiler returned 1, and its genuine UI
and guest UTF-16 message said **No errors in the program**. The export cache
contained `(HELLO, 9)`. Source was preserved. This did not execute PRINT.

BROKEN's exact imported source reached the same parser. Its parser returned
22 (`0x16`), the program compiler returned 0, and the genuine message was
**Error: Syntax Error**. Raw result: source index 0, line 3, UTF-16 offset 51.
The editor positions its cursor at the semicolon, rather than the opening
parenthesis. The derived Unicode-scalar column is 30.

The parser status is retained in `observation.parser_returns`; it is not
presented as a documented HP diagnostic code. `diagnostics[].code` stays null.
The diagnostic message and raw position originate in the guest. The top-level
line/column fields are a checked host mapping: column is one-based Unicode
scalars, not UTF-8 bytes, UTF-16 units or screen pixels. A mapping that cannot
be verified returns null while preserving the raw guest record.

The observed compiler exits on the first parser error. A separate TWOERRORS
probe contained both a missing expression and a later missing parenthesis;
Check returned one diagnostic at the first error (4:7). Report:
`scratch/research/m6-evidence/cli/TWOERRORS.json` (exit 7). Tested invalid cases
produced one message each. The report supports multiple diagnostics as a data
format but the current firmware observer exposes one per Check.

## Cases and native comparison

The checked-in `tests/fixtures/compiler_cases.json` contains 23 observed G1
source cases: 9 accepted and 14 rejected. This includes multiple exports,
LOCAL, nested IF/FOR, strings, comments, blank lines, Unicode including a
surrogate pair, CRLF inputs, a 250-comment program, malformed headers, missing
parentheses/expressions/END/semicolons, incorrect nesting, duplicate locals,
unknown functions/identifiers, arity errors and malformed strings.

CRLF **source inputs** are normalized to LF by M5; this conversion and both
input/source hashes remain in the import report. These results do not claim
that a CRLF-containing guest buffer was passed through unchanged.

| Case | Firmware | Guest line : mapped scalar column |
|---|---|---|
| BROKEN, missing parenthesis | invalid | 3:30 |
| MISSEXPR | invalid | 4:7 |
| MISSEND | invalid | 4:1 (EOF) |
| MISSSEMI | invalid | 4:2 (following statement) |
| NESTERR | invalid | 5:2 |
| DUPLICATE | invalid | 3:11 |
| HEADER | invalid | 2:6 |
| UNKNOWN | invalid | 1:24 |
| IDENT | invalid | 1:29 |
| ARGCOUNT | invalid | 1:37 |
| STRERR | invalid | 5:1 (EOF) |
| COMMENTERR | invalid | 5:14 |
| UNICODEERR | invalid | 3:12; raw UTF-16 offset 38 |
| CRLFERR | invalid | 4:14; raw imported-LF offset 37 |

All above diagnostics read **Error: Syntax Error** on this English factory
configuration. This is an observation, not a message synthesized by the host.

Native comparisons used the independent `ppl check` implementation:

- BROKEN: native reports `unclosed-delimiter` at 3:8 (opening parenthesis);
  firmware reports the semicolon at 3:30. Both reject; locations intentionally differ.
- DUPLICATE (`LOCAL A,A;`): native accepts with duplicate/unused-local warnings;
  firmware rejects. No native semantic change was made to force agreement.
- ADVPMT: firmware accepts and exports ADVPMT with flags 89; native also
  accepts, warning that Finance.TvmPV is unsupported for native execution.
- Leasing20: firmware accepts. Native rejects with 35 errors and 39 warnings,
  beginning with lowercase `export` top-level declarations and cascading
  undeclared-variable/function diagnostics. Those differences need separate
  semantic investigation, not blind compatibility patches.

Original external samples remained read-only:
ADVPMT SHA-256 `bc812d64ad7cbe876af4ac841cd7a4d9872c9afd1ee3947edebe9302feb164a8`;
Leasing20 `ac54830805691d016194b1e05c2146c49363e5aaca875b19548876d7190c3459`.
Their native input was extracted through M5 inspection into separate .ppl files
and checked against the firmware report's source hash before native compilation.

Matrix reports/native logs: `/tmp/prime-m6-matrix/`; summary `summary.json`.
`native-comparison.json` records all 25 genuine source cases with native
diagnostic categories, messages, printed positions, compiled function names
and parameter counts alongside firmware diagnostics/exports. Only DUPLICATE
and Leasing20 differ on acceptance. Native CLI function dumps do not expose
the complete firmware flag vocabulary.
An additional EMPTY plain-source test was rejected by M5 as empty input
(exit 4); it is **not** counted as a genuine firmware source verdict.

## Metadata and caches

Filesystem tracing attributes Check's `wb+` save to script step 3,
caller `0x3068DFF8`; earlier catalog/editor reads use `rb+` from `0x3068DFD8`.
Both valid and invalid programs can be rewritten; save occurrence alone is
not evidence of acceptance.

- HELLO: source-only file 1200 bytes became 1296 bytes; only canonical section
  `0x0100023E` (export cache) changed. BROKEN remained byte-identical.
- MULTI: FIRST flags 9, SECOND flags 25; only the export section changed.
- ADVPMT: existing cache flags 9 became 89 after genuine checking. A supplied
  cache is not trusted and cannot satisfy the compiler-result gate.
- Leasing20: export records stayed the same; canonical sections
  `0x00C001BE` (variables) and `0x0240028E` (ten-slot table) changed.
  Their complete meaning is not established.

Observed flags correlate with 9 | (argument_count << 4) for exported functions
(0, 1 and ADVPMT's 5 arguments); this is not a complete flag specification.
No new checksum/version/timestamp field was identified. In-RAM compiled logic,
constant-cache internals and the complete serialized variable/table ABI remain
unknown. Metadata diffs do not replace compiler return/dialog evidence.

Before/after containers are retained per run as `before.hpprgm` and
`after.hpprgm`. Comparison canonicalizes M5's known name padding. Raw guest
saves may still contain stale allocator bytes; do not publish/commit them.

## Headless API and classification

`checkprog::check(&CheckConfig) -> CheckReport` is reusable without the CLI.
`checkprog::validate` separately validates captured guest evidence.
There is no dependency from this path on ppl-syntax, ppl-compiler or ppl-vm.

```bash
FW="$PWD/scratch/research/firmware/extracted"
scripts/emulator/run.sh run --release --bin prime-emulator -- check-program \
  --program /path/to/HELLO.hpprgm \
  --firmware "$FW/fat/armfir.elf" --os-rom "$FW/PRIME_OS.ROM" \
  --factory-a /tmp/prime-factory/A \
  --out-dir /tmp/prime-m6/hello --report /tmp/prime-m6/hello.json
```

For plain text named .hpprgm use `--as-source`, as in M5. `--name` preserves
the requested program name independently of its exported function names.
Optional `--init-data DIR` copies initialized **program-free** storage to
avoid repeating setup. It never cleans or modifies that input directory.

Each invocation creates a unique directory under --out-dir (default parent:
`/tmp/prime-firmware-checks`) and retains evidence. Existing directories are
never removed. JSON is emitted to stdout and optionally --report; a failed
report write exits 4. Positive boot/key/settling/instruction limits are
validated; --max-seconds and --max-instructions bound each guest session.
There are no arbitrary synchronization sleeps: M4 observes event delivery,
UI quiescence and settled frames; additional compiler hooks observe completion.

| Status | Exit | Meaning |
|---|---|---|
| valid | 0 | Fresh matching guest parser input, successful compiler return and success dialog |
| invalid | 7 | Genuine failed Check with error presentation |
| infrastructure_error | 4 | Input/container/configuration/I/O/guest failure |
| timeout | 3 | Guest instruction/time budget or boot/key deadline exhausted |
| unsupported | 8 | Unknown firmware profile, unverified service or ambiguous/missing extraction |

Every source verdict requires the firmware fingerprint, exactly one imported
program discovered at boot, one Check for its script step, actual compiler
entry with the observed ABI, exact source hash/length, parser status,
compiler return and resolved guest dialog, completed input script and preserved
source. Missing/truncated/mismatched evidence cannot be a source verdict.
The only tolerated unsupported OS calls are the pre-existing M4/M5 baseline
`_FindResourceW` (0x10293) and `GetApplicationHeadInfoA` (0x102CB).

`run --observe-compiler` enables the same passive, version-pinned observations
for diagnostic scripts, adding a `compiler` object to its report. Normal
run/import/verify commands have no compiler hooks installed by default.

## Validation and reliability

Final workspace commands, all exit 0:

```bash
cargo fmt --all -- --check
scripts/emulator/run.sh check --workspace --all-targets
scripts/emulator/run.sh clippy --workspace --all-targets -- -D warnings
scripts/emulator/run.sh test --workspace --all-targets
```

Workspace totals: **1080 passed, 0 failed, 3 ignored**. Thirteen environment-gated
firmware test functions return without firmware when unset in that command;
those entries are not claimed as genuine guest validation. Explicit firmware
runs are recorded separately below. Logs/exit records:
`/tmp/prime-m6-validation/{fmt,check,clippy,test}.log` and `commands.json`.

New synthetic tests cover actual Unicorn hook interception on synthetic ARM
instructions, strict/bounded UTF-16 decoding, surrogate positions, stale and
mismatched results, missing/invalid data, event limits, report/diagnostic
serialization, names, exit mapping, report-write failure and unique storage.

Firmware reproduction (assets are required, not bundled):

```bash
FW="$PWD/scratch/research/firmware/extracted"
PRIME_TEST_FIRMWARE="$FW/fat/armfir.elf" \
PRIME_TEST_OS_ROM="$FW/PRIME_OS.ROM" \
PRIME_TEST_FACTORY_A=/tmp/prime-factory/A \
PRIME_TEST_COMPILER_OUT=/tmp/prime-m6-final-tests \
  scripts/emulator/run.sh test -p prime-emulator --release \
  --test firmware_compiler --test firmware_display \
  --test firmware_programs --test firmware_boot -- --test-threads=1 --nocapture
```

Optional `PRIME_TEST_INIT_DATA` supplies a program-free initialized snapshot;
the final regression command omits it and performs its own first initialization.
`PRIME_TEST_COMPILER_OUT` retains per-case results, screenshots, raw runner
reports and before/after containers. The tests fail when explicit asset paths
are invalid, rather than silently claiming a skipped firmware run.

Final firmware command: **exit 0, 13/13 passed, no skips or ignored tests**:
boot 1/1 (0.79 s), M6 5/5 (353.65 s), M4 4/4 (31.45 s), M5 3/3 (44.18 s).
Log: `/tmp/prime-m6-discovery/final-firmware-tests.log`. The newly isolated
boot fixture was then rebuilt and independently rerun: exit 0, 1/1 passed,
0.79 s (`/tmp/prime-m6-validation/boot-isolation.log`).

M6 retained 46 independent headless source reports (21 valid, 25 invalid),
plus five compiler checks in navigation/correction sessions: **51 genuine
Check invocations**. Reliability includes ten HELLO boots and ten BROKEN
boots, alternating sources, a new corrected BROKEN boot, and an actual editor
correction in one boot. In the editor, Check moved the cursor to the error;
raw key 0x4E inserted (), BACK removed its opening half, and the next Check
accepted the corrected source. The source hashes differ and stale-source
validation fails as required. Mixed-session checks were invalid → valid →
invalid while navigating catalog rows. Home `2+2` afterwards matched an
independent `4` result glyph through M4's existing pixel verifier.

Fresh CLI runs including independent factory setup returned HELLO exit 0,
BROKEN exit 7; a genuine run with `--max-seconds 0.01` returned timeout/exit 3,
with no syntax diagnostic or inherited compiler result. Exact argument arrays,
exit codes, reports and timings: `/tmp/prime-m6-final-cli/commands.json`.

| Measurement | HELLO | BROKEN |
|---|---|---|
| Initialization/setup | 6.804 s | 6.803 s |
| Import | 0.405 ms | 0.467 ms |
| Checking boot to settled UI | 0.633 s | 0.613 s |
| Last boot file-discovery/read event | 0.159 s | 0.160 s |
| Check handler entry to compiler return | 1.646 ms | 1.284 ms |
| Return to resolved diagnostic text | 0.048 ms | 2.126 ms |
| Checking session incl. navigation/settling | 7.206 s | 6.653 s |
| Total incl. setup, copying and reporting | 14.170 s | 13.616 s |
| Main-thread instructions / session wall second | 9.08 M | 9.56 M |

These are release-build, host-paced wall measurements including passive
observation and synchronization, collected alongside regression activity;
they are not a cycle benchmark. Discovery time is the last pre-script fread
event, not a separately instrumented catalog-enumeration timer. Across the
46 final source reports, mean observed compiler latency was 1.973 ms.

For the initial paired headless runs, live guest heap inventory before/after
Check was unchanged: HELLO 5,316,704 bytes / 1,407 blocks; BROKEN 5,316,400
bytes / 1,394 blocks. These are live inventories, not peak allocation counts.
All per-case before/after inventories and largest-free-block sizes are in the
observations. Normal emulator runs install no compiler hooks; M6 uses seven
singleton-PC hooks rather than full-instruction tracing.

With a copied initialized snapshot, a separate isolated HELLO check took
7.022 s total (6.846 s guest session), with 1.745 ms compiler latency.
Report: `scratch/research/m6-evidence/cli/HELLO-initialized.json`.

Fresh CLI screenshots: `/tmp/prime-m6-final-cli/runs/check-JshUT6/check/shots/04_TAP_293_229.png`
(HELLO), `/tmp/prime-m6-final-cli/runs/check-Z7gFHt/check/shots/04_TAP_293_229.png`
(BROKEN). Their `08_IDLE.png` frames were visually verified as Home after
result/warning dismissal. Curated copies and a machine-readable evidence index
are retained at `scratch/research/m6-evidence/`.

## Changed files and limits

Production additions: `src/compiler.rs` (passive guest observer),
`src/checkprog.rs` (isolation, automation, result validation/reporting).
Wiring: `src/{main,lib,trace,runner,engine}.rs`; tempfile moved from a dev-only
to a normal dependency in the emulator manifest. Test additions:
`tests/{compiler,check_program_cli,firmware_compiler}.rs` and
`tests/fixtures/compiler_cases.json`. Fixture fixes:
`src/services/fs.rs` and `tests/firmware_boot.rs`. All paths in this paragraph
are relative to `crates/prime-emulator`.

Supported extraction is deliberately limited to the verified firmware hash
and M5's single PPL source-file container representation. Multiple source files,
CAS source paths, unknown firmware versions and larger-than-bounded buffers
must not inherit a verified result. Only first-error reporting has been
established. This milestone does not run imported PPL entry points or integrate
the firmware engine into the native CLI/IDE.

The docs/**/*.md ignore policy is unchanged: this report is ignored and persists
only locally until deliberately added. The whole emulator crate also predates
M6 as untracked work. No files were staged and no commit/push/merge/reset/PR
was performed. Curated evidence remains in ignored scratch storage; raw guest
saves require sanitization before sharing.

Recommended next milestone: M7, original-firmware PPL execution, using the
verified compilation path and identity checks as its prerequisite. Investigate
the native duplicate-declaration/lowercase-export differences separately.


## Retained evidence index

`scratch/research/m6-evidence/index.json` records copied artifact paths and
SHA-256 hashes. Key artifacts:

- `cli/HELLO.json`, `cli/BROKEN.json`: final headless verdicts and exact identities.
- `cli/HELLO.runner.json`, `cli/BROKEN.runner.json`: guest control flow, fs events and script evidence.
- `screens/HELLO-04_TAP_293_229.png`, `screens/BROKEN-04_TAP_293_229.png`: genuine compiler dialogs.
- `screens/*-08_IDLE.png`: genuine final Home screens.
- `matrix/native-comparison.json`: 25 native/firmware comparisons.
- `firmware-cases/`: 46 final headless test reports.
- `sequences/`: navigation and actual in-editor correction reports/screens.
- `validation/firmware-final.log`: 13/13 real-firmware regression results.
- `validation/commands.json`: required workspace commands and exits.
- `validation/firmware-initial-failed.log`: retained initial navigation failure.
- `baseline/m5.json`: reproduced six-program M5 baseline.
- `raw-saves/`: pre/post containers; local only, see its WARNING.txt.

The evidence directory is ignored. No proprietary firmware or factory drive
archive/tree was copied into it. Original report paths remain intact; the
index maps those paths to retained copies rather than rewriting provenance.
