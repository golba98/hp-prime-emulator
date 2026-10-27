# Developer guide

The scripts support the Rust workspace, offline protocol research, and explicit
hardware acceptance work. Established CLI paths remain stable. Python tools
require Python 3.10 or newer; they can be imported without running their CLIs,
opening displays, loading native libraries, or starting subprocesses.

## Workspace

| Crate | Purpose |
|---|---|
| `ppl-math` | Decimal arithmetic and HP numeric types |
| `ppl-syntax` | Source loading, program containers, lexer, parser, diagnostics |
| `ppl-compiler` | Semantic analysis and native bytecode generation |
| `ppl-vm` | Native execution, values, builtins, host interface |
| `ppl-graphics` | Headless graphics and PNG output |
| `ppl-engine` | Shared results and native/firmware orchestration |
| `prime-emulator` | ARM firmware execution, guest storage, compiler and runtime |
| `ppl-gui` | Native editor, diagnostics, execution and calculator display |
| `ppl-cli` | The `ppl` command |
| `ppl-device` | Optional, explicit USB calculator communication |
| `ppl-compat` | Compatibility suites and evidence reporting |
| `ppl-bench` | Benchmarks |


Ordinary tests do not require proprietary firmware. Firmware tests require local
assets and explicit configuration; a skipped test is not evidence of guest
execution. See the [firmware CLI guide](../emulator/M8_UNIFIED_CLI_REPORT.md)
for setup, result fields, and limitations.

## Layout

```text
scripts/
├── device/                  Diagnostic reports, fixture GUI checks, opt-in hardware acceptance
├── emulator/                Cargo wrapper and local factory-image extraction
├── lib/                     Shared X11 bindings for the two offline display tools
├── maintenance/             Repository tree and minimal Xvfb window manager
├── research/                Offline PE inspection, pinned traces, HID screenshot replay
├── tests/                   Offline unit/characterization tests and synthetic report snapshots
└── ruff.toml                Optional Python lint/format configuration
```

There are no separate build/protocol/screenshots subdirectories: the small
number of tools does not warrant extra nesting. Protocol assembly, metadata
inspection, PNG decoding, and framebuffer composition have separate functions
in `research/ck_screen_replay.py`.

## Entry points and dependencies

| Script | Responsibility and dependencies | Callers and output |
|---|---|---|
| `emulator/run.sh` | Bash wrapper forwarding all arguments to Cargo | README, emulator docs, fixture docs and manual development; inherits Cargo output and exit code |
| `emulator/prepare_factory_a.sh` | Locate the FAT volume in `PRIME_APP.DAT`/`app.img` and extract with `mcopy`; Bash, grep, awk, mtools | README and firmware-test documentation; writes `<out-dir>/A`, skips existing files with `mcopy -n` |
| `maintenance/show-tree.sh` | Print repository structure, excluding generated/private trees; Bash and `tree` | Manual developer utility; optional positive depth, default 4 |
| `maintenance/xvfb_miniwm.py` | Give Xvfb clients useful geometry; Python and libX11 | Legacy/manual desktop smoke workflows; display, width, height and optional duration (default 60 seconds) |
| `device/mirror_report.py` | Summarize saved capture/presentation JSONL; Python standard library | Live-screen documentation; human-readable stdout including P99 for 100+ observations |
| `device/mirror_xvfb_smoke.py` | Check fixture-rendered pixels and shortcuts on a separate Xvfb display | Live-screen documentation; requires Xvfb, ImageMagick `import`, Pillow, libX11, libXtst and built `mirror_diagnostics`; writes new screenshots, diagnostics and `results.json` |
| `device/verify_automated_deletion.py` | Opt-in physical acceptance using guarded Rust CLI commands; Python standard library and `ppl` | Hardware-testing documentation; creates fresh `ZZT_*` objects, backs up/verifies deletion, records command output and `summary.json` |
| `research/ck_inspect.py` | Read-only PE32/PE32+ sections, imports, strings, disassembly, xrefs, pointers, RTTI and Qt metadata | Trace generator, research docs and manual investigation; Capstone needed only for disassembly/xrefs; text and optional provenance JSON |
| `research/ck_trace_evidence.py` | Reproduce pinned Connectivity Kit analysis without executing vendor code | HID trace documentation; imports `ck_inspect.PE`, requires Capstone; creates a new directory containing `.asm`, `tables.txt`, and `manifest.json` |
| `research/official_archives.py` | Offline Burn/CAB/MSI extraction with bounds, fresh directories and PE provenance | `7z` required; never installs vendor code |
| `research/official_screen_trace.py` | Reproduce constructor, virtual framebuffer and Windows raster evidence | Four input hashes checked before writing disassembly and manifest |
| `research/qt555_oracle.py` | Independent conversion through the host distribution's Qt5 QImage | Linux C++ ABI and system Qt5Gui; never loads vendor DLLs |
| `research/ck_screen_replay.py` | Assemble saved HID reports, validate FC PNG envelopes, optionally compose patches in memory | Project-trace documentation and tests; standard library for inspection, Pillow for replay; JSON metadata/hashes on stdout |

`lib/x11.py` owns shared native signatures, display connections, window-tree/title
buffers, and geometry operations. The window manager and mirror smoke test own
their separate policies. `ck_inspect.PE` remains the shared binary reader for
both research commands. There is no general-purpose utility framework.

All CLI scripts were retained: a missing import/caller does not establish that a
manual utility is obsolete. The CI workflow calls Cargo directly and does not
require script-path changes. No scripts are called by Rust at runtime.

## Build, check and run

Run these from the repository root:

```bash
scripts/emulator/run.sh check --workspace --all-targets
scripts/emulator/run.sh clippy --workspace --all-targets -- -D warnings
scripts/emulator/run.sh test --workspace --all-targets
cargo fmt --all -- --check
scripts/emulator/run.sh run --bin ppl -- run examples/FIREWORKS.hpprgm
scripts/emulator/run.sh run --bin ppl -- gui examples/FIREWORKS.hpprgm
scripts/maintenance/show-tree.sh 3
```

The wrapper resolves the repository independently of the caller's working
directory, forwards each argument unchanged, and uses `exec` to preserve Cargo's
exit status. It deliberately selects `../.cache/prime-emulator-target`, matching
`.cargo/config.toml`. In this checkout that resolves to
`/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target`. Do not change
this to a new target directory or clean the cache: Unicorn's native build needs
a space-free build location.

For local proprietary firmware extraction:

```bash
scripts/emulator/prepare_factory_a.sh '/path/to/PRIME_APP.DAT' '/outside/repo/factory'
```

Firmware remains private. Extraction preserves the first matching OEM marker,
subtracts three bytes to find the boot sector, and retains non-overwrite mode.

## Offline research and verification

```bash
python3 scripts/research/ck_inspect.py /path/to/ConnectivityKit.exe --provenance
python3 scripts/research/ck_inspect.py /path/to/ConnectivityKit.exe \
  --disasm 0x5a17b7 0x5a17f7
python3 scripts/research/ck_trace_evidence.py \
  scratch/research/autonomous-control-investigation/ck/ConnectivityKit.exe \
  /tmp/prime-trace-new
python3 scripts/research/ck_screen_replay.py \
  artifacts/live-mirror/planning-baseline-20261024T213653Z/format-*.hid.log
python3 scripts/research/ck_screen_replay.py --replay \
  artifacts/live-mirror/planning-baseline-20261024T213653Z/format-0d-*.hid.log
python3 scripts/device/mirror_report.py artifacts/live-mirror/full-new
```

Trace generation checks the pinned input SHA-256 before creating output and
refuses an existing output directory. Disassembly candidates and raw pointers
are not proof of reachability. The binary is read, never loaded as executable
code. The pinned version, ranges, file naming, address conventions and manifest
fields are compatibility-sensitive.

Replay consumes logs in **argument order**. It requires one FC request and a
complete ordered response per log, validates length/CRC and discards HID padding.
A format change requires a new full-origin baseline. The 16-bit grayscale PNG
path represents little-endian packed RGB555, not ordinary grayscale. Scaling and partial
third-width coverage follow the recovered viewer. Output contains metadata and
hashes; no calculator pixels are written by replay.

The 100 retained logs contain 20 captures each of formats 08, 09, 0A, 0B and 0D.
Tests compare all metadata and the 0D composition against the saved reports in
`scratch/research/autonomous-control-investigation/continuation-20261025/`.
These private inputs are optional on other checkouts: tests explicitly skip
when unavailable. Synthetic report snapshots live under `tests/fixtures/` inside
`scripts/`; they are not hardware evidence. No captured logs or firmware are
copied into the test suite.

Run all script tests with:

```bash
python3 -m unittest discover -s scripts/tests -p 'test_*.py' -v
bash -n scripts/emulator/run.sh scripts/emulator/prepare_factory_a.sh scripts/maintenance/show-tree.sh
# Optional development tools; neither is a runtime dependency:
ruff check scripts
ruff format --check scripts
shellcheck scripts/emulator/*.sh scripts/maintenance/*.sh
```

The internal test modules moved from `scripts/research/test_ck_research.py` and
`test_mirror_report.py` to `scripts/tests/`. Replace the old research-only
`unittest discover -s scripts/research` command with the command above. Public
CLI paths did not change, so no compatibility wrappers are needed.

The GUI example must already be built for this fixture-only acceptance check:

```bash
scripts/emulator/run.sh build --release -p ppl-gui --example mirror_diagnostics
bwrap --bind / / --dev /dev --unshare-net --die-with-parent -- \
  python3 scripts/device/mirror_xvfb_smoke.py \
  --fixture tests/fixtures/prime-rgb555-le.png --out-dir /tmp/prime-xvfb-new
```

The default binary is the existing shared target's
`release/examples/mirror_diagnostics`; `--binary` overrides it. Output directories
must be new. All shortcut injection targets the isolated X server. The test
checks submitted versus displayed pixels, pause/resume, scale 1–4, snapshot,
letterboxing, and Escape shutdown. External captures and cleanup are bounded.

On hosts where NVIDIA EGL crashes under a private `/dev`, prefix the command
inside `bwrap` with
`env __EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json`
if that Mesa vendor file exists. This is a host validation workaround, not a
script default or a calculator capability.

## Physical-device boundaries

`verify_automated_deletion.py` is the only script that invokes physical device
operations itself. Its `--hardware` flag remains required. It uploads and deletes
only newly named `ZZT_*` objects through the existing Rust guards, retains raw
backup verification and independent absence queries, and stops without retry or
cleanup after failure. Agents need separate explicit authorization to run it
against hardware. Its automated tests mock **every** subprocess and discard
synthetic reports with their temporary directories.

Its historical `--ppl target/debug/ppl` default is retained for compatibility;
this checkout's executable is actually in the shared target. A separately
authorized hardware run must supply the correct `--ppl` path explicitly. Use
`--help` for options. No hardware acceptance run is part of this refactor.

The Cargo wrapper can forward device commands, including
`run --bin ppl -- device detect`; tests verify that forwarding with fake Cargo.
They do not execute the device command. Research tools never open USB.

The complete Rust suite includes device enumeration tests. Run it with hardware
hidden during automated validation:

```bash
bwrap --bind / / --dev /dev --unshare-net --die-with-parent -- \
  scripts/emulator/run.sh test --workspace --all-targets
```

Do not enable proprietary firmware asset variables for routine script checks.
Tests returning early without assets are not firmware verification. Previously
recorded cold-boot timeouts remain separate unresolved functional work. Saved
fixture replay does not authorize physical FC04 experiments or remove existing
incremental-mirroring capability gates.

## Conventions and extension

Use descriptive `snake_case.py` and `kebab-case.sh` names for new scripts;
existing documented names are compatibility exceptions. Prefer existing groups
and add a directory only for a distinct responsibility.

Separate `argparse` parsing, core work, and `main()`. Guard the entry point. Keep
human diagnostics on stderr when stdout carries JSON. Preserve successful
schemas, ordering and stdout when changing established tools; argument errors
use exit 2 and operational errors exit 1. Missing diagnostic files deliberately
mean no measurements; malformed files are errors with file/line context.

Use context managers and bounded subprocesses. Hardware timeout, packet and
authorization changes require separate functional review. Put a helper in
`lib/` only when multiple meaningful callers share the same domain behavior.
Add focused tests for protocol/output contracts and use temporary directories
for generated data. Document optional private fixtures and skips explicitly.
New documentation needs a precise `.gitignore` exception; never broadly unignore
private research assets.
