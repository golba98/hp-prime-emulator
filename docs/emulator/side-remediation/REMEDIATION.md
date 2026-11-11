# HP Prime side emulator remediation

This is an implementation record against the 33 findings captured at `8d7535e`.
The original findings and inventory are retained beside this report as
`baseline.txt` and `baseline-inventory.json`. `matrix.json` records every finding,
its root cause, implementation, tests and actual outcome.
[verification.json](verification.json) records command results and log hashes;
[the Native graph window](evidence/native_side_graph.png),
[graph LCD](evidence/native_side_graph_lcd.png) and
[Solve LCD](evidence/native_side_solve_lcd.png) preserve public GUI evidence. Partial and unresolved
rows remain open requirements; this report does not call the whole calculator
or every HP command fully implemented.

## Integrated behavior

The existing calculator body and 51-key layout are preserved. Native input now
uses one declarative action table and one editor shared by Home, CAS, mathematical
app fields and modal controls. Cursor/selection, clipboard, undo/redo, structured
fractions/powers/roots, scientific modifiers, Alpha locks, mathematical constants,
DMS/polar entry, history recall and held-key repeat have executable behavior.
Space and Shift+Plus follow their printed labels. Shift+Minus opens Edit Integer.

Home and CAS have independent histories, drafts, answers and settings. Native
numeric arithmetic remains the BCD VM. Symbolic evaluation uses a supervised,
separately installed Giac executable, typed results and a validated protocol.
Home/CAS names are shared through a typed bridge; private transport aliases never
become saved calculator variables. CAS failure/cancellation does not commit its
proposed environment. Giac compatibility is not identical to all HP CAS semantics.
Build/discovery and licensing are documented in `tools/giac-helper/README.md`.

Apps have persistent models, editable symbolic fields and asynchronous numerical
calculations. Function, Parametric, Polar, Sequence and implicit plots share their
expressions with numerical tables. Plot workers use immutable calculator memory,
settings and program snapshots; zoom/pan resample the plot. Independent variables
are scoped to each graph kind; other Home names retain their actual values.
Trace follows computed samples rather than interpolating fictional points. App Symbolic Setup
has independent overrides, including the documented PPL A-setting variables.
Non-graph apps provide real calculations for statistics, finance, linear/triangle
solving, inference, symbolic solving, geometry and spreadsheets. Finance plots
and grouped numerical schedules share its solved payment and cent rounding.
Solve uses a cloned Home/CAS revision and plots both sides of the equation;
its chosen unknown cannot be replaced by an existing binding of that name.
Inference plots use the actual probability distributions and computed boundaries. Their full HP
app-specific feature sets remain a parity limitation, not a completed claim.

Programs, notes, lists, matrices and variables use calculator-owned storage,
shared with PPL. Installed programs survive closed IDE tabs and run inside larger
Home expressions. Catalog runs keep a stable program namespace and file-local
values without opening IDE tabs. Explicit entry/argument controls use the same
worker; missing required arguments open a real calculator INPUT form. Programs/notes have creation, editing, deletion, rename and
search. App Info is a separate per-app note; Shift+Apps and STARTVIEW(6)
open that note, while Help opens the command help browser. App notes persist
independently of the standalone Notes catalog, following the
[HP User Guide, chapter 27 and STARTVIEW table](https://h10032.www1.hp.com/ctg/Manual/c05332710.pdf).
Collection editors reuse the actual INPUT/EDITLIST/EDITMAT host machinery.
The program catalog is no longer only a list of exports in open IDE tabs.

The User keyboard resolves documented KEY declarations from exact source
snapshots. Temporary/persistent modes, ambiguity handling and typed return
insertion are implemented. Handlers use the normal cancellable VM worker and
interactive host. Their return text does not execute automatically or overwrite
Home Ans. ON interrupts programs, nested calls and interactive operations at the
execution engine; it cannot be intercepted by a User binding while a program runs.

Calculator memory uses a separate, versioned Native profile. A single writer
coalesces changes, writes an atomic replacement, retains the previous generation,
syncs files/directories, and holds an exclusive profile lock. Loading validates
exact typed values and UI schema. Future schemas are refused without overwriting;
corrupted current data can recover from a preserved previous generation. Native
VM stacks are operation state, not a pretend whole-machine snapshot.

Native tool LCD screenshots capture the actual composed display, including app
controls and plotted curves. DPI, viewport, request, owner and generation checks
reject stale captures. An old GROB is never substituted for a visible tool page.

## All 33 finding outcomes

The detailed source/test matrix is [matrix.json](matrix.json). “Partial” and
“Unresolved” retain required work; they are not acceptance closures.

| ID | Finding | Actual outcome |
|---|---|---|
| 1 | Apps and app views | Partial: app-specific HP parity remains incomplete, including full Geometry constructions, spreadsheet formatting/functions, all statistics fits and app-specific plot tools. |
| 2 | CAS and CAS settings | Implemented for supported Giac operations; external helper installation required, no claim of every HP CAS command matching Giac. |
| 3 | Home/app setup routes | Implemented and covered by settings/session/keypad and integrated GUI tests |
| 4 | Help and User keys | Implemented; User worker/reducer tests and actual desktop temporary/locked workflows passed |
| 5 | Vars/Toolbox/Templates | Partial: Help/menu contents cover supported native functions; full HP catalog/help database not bundled. |
| 6 | Alpha entry | Implemented |
| 7 | All primary keys | Implemented; full declarative primary/Shift/Alpha coverage and desktop input/navigation exercised |
| 8 | Shift scientific functions | Implemented |
| 9 | Other Shift legends | Partial: implemented for supported actions; HMS/DMS and polar entry/answer toggles have parser, numeric and GUI regressions; full formatted-value metadata remains partial |
| 10 | Space and Ans | Implemented |
| 11 | Parentheses/sign/fractions | Implemented |
| 12 | Cursor/selection/undo | Implemented |
| 13 | Interactive history | Implemented |
| 14 | Clipboard | Implemented |
| 15 | Idle LCD interaction | Implemented |
| 16 | Pointer dialogs | Implemented for native dialog controls |
| 17 | ON interrupts VM | Implemented |
| 18 | Native power/persistence | Implemented for calculator memory; no suspended Native VM stack persistence promised |
| 19 | Independent Programs catalog | Implemented; 17 real egui/controller/VM management tests plus restart and linked-program desktop workflows |
| 20 | Lists/matrices/notes | Implemented; real egui list/matrix/note CRUD, transactional editors, profile reopen and desktop object restart verified |
| 21 | EDITMAT dimensions | Implemented |
| 22 | INPUT controls | Partial: implemented for tested documented control forms; other unsupported type codes remain explicit errors |
| 23 | Nine unsupported entries | Partial: app/view subset tracks Native apps; not every HP app-specific view is implemented. |
| 24 | 27 approximate entries | Partial: exact fonts/raster, opaque firmware caches, raw UTF-16 string semantics and remaining HP overloads require work; see command-by-command report |
| 25 | Status and textbook display | Partial: full textbook layout for all result types and exact HP softkey parity remain incomplete |
| 26 | Held virtual keys | Implemented; firmware continues receiving actual held transitions |
| 27 | Firmware typing | Partial: unsupported text/newlines/template contexts and unknown status glyphs still rejected atomically |
| 28 | Firmware copy-out | Unresolved: requires a verified firmware selected-text/clipboard ABI, not OCR or framebuffer content |
| 29 | Live firmware save/resume | Unresolved: complete CPU context export, native callback continuation state machines and resource reconstruction required |
| 30 | Firmware OS coverage | Partial: 663 slots remain unhandled; exact signatures/resource ABI evidence and implementations required |
| 31 | Partial OS services | Partial: priority scheduling, wake semantics, auto-power-off and extended printf formats remain incomplete |
| 32 | Virtual device environment | Unresolved: device contracts, changing virtual environment and observed OS struct layouts need implementation/ABI evidence |
| 33 | Native automation dispatch | Implemented |

## PPL remediation

| Original unsupported entry | Actual outcome |
|---|---|
| EXACT | Genuine algebra exact conversion; preserves rational/symbolic types. |
| EDITLIST | Interactive list editing, cancellation and calculator-list writeback. |
| EVAL | Native numeric values and genuine symbolic evaluation. |
| STARTAPP | GUI host acknowledges an actual supported app; unknown names return 0. Headless hosts explicitly reject absent presentation services. |
| STARTVIEW | Supported standard/setup/catalog views acknowledged by the GUI. App-specific unsupported views remain a limitation. |
| RECALL | Audit correction: the tested G1 firmware rejects `RECALL("A")` during compilation. It is not retained as a fabricated supported command. |
| AFiles | Per-app file listing, typed retrieval/assignment and persistence. |
| HVars | User variable discovery/read/write and stored function parameter metadata/calls. |
| CAS | Real Giac process service with typed values, shared names and independent CAS state. |

The 27 originally approximate entries are tracked below. Classifications remain
conservative where raster, overload or HP-specific parity has not been established.

| Command(s) | Implementation and retained limits |
|---|---|
| APPROX | Existing BCD approximation plus typed symbolic values. No blanket HP algebra parity claim. |
| STDDEV | Weighted population deviation in decimal arithmetic, vector/matrix-column forms, bounded frequencies and validation. |
| STRING | Extended number formats, fractions/mixed fractions, separators, signs, DotZero and pixel-fit forms; styled-font parity remains limited. |
| EXPR | Existing native parser/evaluator; it does not silently promise every HP expression grammar/command. |
| APPEND | Lists and numeric vectors, correct copy semantics and type validation. |
| EDITMAT | Interactive cells, dimensions, insert/delete rows/columns, vectors, validation and cancellation. Bare calculator M variables write back; ordinary local matrix expressions return an edited value, matching retained controls. |
| RECT, LINE, PIXON, PIXOFF, GETPIX | Calculator plot-coordinate integration; LINE also implements numeric 3D meshes, transforms, clipping and reusable Native caches. Exact raster boundaries remain a parity limit. |
| CIRCLE | Native compatibility extension; the tested G1 rejects this name. It is not mislabeled as a verified HP drawing command. |
| FILLCIRCLE_P, ARC | Pixel forms retained. ARC has a pixel radius even in its Cartesian form, as documented; a proposed ellipse/radius change was rejected after checking evidence. |
| TEXTOUT_P, TEXTOUT | Existing text rendering and documented pixel-width/end behavior; exact HP font/style raster parity is incomplete. |
| BLIT, SUBGROB | Pixel forms retained; Cartesian conversion, omitted-source forms, scaling and clipping corrected against retained genuine local-firmware controls. Exact resampling/raster parity remains limited. |
| DIMGROB | Dimensions and genuine packed fourth-argument pixel data, including observed 16/32-bit selection and transparency. |
| TRIANGLE_P | Scalar/vertex/mesh forms, gradient/opacity, transforms, clipping, reusable Native point caches and actual writable depth strings. Cartesian TRIANGLE shares this implementation. |
| MSGBOX | Real confirmation/cancel controls through the host; desktop pointer and keyboard actions. |
| INPUT | Real suspended execution, typed fields, checkbox/radio/choice controls, initial/reset/default values, confirmation/cancellation and writeback. Unsupported forms still error explicitly. |
| CHOOSE | Selection, keyboard/pointer confirmation and cancellation through the real host. |
| MOUSE | Correct idle index result and shared pointer state; no fabricated touch events. |
| DISP | Existing terminal/display behavior retained; exact HP font/layout parity remains limited. |
| FREEZE | Waits for actual key input and remains interruptible. |
| DRAWMENU | List/positional labels and argument validation render actual screen softkeys. |

Native 3D cache strings are explicitly versioned and backend-specific. Firmware
heap-bearing caches are rejected before drawing. Native depth writeback preserves
all bytes through a tagged representation when Rust UTF-8 strings cannot represent
isolated UTF-16 surrogates. Exact DIM/ASC/raw-index parity for those binary strings
requires a UTF-16-unit value model. See `docs/development/native-graphics-data.md`.
These limitations are not hidden by changing every builtin status to Implemented.

## Firmware remains distinct

Desktop power-expression typing and additional Unicode translate to actual guest
key sequences. Deferred thread creation, suspend/resume/termination, IRQ masking,
reenable/done, deterministic ISR error handling and common ARM debug printf forms
have concrete implementations and synthetic ARM tests. The initial audit missed
an existing ISR dispatcher; the repaired defects were its masking/error semantics.

The precise open firmware requirements are recorded with source evidence in
`docs/emulator/SIDE_FIRMWARE_CAS_REMEDIATION.md`:

- Selected firmware text has no verified clipboard/selection ABI in the current
  host interface. Pixels or OCR are not substituted for calculator text.
- Live restore lacks complete Unicorn CPU export, resumable native SVC callback
  continuations and reconstruction of host resources/waiters. Existing component
  images explicitly reject live resume. OFF recovery is preserved; it is not
  called a whole-machine snapshot.
- The registry still has 663 unhandled slots. Resource/metadata and device APIs
  require exact ABI investigation and implementation. Unused slots are coverage
  gaps, not 663 demonstrated failures.
- Priority/wake/power/device environment and full storage structures remain
  incomplete. Constant/synthetic replies are not represented as physical parity.

Local firmware execution used legitimate existing assets and disposable copied
profiles. No physical calculator was accessed. No firmware assets, vendor binaries
or opaque heap captures are included in the repository.

## Verification record

The original local results used Rust 1.96.0. The first hosted runs on PR #30
used Rust 1.99.1 and failed the Check step on both Linux and Windows: three
new graph strokes relied on implicit float fallback to f32, rejected by the
new compiler with warnings denied. The corrective change explicitly types those
widths as f32. Warnings remain denied; the workflow and test assertions were not
relaxed. Check and all-target Clippy also passed locally with Rust 1.99.1 after
that correction. Hosted results are available in the
[PR checks](https://github.com/golba98/hp-prime-emulator/pull/30/checks).

The follow-up also corrects Windows Native profile persistence: the copied
backup is reopened with write access for FlushFileBuffers, the staged file is
closed before replacement, and directory syncing uses the existing
platform-aware helper. A new regression rotates three generations, corrupts the
current image, recovers the previous image, and verifies the corrupt bytes were
preserved. File contents are synced on all supported platforms; directory fsync
is applied on Unix, with the existing OS-managed directory durability policy on
Windows. This follows the documented
[Windows file-flush access requirement](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers).

With the correction, Rust 1.99.1 GUI tests passed **533 / 0 failed / 2 existing
ignored** and all-target GUI Clippy passed with warnings denied. The preceding
CI-equivalent all-targets workspace run passed **2,219 / 0 failed / 6 existing
ignored** with the CAS helper deliberately unavailable; the new profile
regression additionally passed in the corrected GUI run. Original validation
counts below remain the historical Rust 1.96 record.


Commands below were run in the canonical checkout. All Cargo commands used
`CARGO_TARGET_DIR=/home/k9-vortex/Development/4-Rust/.cache/prime-side-remediation-target`.
CAS commands used the genuine separately installed Giac helper. Counts from
separate commands overlap and must not be added as unique tests.

| Exact command / workflow | Result | Preserved log/evidence |
|---|---|---|
| `cargo check --workspace` | Passed | `/tmp/side-final-check.log` |
| `cargo fmt --all -- --check` and `git diff --check` | Passed | `/tmp/side-final-fmt.log` |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed | `/tmp/side-final-clippy.log` |
| `cargo clippy -p ppl-cas -p ppl-vm -p ppl-gui --all-targets --features giac-integration -- -D warnings` | Passed | `/tmp/side-final-giac-clippy.log` |
| `cargo test --workspace` | **2,219 passed, 0 failed, 6 existing ignored** across 124 result suites | `/tmp/side-final-workspace-tests.log` |
| GUI library within that workspace run | **532 passed, 0 failed, 2 existing ignored**; original baseline was 322 passed / 2 ignored | Same workspace log |
| `cargo test -p ppl-gui --lib --features giac-integration` | **552 passed, 0 failed, 2 existing ignored** | `/tmp/side-gui-integration-final.log` |
| `cargo test -p ppl-cas -p ppl-vm --features giac-integration` | **256 passed, 0 failed, 0 ignored** | `/tmp/side-final-giac-tests.log` |
| `PPL_BIN=<debug/ppl> PPL_CAS_HELPER=<installed helper> bash tests/ide_xvfb_smoke/run_native_side.sh` | **11 real desktop sessions; 21 input + 77 workflow assertions passed** | `/tmp/side-native-acceptance-final.log`; `/tmp/prime-native-side-K05Ieh`; `/tmp/ppl-ide-e2e/native_side_*.json` and PNGs |
| Native input script through `run_ide_auto.sh` with `WINIT_X11_SCALE_FACTOR=2 PPL_XVFB_SCREEN=2560x1800x24`, then `assert_native_side_input.py` | **21 assertions passed at scale 2** | `/tmp/side-final-input-hidpi.log`; `/tmp/side-final-input-hidpi-assertions.log` |
| `PPL_BIN=<debug/ppl> python3 tests/ide_xvfb_smoke/assert_native_side_crash_restart.py` | **8 assertions passed** after SIGKILL and a separate GUI process | `/tmp/side-crash-restart-final.log`; `/tmp/prime-native-crash-6m3x3qya` |
| `PPL_BIN=<debug/ppl> bash tests/ide_xvfb_smoke/run_m17_native_linkage.sh` | **15 existing linked IDE assertions passed** | `/tmp/side-final-m17.log`; `/tmp/side-final-m17-complete` |
| `PPL_BIN=<debug/ppl> bash tests/ide_xvfb_smoke/run_m18_native_debug.sh` | **40 existing debugger GUI assertions passed** | `/tmp/side-final-m18.log`; `/tmp/side-final-m18-complete` |
| Existing `script_compile_gate.json` through `run_ide_auto.sh`, then `assert_compile_gate.py` | **6 assertions passed** | `/tmp/side-final-compile.log`; `/tmp/ppl-ide-e2e/dump_compile_gate.json` |
| Existing editor and HiDPI scripts through `run_ide_auto.sh`, then `assert_editor.py dump_editor.json dump_editor_hidpi.json` | **37 assertions passed**, including typing, diagnostics, breakpoints, resize/cursor retention and scale 2 | `/tmp/side-final-editor-assertions.log`; both dumps and screenshots in `/tmp/ppl-ide-e2e` |
| Genuine local firmware `cargo test -p prime-emulator --test firmware_session -- --test-threads=1 --nocapture` with the three `PRIME_TEST_*` asset variables below | **2 passed, 0 failed, 0 ignored**, 93.04 seconds | `/tmp/side-firmware-live-final.log` |
| `scripts/emulator/verify_desktop_typing.py` local firmware comparisons | **3 expressions; zero differing pixels** in LCD rows 20–239 | `/tmp/prime-side-firmware-typing-evidence-2/report.json` and six PNGs |

For the genuine firmware session run, the asset variables were:

```sh
PRIME_TEST_FIRMWARE=/home/k9-vortex/.local/share/hp-prime-emulator/firmware/armfir.elf
PRIME_TEST_INIT_DATA=/home/k9-vortex/.local/share/hp-prime-emulator/firmware/initial-data
PRIME_TEST_OS_ROM=/home/k9-vortex/.local/share/hp-prime-emulator/firmware/PRIME_OS.ROM
```

Native desktop tests set `PPL_IDE_NO_DEVICE=1`, use isolated Native profiles,
config and application-data directories, and drive the production event loop
under Xvfb. Mathematical entry is clicked and typed; INPUT/EDITMAT suspend actual
programs; clipboard checks use the desktop clipboard; graph points, tables,
trace and zoom are asserted; installed programs call shared Home/CAS values.
The restart test kills only its own isolated GUI after observing durable writes.

Both existing ignored GUI tests were investigated and explicitly exercised:

- `PPL_MIRROR_FIXTURE=<absolute artifacts/current_screen.raw.png> cargo test -p ppl-gui --lib private_hardware_fixture_matches_the_standalone_decoder -- --ignored`:
  **1 passed**, final source, `/tmp/side-final-private-fixture.log`. It reads
  preserved evidence and does not connect to hardware. It remains opt-in because
  the fixture is private. A first invocation used a relative path and failed to
  locate it from Cargo's crate directory; the absolute-path run passed.
- `cargo test --release -p ppl-gui measure_analysis_latency -- --ignored --nocapture`:
  **1 passed** at the analysis handoff, `/tmp/prime-side-analysis-latency-release.log`.
  Recorded analysis best times: 277 lines 0.20 ms; 4,504 lines 2.84 ms;
  18,004 lines 12.47 ms; edit-to-diagnostics 258 ms with a 250 ms debounce.
  This is a manual measurement and remains opt-in. These handoff timings are
  not presented as a benchmark of the final whole application.

The other four existing workspace ignores belong to the optional standalone GUI,
display timing and two private firmware-file compatibility tests. No new ignore
was introduced. Genuine firmware lifecycle coverage above ran with actual assets.

No tests were deleted or permanently skipped to make results pass. Tests whose
previous unsupported assumptions became obsolete now assert actual implemented
semantics and retain separate unsupported-command coverage. Import classification
now inspects compiled dependencies, rather than incorrectly searching comments
and strings or reporting newly supported app services as unsupported. The older
editor scripts assumed an extra initial Untitled tab and inherited layout; they
now select the actual two tabs and explicitly set Native/Split. Their diagnostic
assertion requires the exact remaining warnings; obsolete INPUT-default and
symbolic-unsupported warnings would fail it. Actual INPUT/CAS behavior is tested
separately rather than replacing functional assertions with render checks.

## Remaining required work

The matrix contains **20 implemented, 10 partial and 3 unresolved findings**.
These are engineering outcomes, not a declaration that all 33 requirements closed.
In addition to the precise firmware blockers above:

| Remaining Native limitation | Code cause and work required |
|---|---|
| Complete app parity | `native_app_models.rs` contains defined finite models (for example linear regression and point/segment Geometry); `native_app_views.rs` exposes their supported views. Full Geometry constructions, additional statistical fits, spreadsheet functions/formatting and HP app-specific tools still need models, controls and real behavior tests. They are not external blockers. |
| Every HP app special view | `app_tools.rs` acknowledges supported standard/setup/catalog views; unimplemented special views return false. Split Plot Detail/Table and corresponding PPL view-state synchronization require implementations and GUI/runtime tests. Native decimal/integer/trig presets and actual trace/autoscale exist independently. |
| Complete Help/catalog database | `views.rs` uses the real supported builtin registry and links the HP guide. It does not embed the complete HP help/catalog database or claim unsupported entries work. |
| Full textbook/result metadata | `native/math_layout.rs` implements editable input structures; many result types still use textual presentation. DMS/polar answer toggles work, but arbitrary formatted-value propagation and exact HP softkey/rich-note formatting remain incomplete. |
| Exact graphics/raw-string parity | Native 3D caches have a declared portable schema. HP heap-address caches are opaque. `Value::Str(Rc<str>)` cannot represent isolated UTF-16 units, requiring a coordinated value/string/persistence redesign for exact binary DIM/ASC/indexing semantics. Fonts and some raster/resampling boundaries still differ. |
| Complete INPUT/overload surface | Tested controls and default forms execute through the actual suspended VM; unsupported type codes/argument forms still error. The compiler deliberately retains conservative Approximate classifications where documented overload/font/backend parity remains incomplete. |

The missing Native features above remain required work. Firmware selected-text
copy-out, live whole-machine restore and complete virtual device contracts remain
unresolved; they have no successful acceptance result. The application is not
represented as a fully functional HP Prime replacement.

## Source organization

| Area | Main changed files |
|---|---|
| Native UI and dispatch | `app_native.rs`, `app_tools.rs`, `ide/native.rs`, `ide/native/keys.rs`, `keymap.rs`, `ide/keypad.rs` |
| Editing/layout | `ide/native/expression.rs`, `math_layout.rs`, `decimal_display.rs`, `sexagesimal.rs`, `polar_display.rs`, `app_math_entry.rs` |
| Apps | `native_apps.rs`, `native_graph_context.rs`, `native_equations.rs`, `native_app_models.rs`, `native_app_ui.rs`, `native_inference*.rs`, `native_model_plots.rs`, `native_plot_controls.rs`, `native_plot_view.rs`, `native_integer.rs`, `native_tools.rs` and its `views.rs` |
| Catalog/User execution | `app_catalog.rs`, `native_user_keys.rs`, `app_user_keys.rs`, execution controller/worker, VM linker/session |
| Persistence | `native_profile.rs`, execution worker, `ppl-vm/src/session/memory.rs` |
| Algebra | new `ppl-cas` crate, compiler `symbolic.rs`, VM `cas.rs`, separate `tools/giac-helper` |
| Dialogs/PPL | GUI dialog/list/matrix/host; VM builtins; compiler builtin/bytecode/compile layers |
| Firmware | session typing, thread/IRQ/OS handlers, runner and engine |
| Verification | VM/compiler/GUI/firmware regressions, sanitized controls, desktop scripts/assertions |

Implementation and validation completed before publication. The user subsequently
authorized a checkpoint commit, branch push and preservation PR. The PR targets
`feat/g1-usability-overhaul`, which supplies the existing milestone dependencies.
The local results above do not assert hosted CI success.
