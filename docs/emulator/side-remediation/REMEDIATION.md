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

Empty function templates now delete in one Backspace action, including typed
lowercase calls and recalled incomplete function headers. Nested template deletion
preserves entered operands; undo restores the complete prior document and caret.
Selections, literal strings and ordinary digit deletion retain text semantics.
This Native editing rule follows the requested one-action behavior; the installed
G1 line-mode reference removes the opening parenthesis separately.

Characters now has the 46 internal page names observed in the installed G1 menu,
a selectable Unicode grid, code-point details, page navigation and confirmation.
Page-menu and grid navigation work with desktop and calculator arrows; Enter
inserts once. Selection survives a separate GUI-process reopen. A character such
as π participates in normal calculator token insertion, so selecting it after 2
executes as 2*π. Native font coverage remains limited: unavailable glyphs display
their code point while inserting the original scalar. This does not close the
remaining command/help catalog or exact-font parity requirements.

Home and CAS have independent histories, drafts, answers and settings. New Native profiles use the15751 exact real model, with12-digit display
precision; older profiles retain the independently supported15157 BCD model. Symbolic evaluation uses a supervised,
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

## Verified continuation: applications and view execution

The continuation starts from `2c80c4c`. **Acceptance remains incomplete:
30 implemented, 3 partial and 0 unresolved findings.** Finding 28 is closed
by genuine selected-text extraction and desktop clipboard validation below. The remaining Native features are engineering
work, not external blockers.

- Statistics now has ten VM-owned C/D columns, five independent analyses and
  thirteen fits, including logistic, trigonometric, median-median and a bounded
  user-defined PPL curve. Weighted summaries, residuals, Make expressions and
  selected fits share actual column data. Full-screen plots provide computed
  curves, zoom, trace, sampled Go To, fit visibility and equation inspection.
  Actual cell expression entry, all fit choices, five analyses and separate-process
  restoration pass. One-variable plot coverage remains Histogram; frequencies
  are nonnegative integers, Make fills a list, and the finite analysis range and
  sampled Go To remain limitations.
- Geometry now evaluates typed constructions, dependencies, intersections, point
  transformations and measurements against the actual app/Home context. Actual
  Cmds actions, point dragging, colors, rename/reference rewriting, atomic invalid
  edits and separate-process restoration pass. Computation uses finite f64
  geometry; curve/tangent/trace/point-on tools, exact CAS geometry and the PPL
  object bridge remain open. Spreadsheet is still open.
- Function Plot Detail keeps a reference window and computed samples on the
  left while the right plot zooms or pans. Replace reference and Unsplit perform
  actual state changes. Plot Table presents the shared numeric table beside the
  plot, with selectable rows and independent asynchronous calculations.
- Function `STARTVIEW(8..13)` implements those two split views, Autoscale,
  Decimal, Integer and Trig, following the
  [HP User Guide's Function view ordering](https://h10032.www1.hp.com/ctg/Manual/c05332710.pdf).
  Other apps' unimplemented special-view ordering remains explicitly rejected.
- A presentation request now carries an immutable snapshot of the running VM,
  including assignments that have not committed to the persistent session.
  Changed plot bounds return to that VM. Autoscale waits asynchronously before
  replying, so following PPL statements read the resulting bounds.
- Automation `wait_idle` now waits for graph, split-reference, table and pending
  model calculations. A new desktop program checks immediately returned plot
  data without inserting a settling delay.

Numerical regressions recover independently specified curves and a hand-computed
noisy least-squares result, reject invalid domains/rank-deficient data, and check
profile compatibility. Production egui tests click the fit selector, Calculate,
split-view controls and table rows; they verify computed worker results, plot
samples, pane layout, immutable references and VM-visible bounds. The desktop
special-view scenario executes real PPL programs and inspects their results.
Screenshot inspection found and corrected an inherited horizontal table layout;
regressions now require vertically arranged rows inside the right LCD pane.

The actual desktop [Plot Detail](evidence/native_function_plot_detail.png) and
[Plot Table](evidence/native_function_plot_table.png) captures show both panes
inside the existing calculator interface.

Home/CAS history now selects expressions and results independently while
preserving the draft. Copy/Enter insert at the existing cursor. Deleting a
selected history half preserves its partner, draft and Ans; the deletion survives
reopening. Lists/matrices render structurally, and Show scrolls oversized results
without evaluating or changing Ans. Per-scalar display metadata preserves history
precision. Rational/power layout, nth-root degree entry and fraction vertical
navigation have real GUI tests and installed-G1 controls. Legacy collection
formatting, other templates/fonts and complete textbook/softkey parity remain25;
these improvements do not close that finding.

All six Native tools now have complete action acceptance: Vars/Memory inspect
and manage real independent Home/CAS bindings; the command picker searches the
real compiler registry and opens selected-command Help; eighteen templates insert
editable structures; Characters inserts actual scalars; Units/prefixes validate
through the real dimensional translator and Giac. Desktop controls store19,
insert/evaluate it, calculate20cm+5in=32.7cm and5m=16.4041994751ft, delete only the
chosen scope, and retain state after a separate-process reopen. This closes5;
complete proprietary Help contents and exact HP menu appearance are not claimed.

The accepted tools/text/OS checkpoint passed2,435 default workspace tests and
2,493 with genuine Giac (both0failed,6existing ignores), warnings-denied Clippy,
formatting, all28 Native desktop scenarios, crash/restart16, linkage15 and
debugger40. Actual firmware GUI clipboard/environment/live Save/Resume and the
new literal-text desktop scenario pass. Source changes after that checkpoint
remain subject to their own regression results in verification.json.

All 33 matrix rows record their previous status, current status, implementation,
source files, tests and limitations. `previous_status` identifies the continuation
baseline; `status` is the current outcome. `continuation_validation` distinguishes
current regression evidence from historical results. Passing regression coverage
for an incomplete feature does not close its missing acceptance cases.

### Continuation validation

| Command/workflow | Actual result |
|---|---|
| `cargo test --workspace` with helper override unset | 2,396 passed; 0 failed; 6 existing ignores at the integrated live-machine checkpoint. Isolated asset configuration; baseline: 2,220 passed. |
| `cargo test --workspace --features giac-integration` with matched rebuilt genuine helper | 2,454 passed; 0 failed; 6 existing ignores at the integrated live-machine checkpoint. Isolated asset configuration; counts overlap the default workspace. |
| `cargo clippy --workspace --all-targets --features giac-integration -- -D warnings` | Passed. |
| `cargo fmt --all -- --check`; `git diff --check` | Passed. |
| `run_native_side.sh` | 21 actual desktop sessions:21 input,90 workflow,19 editing/reopen and32 format/reopen assertions, plus history/deletion/Show/textbook and PPL/split-view scenarios passed. |
| New special-view scenario at scale 2, 2560×1800 Xvfb | Passed value/state assertions; both split panes visually inspected. |
| `assert_native_side_crash_restart.py` | Existing8 assertions plus5 format and3 typed unit/CAS vector checks passed after actual SIGKILL and reopening. |
| Existing M17 linkage / M18 debugger desktop scenarios | 15 / 40 assertions passed. |
| Genuine local `firmware_session` tests with all three `PRIME_TEST_*` variables | 2 passed, 0 failed, 0 ignored; actual source/target start notifications additionally validated. |
| Genuine `firmware_storage` | 1 passed: real saved program/settings and A/L1 survived complete-tree restoration and a fresh firmware boot. |
| Strengthened genuine `firmware_inspection` | 1 passed: all owners paused, memory acquired, zero parked callbacks; live PPL resumed after inspection. |
| Firmware clipboard helper / actual GUI OS clipboard | 5 exact clipboard cases / 7 assertions passed. |
| Genuine desktop typing vs physical keys | 3 expressions; both caret phases match with zero differing pixels in rows 20–239. |

Commands ran against an isolated continuation checkout with the physical device
link disabled. The [verification record](verification.json) retains separate
historical and continuation results with log hashes. Proprietary assets and
private execution logs are not included. No physical hardware verification is claimed. Genuine local-firmware live
resume and selected-text extraction are verified below.

## Verified audited PPL entries and installed fonts

Finding23 is now implemented. In addition to EDITLIST, AFiles/HVars and real app
presentation, CAS arguments retain the distinction between literal source and
runtime data. Scalar EXACT strings parse as exact-conversion source; strings in
collections remain data. EVAL preserves strings. Declared Native calls embedded
in quoted algebra execute once and contribute typed values, including quoted
strings, without scanning inserted values as source templates. Quoted assignment
syntax returns the observed error value without changing the Native local.
Undefined and signed/unsigned infinity values are distinct from parser failures;
failed parsing cannot commit a proposed CAS environment. Protocol4 retains older
client reply shapes. Giac spelling and broader proprietary algebra differences
remain explicit under24.

Thirty-three independent installed-G1 controls now execute through the actual
IDE compiler, Run action and production worker. Actual calculator keyboard CAS
entry and separate-process restoration pass. Per-app STARTVIEW menus execute26
app-specific actions alongside the Function views and live VM-bound calculations.
RECALL remains an inventory correction: the original firmware rejects that name.

The accepted checkpoint passed2463 default and2524 genuine-Giac workspace tests
(0failed,6existing ignores), warnings-denied Clippy, build/format/diff checks,
36 actual desktop sessions,16 forced-restart assertions,15 linked IDE assertions
and40 debugger assertions. Installed-font PPL and12 CLI comparisons additionally
match three complete independent G1 framebuffers. These narrower proofs do not
close the remaining application, graphics-overload or display findings.

## Verified settings, expression catalogs and word values

The accepted Geometry/word-editor/firmware-readiness checkpoint passes2,555 default
workspace tests and2,616 with genuine Giac, with0failures and6existing ignores. All-target
warnings-denied Clippy, build and formatting pass. This includes the Statistics
handoff, Geometry, same-machine EXPR catalog execution and the word/UTF16 changes below.
It does not close findings1,24or25.

Colored LINE samples now match six original-firmware programs, including complete
small framebuffers, every horizontal/vertical/diagonal sample, reversed endpoints,
offscreen clipping and coincident endpoints. The renderer preserves constant
channels and follows the measured interior color boundaries. Numeric point rows
follow genuine real/word conversion rules; an invented colored-complex-row
interpretation has been removed. Other raster boundaries remain under24.

Home settings implement all six number formats, eleven grouping choices,
complex display codes and the selected system font. History retains typed values
and canonical recall separately from formatted paint. Independent installed-G1
font regions validate the selected history/header roles. A desktop regression
exposed a UI route that accepted Signed with64bits; the firmware rejects it.
The integrated fix rejects that combination before mutation and preserves a
valid63-bit signed setting. The combined48-session desktop rerun and all24 assertion invocations pass,
including Geometry, intrinsic word entry, selected-history recall, exact saved
word metadata and installed-font settings/reopen checks. Actual SIGKILL and reopening also retain signed words; resumed PPL returns
−5 and formats the saved255word in binary after a Base change. Existing linked
IDE15assertions and debugger40assertions pass.

Words now carry signed width, mathematical sign and system-base affinity.
Signed widths1through63 reserve an additional sign bit; unsigned widths allow64.
Changing display settings cannot reinterpret a stored word. Word arithmetic
wraps with the left operand's metadata; mixing a real promotes the result to a
real. R→B rounds halves away from zero and saturates; SETBITS wraps. Logical word
operators are bitwise and obey the measured short-circuit rules. Program literals
retain32-bit defaults, while live Home/EXPR roots use current settings. Numeric
matrix/vector cells normalize word values to reals.

Thirty-one independent original-firmware STRING controls match, including full
widths, conversion limits, dynamic bases and side effects. Typed arguments and
memory restoration preserve the same metadata. Eight actual stored firmware
programs additionally verify signed-positive255 versus signed-negative values,
system-base returns and all64bits through the production decoder. These are
local-firmware observations, not physical-calculator verification.

STRING composes numeric, complex and collection results as UTF16 units. Custom
symbols use the first three units; isolated units are retained, and an embedded
NUL terminates the numeric component while collection composition continues.
Grouping patterns retain units and follow the measured terminator/admission
rules. The selected font measures each16-bit index, including its missing glyph,
without replacing the returned units. Original ASC/DIM controls and all sixteen
font-fitting thresholds match. Exact rational/pi provenance and lossless GUI
history now execute; wider formatting compatibility remains under24.

Fresh-factory firmware display tests now run with unhandled services set to stop
execution. All four pass, including actual2+2and2+3pixels and input-pump delivery
bound to the observed firmware thread. The earlier timeout is traced to an
animated welcome screen repeatedly resetting the script's unchanged-pixel gate.
Script boot input now uses the production readiness evidence: two firmware pump
entries, two timer deliveries and a subsequent main event wait with empty input.
Changing pixels cannot defeat that evidence; static pixels alone cannot provide
it. Serialized readiness progress survives restoration, and legacy checkpoints
require fresh evidence. Two subsequent four-test strict factory runs pass.

All sixteen selected firmware integration tests passed with actual installed
assets, including active/timed PPL resumption in a replacement process, literal
text across process kill, complete storage restoration, inspection, virtual USB,
UTF16 returns and signed words. An interrupted batch ended with exit143 after
nine passing tests; the seven remaining tests passed in a separate invocation.
All22 service tests also pass. The unsupported-call report now uses the same
handler lookup as dispatch, recognizing the implemented private USB receive
adapter while still reporting unknown services. Actual legacy15157 execution
uses Stop policy and reports no unsupported calls.

Scalar ARC now uses an outer integer boundary and quantized fixed-point endpoint
rays instead of dense angle samples. Sixty-six independently returned frames
cover full/partial arcs, radii0–12, fractional/negative radii, wrapping, degenerate
endpoints and BLIT scaling/clipping/transparency/opacity. Both focused regression
tests pass, including bounded work at extreme coordinates. Elliptical radii,
edge/fill colors and quantized filled sectors now execute too. An additional189
complete original-firmware frames cover small and large ellipses, narrow/wrapped
sectors, transparent colors, negative fractional radii and clipped scalar lines.
LINE reconstructs the original accumulator after clipping x and then y, rather
than rounding a continuous line. Work remains bounded at extreme coordinates;
4,096 endpoint-reversal cases also pass. Circle and ellipse transparency retain
the original repeated edge calls, including distinct center-row fill behavior.
The actual IDE Run path passes complete7×7 original sector/circle/line pixel
comparisons in the composed320×240 LCD. The original firmware leaves some thin
ellipse end-cap slots uninitialized; independent executions show allocation-
dependent stray pixels. Native initializes these slots, retains the observed
variation in the fixture and does not claim to reproduce unknown heap contents.
The tested firmware also rejects the Native FILLCIRCLE_P extension name; its
behavior is not presented as an HP command. These changes do not close24's
remaining exact-number and broader overload requirements.

Qualified Spreadsheet access now uses the production GuiHost request queue and
the authoritative worksheet. Home, PPL, dynamic EXPR and qualified CAS reads
receive typed values from a supplied live frame. Writes commit only while the
request epoch, cancellation state and worksheet source still match. INPUT and
CHOOSE cancel preserve formulas; confirmed writes freeze typed values. Formula
attribute1 returns symbolic data, matching the original type8 result. Signed
words, full-width integers, tiny BCD values, definitions and formats survive a
separate GUI process. The integrated54-session desktop suite and27 assertion
invocations pass across two runs after an external interruption, including the
worksheet bridge. The G1 caches an observed worksheet value across a running
PPL frame; Native recalculates against its supplied live snapshot. That timing
difference remains explicit. Other apps' qualified namespaces, remaining sheet
attributes/functions and full app tools remain required work.

The Application Library now has a working four-column calculator-sized gallery,
with independent highlight, actual app start, keyboard/prefix/touch navigation,
three sorting modes and persistent recency. Its17 entries use the supported app
implementations. Authored icons approximate the original artwork; omitted apps,
custom-app management and Reset are not represented as working tools. The
reviewed ordered-input fix retains Arrow/Enter/Text order within a single frame.

Legacy G1 build15157 STRING fractions now use exact integer search coefficients,
a12-digit BCD quotient comparison, format-specific tolerances and intermediate
continued fractions. Floating's scientific fallback re-enters numeric conversion
with the precision derived from its rounded coefficient. All832 original results
pass, including independent holdouts, signs, near-integer values, precision0/12,
and scalar fallbacks. Fresh isolated single-call programs independently confirm
surprising outputs rather than attributing them to transport corruption.

The existing15751 component style and the legacy15157 conversion are distinct
`Settings::string_format_version` values. Native CLI `--string-format-version 15157` selects the legacy algorithm in both execution routes; the default retains
the existing15751 style. The selected version is saved/restored; malformed values
are rejected atomically and older memories retain their previous default. Both
explicit version flags are rejected by firmware execution. This selection changes
STRING formatting, not the BCD arithmetic model. Newer exact-number provenance
and complete15751 rational semantics remain required work, so24 stays partial.

## Verified nested CAS values and application settings

The combined Function Symbolic, runtime app dispatch, app settings, CAS projection,
ellipse/LINE and STRING checkpoint passes2,592 default tests and2,658 with the
rebuilt genuine Giac helper, with0 failures and6 existing ignores. The later
rectangle checkpoint passes2,594/2,660 tests. All-target
feature Clippy, builds and formatting pass. The preceding60-process Native
desktop checkpoint passes; four additional actual Function Symbolic/reopen
sessions with system and installed Prime fonts also pass. The corrected combined64-session optimized rerun and all30 assertion
invocations pass. The original curve, plot/table, trace and pixel checks remain.

APPROX now visits actual list/matrix cells. The Home/PPL CAS boundary converts
numeric cells in mixed symbolic arrays without executing the original source
again or replacing the exact CAS answer. Original15751 controls cover eleven
programs, including mixed row types, DIM, strings, units, quotes and an assignment
counter that increments once. Giac prints some symbolic factors in a different
order; those expressions are checked algebraically, not reported as identical
text. This requires a rebuilt protocol5 helper; older helpers fail explicitly.
Ordinary CAS evaluation and saved-value parsing retain protocol4.

Original15751 controls also establish AFormat0–6 and ADigits0–11, extending the
older guide's narrower list. Floating and Rounded now affect app workers;
Home settings remain independent. Every format, boundary digit, profile
roundtrip and invalid-range atomic rejection is tested. Headless tests retain
the original source and explicitly omit only STARTAPP because the headless VM
already has Function active and does not provide application windows.

Function Symbolic now has measured rows and real inline editing, Show/Eval,
function insertion, colors and true tenth-slot labels F0/X0/Y0/R0. Parsed
Function dependencies execute in graph/table workers, including references to
unchecked definitions. Native Home/PPL app-function transport is implemented,
but its production host integration is still open: a qualified Function call
must retain the caller app's angle and resolve nested names in that caller.
Genuine Spreadsheet name-attribute controls prove this differs from expanding
all function definitions under their owner app. Findings1,24 and25 stay partial.

Calculator RECT now blends each border or interior pixel once, rather than
painting fill beneath the edge and repeatedly blending corners. Thirty-two
complete original15751 frames cover swapped/degenerate/clipped coordinates,
full-GROB forms and transparency; visible work is bounded even across fulli64
coordinates. Actual IDE Run returns the original edge/interior values and the
checked LCD pixels match. The optimized binaries and adjacent protocol5 helper
also pass ten original CAS/APPROX programs through actual IDE Run.

## Verified value formatting and editing

Finding9 now preserves hexagesimal real and rectangular/hexagesimal/polar complex
presentation as value metadata. Arithmetic follows observed propagation rules;
explicit DMS angles use degrees in trigonometric calls. Minutes/seconds are
padded, seconds round to five fractional places and carry correctly. The real
number stays decimal internally. Complex magnitude now rounds its scaled norm
once, correcting the final digit lost by separately rounded squared components.

`→HMS` and `HMS→` now tokenize and set/clear presentation rather than performing
incorrect packed H.MMSS arithmetic. Lists and scalar coercion work; the matrix
and nonreal rejection cases match the installed G1. Native accepts ∠ as an alias;
genuine G1 PPL uses ∡.79 reference results in
`tests/fixtures/native-formatted-controls.json` match without numeric tolerance,
after explicitly mapping typographic minus and the private imaginary glyph.
This is not an exact glyph/font claim; finding24 remains open.

Variables, collections, typed arguments, PPL parameters/returns and Ans retain
metadata. Memory version2 loads version1 and refuses unknown presentation values.
History stores an explicit display selection, including migration of older
fraction/DMS/polar rows. Actual desktop toggles, arrow-named commands, a separate
process reopen and SIGKILL restoration passed. A desktop test also found and
fixed explicit `(` incorrectly closing an outer call in `STRING(L1(2))`.
Nested input and atomic conversion-name deletion now have regressions.

Complete storage images now include all guest-drive files/directories, timestamps,
permissions and hard links, with bounded checksummed data and atomic publication.
Cancellation or a killed capture preserves the previous archive. Production VFS
restoration uses this image. Genuine firmware saved65 regular files; a restored
storage boot retained its program/settings and returned A=123/L1={7,8,9}.
This validates storage rehydration, not live execution resumption.

The direct compiler no longer retains a boxed call-plan trait. Typed request,
source, phase, timing and return-frame images bind a new result channel. Guest
calls save the full interrupted banked/FPU/CP15 context. Component tests exercise
all call phases and fresh-process actual ARM return; their controlled callee
values are not firmware compiler verdicts. Genuine compiler lifecycle/import
and inspection separately pass. Kernel, managed-job and compiler/runtime observer images now reconstruct
independently. Complete session producer/owner/transport integration and
whole-machine orchestration and GUI Save/Resume now execute, as verified below. A shared pausable guest clock
now drives production deadlines; its independent restoration is verified below.

## Verified INPUT and live expression execution

Finding22 now executes the installed G1 control/default contract. Edit, checkbox,
radio and choice controls preserve confirmation/cancellation writeback. Integral
filters -1 through9 include5 and7; fractional and14.x descriptors are rejected by
the actual G1. Later documentation's14.x codes are not silently enabled for this
build. Full, short, empty and scalar multifield defaults work; omitted fields keep
their original values. Excess defaults fail before opening the form.

Untouched fields retain their original typed objects. Edited text is validated
even when it equals the initial text. Rejected early fields keep later edits;
already accepted expressions are not evaluated again on unchanged retries.
Host interruption and execution limits propagate instead of reopening a dialog.
The catalog parameter and internal list/matrix dialogs retain their text contract.

INPUT and EXPR now use the running VM's lexical locals, file/Home values, calls,
assignments, settings, RNG, graphics, limits and host. Nested evaluation retains
its caller's scope; block locals expire correctly. Temporary compiled code is
removed on normal/error return. EXPR also links referenced installed and open-editor exports into the running
machine, retains independent file-local initializers and persists imported module
state. Nested expression code cannot shift active frames or reset another
module's state. Broader HP grammar remains compatibility work under finding24.

Genuine G1 controls establish the Home types of CAS integers/decimals, complex
values, ordinary numeric vectors/matrices, symbolic collections, CAS objects and
units. The adapter now preserves those distinctions. CAS arguments retain exact
expression syntax: `CAS(1/2)` returns the rational object. Explicit `CAS.x` access
and live locals in CAS text work. CAS source/data and assignment-argument error values now execute as verified below.

The matched Giac helper negotiates protocol4 for typed undefined/infinity values
and structural unit/vector metadata. It still serves old1/2/3 clients with their original reply shapes. Stale helpers
fail with rebuild instructions; the adapter prefers a matching adjacent helper.
See [helper build/protocol instructions](../../../tools/giac-helper/README.md).
The installed legacy helper was preserved during validation.

Actual desktop programs returned21 from `A+n+items(2)`, retained local assignment,
accepted untouched defaults, rejected edited invalid types, applied short defaults,
and edited unit/CAS objects. The16-session scenario now passes90 workflow
assertions alongside the existing input/editing/format/special-view checks.
A real SIGKILL and fresh process preserved the new typed unit/vector state and all
previous crash assertions.

The full suite also reproduced the old intermittent STARTVIEW failure as a stale
request: retiring the launch key changed the input epoch after the worker queued
a valid view. View ownership now uses a separate execution epoch, captured by the
creating host. A deterministic regression checks that exact interleaving and
refuses retired hosts/new-run requests. This repairs that failure; other apps'
special-view requirements remain open.

## Verified OS service execution and guest timing

Finding31 now meets its functional HLE service criteria. The legitimately installed
OS ROM contains the real service table and implementations. Three isolated ARM
controls execute its thread/wake, priority and auto-power code. The tests supply
kernel data and intercept only the scheduler context-switch boundary; they do
not establish hardware or whole-OS boot parity.

Wake cancels timed sleep, including its65535 sentinel, and preserves suspension.
Suspend sets one flag; Resume clears it and fails when already running. Accepted
wakes survive serialization. Thread creation uses the observed descriptor size,
minimum stack, deferred flag and free priority allocation. Unique priority slots
now determine the order of actual queued ARM owners. Engines serialize their
slices over shared guest memory; this is cooperative HLE scheduling.

Auto-power modes0–3 return the previous enable state, preserve the OS interval
conversion/wrap rules and consult the same system-mode variable. Input resets
the timer. Automatic sleep retains each CPU's instruction and ON resumes it;
normal firmware OFF/recovery remains a separate workflow. The shared monotonic
clock freezes timed services and UI/IRQ timing during power and bounded inspection
pauses. A fresh-process test reconstructs a timed ARM continuation after host
downtime and verifies its real delayed return. Whole-machine save/resume remains29.

Debug printf now supports aligned64-bit and floating values, integer lengths,
width/precision and their dynamic arguments, strings/characters, pointers and
explicit guest count writes. Parsing and output are bounded. No guest format or
guest pointer is forwarded to host libc. Unknown formats are reported explicitly.

At the finding31 checkpoint, validation passed2333 default and2391 genuine-Giac workspace tests,
feature Clippy, three OS ROM controls, four genuine firmware workflows,16 Native
desktop sessions and16 SIGKILL/reopen checks. Workspace tests use an isolated
asset configuration; genuine firmware runs explicitly supply their assets. A
prior ambient-configuration run failed a program replacement source check after
invoking the older installed firmware runner. Its failed result is retained in
the verification record, not counted as a pass.

The typing comparison additionally exposed a caret-phase sampling error:38
pixels differed in one cursor column. It now captures both stable blink phases
and requires exact matching for both complete expression/history/menu areas.
All three expressions pass both phases without a cursor mask or pixel tolerance.

## Verified virtual device environment

Finding32 now satisfies the original battery, USB/USBHOST, INDICATOR,
unknown-device output and disk-room requirements for the installed G1 profile.
Replies come from stateful virtual models checked against executable original
OS code. This does not claim electrical or physical-calculator verification.

Battery ADC and battery/charging/charged selections produce the complete16-byte
record and real queued OS notifications. Critical shutdown returns asynchronously
and honors the original inhibition variable. Disk reporting fills the complete
observed60-byte structure. Logical C-volume growth, append and sparse writes
respect its configured capacity; rejected metadata replacement/capacity changes
preserve existing data.

USB distinguishes cable, enable, callback, host descriptor/address/init and peer
extension state. Bounded queues transfer actual packets. A persistent guest ARM
worker invokes the firmware callback, including callbacks which wait. The new
`ppl device virtual` command and `Prime::open_virtual` transport use the original
firmware protocol without opening physical USB. Read-only readiness probes wait
for a genuine protocol reply; callback registration alone was insufficient.

The original firmware supplies its own information,320×240 PNG and program
transfer replies. Guarded upload, CRC/readback, existing-name refusal and fresh
boot execution returning1234576 passed. Battery updates changed the real LCD
while an interactive PPL program continued and returned73. Fourteen original-ROM
controls and production ARM tests verify the software structures and return
contracts, including explicitly configured USB host peer descriptors.

BACK implements brightness, inhibition and timed PWM ramps. INDICATOR preserves
raw pin bits and executes RTC-driven finite/indefinite patterns, progress,
restoration and ON cancellation. The RTC uses the original1980 epoch and keeps
running during power sleep. Automatic/indicator sleep cannot admit an OFF
recovery checkpoint. Unknown devices fail without fabricating output. The old
FACTORY stub, including invented empty factory data, has been removed; its
unestablished driver/partition behavior remains a finding30 requirement.

Actual GUI menu/slider/charging/Apply actions passed seven assertions. A separate
GUI process retained and reapplied ADC838/Charging with four further assertions.
The existing interface also receives actual indicator/backlight output. Physical
LED wiring, an electrical battery model and a physical USB link are unverified.
Whole-machine persistence of this working state is verified under finding29.

## Verified live whole-calculator saves

Finding29 now satisfies its original live save/resume requirement. The existing
firmware session controls expose **Save live state** and **Resume live state**.
Save captures the current execution without OFF. Resume replaces the owned helper
on its exact calculator profile and restores the saved machine; it does not
substitute a fresh boot. Closing normally still uses the existing OFF workflow.
Live saving and resuming are explicit actions.

One device-producer transaction pauses every CPU owner and excludes memory
writers. It captures all RAM and allocator state, banked ARM926/FPU/CP15 context,
scheduler and clocks, thread stacks and queued launches, virtual devices and
input queues, complete drive trees, open files and synchronization leases,
managed jobs/calls, compiler/runtime observations, typing and frame state.
Compression and atomic publication use immutable data after execution resumes.
Bounded checksummed parts and current/previous pointers preserve the earlier
save when capture or publication fails.

Restoration admits identities and resources in staging, constructs each CPU on
its own host thread, and holds all execution behind a startup barrier. Journaled
drive promotion retains old trees and rolls back interrupted installation.
Compiler channels reconnect to the exact saved request/source; both guest and
producer timers exclude loading and host downtime. The parent binds restored
requests and environment before Ready can apply startup settings. Pending IRQ
edges survive the pre-ISR pause; completed workers cannot become new launches.

A genuine PPL program saved during WAIT(0), followed by SIGKILL and a fresh
helper, returns **{113,37,150}** after physical input. This proves the local37
survives and its entry-side counter increment occurs once. A second timed WAIT(5)
frame survives ordinary Shutdown and returns **{1114,73}** after its remaining
guest delay. Battery838/Charging, board pins3 and128MiB capacity survive. Damaging
the current RAM payload explicitly selects the intact previous active WAIT.
Changing the ROM identity is refused without changing A/C files.

The actual desktop scenario clicks Save, performs normal Restart, clicks Resume
and presses calculator Enter across three helper sessions. It verifies the same
real PPL result and byte-identical320×220 program graphics. Full default2396 and
genuine-Giac2454 regressions, warnings-denied Clippy, existing firmware/ROM flows,
Native21-session workflows, SIGKILL/reopen, linkage and debugger checks pass.
An existing display-test capture race was independently reproduced: temporary
Busy was unwrapped and left an orphan CPU loop. The corrected caller retries only
Busy with a deadline and always stops on device-thread failure; all actual pixel
and input assertions remain. The interrupted run is recorded separately.

Archives are local and tied to admitted resource/backend schemas. Missing or
incompatible data is an explicit error. Current/previous generations and old
pre-restore drive trees are retained; there is no automatic pruning. No physical
calculator or external physical USB-peer state is reconstructed. Those limits
are distinct from the working virtual-device state included in the image.

## Firmware clipboard acceptance

Finding 28 now satisfies its original selected-text-to-system-clipboard criterion
for the installed G1 image. Normal Copy does not call the four named OS clipboard
stubs. A passive observer reads the firmware's committed text record after its
own setter has written the size, UTF-16 payload and type byte. The setter's code
digest gates installation; an unknown build reports this capability unavailable.
The observer never writes guest memory or uses the framebuffer or host typing
cache. Malformed or oversized records and invalid UTF-16 preserve the desktop
clipboard and report the error. Private HP glyph code points are preserved,
without claiming a general font-to-Unicode conversion.

The versioned session forwards a bounded clipboard update to the GUI's existing
egui clipboard path. Real LCD clicks also revealed that stationary touches
needed repeated MOVE samples; the normal pointer path now supplies them during
held presses and the minimum tap duration.

`verify_firmware_clipboard.py` passed five genuine local-firmware cases: source
entered only through calculator keys; repeated Copy; a three-character partial
selection; and exact multiline Unicode program Copy and Cut. Program source is
self-authored and imported into a disposable profile. The actual GUI scenario
clicks All/Copy through the mouse path, switches to Native Home, and presses
Ctrl+V. Seven assertions verify that the exact guest text arrived through the
operating-system clipboard. Sanitized controls are retained in
`tests/fixtures/firmware-clipboard-controls.json`. Notes bodies and every app
field have not separately been tested; other builds require an established ABI.

Production Sleep, GetEvent, SysPowerOff, event, semaphore and critical-section
waits now leave the native Unicorn callback and retain explicit service data.
Critical sections use stable guest IDs. Suspended owners also acknowledge pause
requests while remaining suspended. Regression tests verify bounded service
entry, remaining timer data, pause acknowledgement, descriptor lifetime, lock
ownership and exactly-once completion. Blocking synchronous ISR calls fail
explicitly while preserving the interrupted continuation.

The genuine local firmware lifecycle suite passed 2/2. A strengthened live
inspection test separately paused all owners and acquired memory with zero
parked callbacks, then delivered input to a running PPL WAIT program and verified
its returned 7. A faster pump exposed a physical-key delivery race; subsequent
presses now wait for the main UI acknowledgement, with the existing bounded
fallback for programs that own their own event loop. Genuine typing controls
still match physical input with zero differing pixels for all 3 expressions.

Production engine images now export versioned banked ARM926 register, floating
point and vetted writable CP15 state, pinned to the actual Unicorn runtime/model.
The engine explicitly selects ARM926; Unicorn's previous default was Cortex-A15.
Owners publish their images before acknowledging the exact pause epoch.
Synchronization images preserve stable object generations, reset/count/recursive
ownership state, guest waiter IDs and closed-object leases. Restoration binds
production pending services to these objects and checks the actual saved SVC frame.

The mapped-memory image captures every mapping and allocator layout under one
memory gate, with bounded binary payloads and checksums. It supports allocator
budgets smaller than their mapped region. Fresh-process self-authored ARM tests
restore RAM, CPU and resources together, then verify actual instructions produce
the expected result exactly once for event, semaphore and recursive-lock waits.
These tests use production execution/restoration code; they are not evidence of
whole-calculator restoration.

Writable VFS resource images preserve access/append modes, stream positions,
directory searches, remaining names, search-string allocations, cwd and handle
counters. A fresh-process test resumes reads, sparse writes, append and directory
iteration over an explicit matching storage copy. Replaced file identities,
changed storage and malformed images are refused before target installation.
Append writes now honor EOF after seeking, and exhausted handles never wrap.

The complete machine now composes these components with session producer/input,
managed jobs/calls and observer state at one coordinated boundary. Finding29 is
implemented by the genuine fresh-process and GUI Save/Resume results above;
independent component tests retain their narrower claims.

A retained genuine run exposed a return-observation race: while a peer engine
was active, immediate value capture refused the memory gate and the observer
finalized an Indeterminate result. The observer now parks before acquiring a
bounded coherent capture and its own mutex. Nested decoder reads are admitted
only inside that owned hook capture; ordinary busy-capture refusal is preserved.
Deterministic contention/timeout/observer regressions pass. Both genuine lifecycle
workflows subsequently returned 42 after invalid-source rejection with coherent
representations and no caveats, and final inspection passed. The earlier failed
run and the fix are recorded separately in verification.json. An earlier
unretained loader failure cannot independently be attributed to this race.




## All 33 finding outcomes

The detailed source/test matrix is [matrix.json](matrix.json). “Partial” and
“Unresolved” retain required work; they are not acceptance closures.

| ID | Finding | Previous status | New status | Actual outcome |
|---|---|---|---|---|
| 1 | Apps and app views | Partial | Partial | Partial: app-specific HP parity remains incomplete, including remaining Geometry tools, spreadsheet formatting/functions, additional one-variable plots and app-specific plot tools. |
| 2 | CAS and CAS settings | Implemented | Implemented | Implemented for supported Giac operations; external helper installation required, no claim of every HP CAS command matching Giac. |
| 3 | Home/app setup routes | Implemented | Implemented | Implemented and covered by settings/session/keypad and integrated GUI tests |
| 4 | Help and User keys | Implemented | Implemented | Implemented; User worker/reducer tests and actual desktop temporary/locked workflows passed |
| 5 | Vars/Toolbox/Templates | Partial | Implemented | All six tools execute insertion/inspection/management; actual command/template/unit results, scope deletion and process reopen pass. Full proprietary Help breadth remains a limit. |
| 6 | Alpha entry | Implemented | Implemented | Implemented |
| 7 | All primary keys | Implemented | Implemented | Implemented; full declarative primary/Shift/Alpha coverage and desktop input/navigation exercised |
| 8 | Shift scientific functions | Implemented | Implemented | Implemented |
| 9 | Other Shift legends | Partial | Implemented | Typed DMS/polar propagation, true HMS conversions, explicit history presentation and durable metadata now execute;79 genuine reference results and actual desktop/restart checks passed. |
| 10 | Space and Ans | Implemented | Implemented | Implemented |
| 11 | Parentheses/sign/fractions | Implemented | Implemented | Implemented |
| 12 | Cursor/selection/undo | Implemented | Implemented | Implemented |
| 13 | Interactive history | Implemented | Implemented | Implemented |
| 14 | Clipboard | Implemented | Implemented | Implemented |
| 15 | Idle LCD interaction | Implemented | Implemented | Implemented |
| 16 | Pointer dialogs | Implemented | Implemented | Implemented for native dialog controls |
| 17 | ON interrupts VM | Implemented | Implemented | Implemented |
| 18 | Native power/persistence | Implemented | Implemented | Implemented for calculator memory; no suspended Native VM stack persistence promised |
| 19 | Independent Programs catalog | Implemented | Implemented | Implemented; 17 real egui/controller/VM management tests plus restart and linked-program desktop workflows |
| 20 | Lists/matrices/notes | Implemented | Implemented | Implemented; real egui list/matrix/note CRUD, transactional editors, profile reopen and desktop object restart verified |
| 21 | EDITMAT dimensions | Implemented | Implemented | Implemented |
| 22 | INPUT controls | Partial | Implemented | Live-frame expressions, G1 filters, edit/checkbox/radio/choice controls, all valid default forms, retry/untouched semantics, confirmation and cancellation verified. |
| 23 | Nine unsupported entries | Partial | Implemented | All nine audited entries execute their supported semantics; genuine CAS source/data/dynamic calls and error values,26 app-specific views and actual PPL/GUI/reopen controls pass. RECALL is a verified inventory correction. |
| 24 | 27 approximate entries | Partial | Partial | Partial: raw UTF16 and selected-font TEXTOUT now match genuine controls; remaining overload/formatting and graphics/resampling parity require work. |
| 25 | Status and textbook display | Partial | Partial | Partial: full textbook layout for all result types and exact HP softkey parity remain incomplete |
| 26 | Held virtual keys | Implemented | Implemented | Implemented; firmware continues receiving actual held transitions |
| 27 | Firmware typing | Partial | Implemented | Genuine text/Paste ABI, literal Unicode/newlines/selection, actual math caret/templates, PPL INPUT, GUI paste beyond2048 and exactly-once resume pass. |
| 28 | Firmware copy-out | Unresolved | Implemented | Verified G1 committed UTF-16 clipboard observer; genuine Home partial/repeated Copy and Unicode program Copy/Cut; actual GUI Copy → OS clipboard → Ctrl+V passed. |
| 29 | Live whole-calculator save/resume | Unresolved | Implemented | Explicit GUI Save/Resume restores all CPU owners, RAM, storage/resources, jobs/input/observers and virtual peripherals. Genuine active/timed PPL, graphics, normal/SIGKILL restart, corruption recovery and ROM refusal pass. |
| 30 | Firmware OS coverage | Partial | Implemented | Used metadata/resource ABI and strict dispatch verified; named import faults/rebinding; accurate742-slot inventory retains656 unhandled gaps. This does not claim all742 APIs work. |
| 31 | Partial OS services | Partial | Implemented | Executable ROM controls establish thread/wake/priority/power semantics; real ARM, extended printf, pausable guest deadlines and genuine firmware/GUI regressions pass. HLE scheduling remains at bounded CPU slices. |
| 32 | Virtual device environment | Unresolved | Implemented | Stateful battery, USB/HOST callbacks/protocol, indicator/backlight, full disk structure and quota now execute;14 original-ROM controls, real firmware/PPL and GUI/reopen evidence pass. |
| 33 | Native automation dispatch | Implemented | Implemented | Implemented |

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
| APPROX | Recursive list/matrix approximation and typed CAS-to-Home numeric array projection match11 original15751 controls. Exact CAS answers remain independent. Giac symbolic factor print order and wider HP algebra compatibility remain limits. |
| STDDEV | Weighted population deviation in decimal arithmetic, vector/matrix-column forms, bounded frequencies and validation. |
| STRING | All six formats, custom UTF16 separators/signs, DotZero and per-component pixel fitting execute. Original regular/bold/mono font metrics and sixteen raw-unit fitting thresholds match. Legacy15157 fraction reconstruction now matches832 original results through an explicit persisted format-version choice. Explicit15751 arithmetic now preserves exact rational/pi provenance; new GUI profiles and exact history copy/restart now retain that model; wider formatting compatibility remains open. |
| EXPR | Existing native parser/evaluator; it does not silently promise every HP expression grammar/command. |
| APPEND | Lists and numeric vectors, correct copy semantics and type validation. |
| EDITMAT | Interactive cells, dimensions, insert/delete rows/columns, vectors, validation and cancellation. Bare calculator M variables write back; ordinary local matrix expressions return an edited value, matching retained controls. |
| RECT, LINE, PIXON, PIXOFF, GETPIX | Calculator plot-coordinate integration; LINE also implements numeric 3D meshes, transforms, clipping and reusable Native caches. Exact raster boundaries remain a parity limit. |
| CIRCLE | Native compatibility extension; the tested G1 rejects this name. It is not mislabeled as a verified HP drawing command. |
| FILLCIRCLE_P, ARC | FILLCIRCLE_P is a Native extension rejected by the tested G1. Scalar ARC has a pixel radius in its Cartesian form; original full/partial circle controls now match. Elliptical-radius/edge-fill/partial-sector overloads and transparency now match189 complete additional original frames; uninitialized thin-ellipse end-cap variation is recorded explicitly. |
| TEXTOUT_P, TEXTOUT | Seven selected font codes, raw UTF16 glyph indices, grouped typed values, partial bitmap-column clipping and measured backgrounds match complete original frames. Explicit zero width draws nothing and returns the full measured endpoint; negative/fractional widths fail. Wider screen layout remains25. |
| BLIT, SUBGROB | Pixel forms retained; Cartesian conversion, omitted-source forms, scaling and clipping corrected against retained genuine local-firmware controls. Exact resampling/raster parity remains limited. |
| DIMGROB | Dimensions and genuine packed fourth-argument pixel data, including observed 16/32-bit selection and transparency. |
| TRIANGLE_P | Scalar/vertex/mesh forms, gradient/opacity, transforms, clipping, reusable Native point caches and actual writable depth strings. Cartesian TRIANGLE shares this implementation. |
| MSGBOX | Real confirmation/cancel controls through the host; desktop pointer and keyboard actions. |
| INPUT | Live-frame expressions; actual G1 type filters, edit/checkbox/radio/choice controls, full/short/empty/scalar defaults, retry/untouched semantics and confirmation/cancellation writeback verified. Invalid descriptors fail explicitly. |
| CHOOSE | Selection, keyboard/pointer confirmation and cancellation through the real host. |
| MOUSE | Two optional pointer lists, zero-based indexed coordinates, retained drag origins and typed SYSTEM words now follow original controls. Actual desktop press/drag/release/new-tap programs pass at normal and1.5scale; release returns two empty lists and absent indexed values return real−1. Multi-touch and additional gesture generation are not claimed. |
| DISP | Existing terminal/display behavior retained; exact HP font/layout parity remains limited. |
| FREEZE | Waits for actual key input and remains interruptible. |
| DRAWMENU | Thirteen complete original frames match using the installed menu texture and font. Empty lists are valid; extra list entries after six are ignored. Raw isolated UTF16 labels render their original font indices. Actual IDE Run/Enter controls pass. |

Native 3D point caches remain explicitly versioned and backend-specific; opaque
firmware heap caches are refused. Depth writeback now preserves arbitrary UTF16
units, including isolated surrogates. DIM/ASC/indexing, edits, copies, parameters
and memory schema3 retain those exact units; legacy tagged depth input migrates.
See `docs/development/native-graphics-data.md`. TEXTOUT and menu rendering consume raw16-bit font indices directly, including
isolated units; they do not replace calculator output with escape text.

Selected Prime Sans rendering now matches three complete original-firmware
framebuffers with no masks or tolerance: joined text in seven font codes,
63 isolated glyphs, colored text and line feeds. Both Native CLI paths pass all
12 comparisons through configured/explicit resources. The current font parser
feeds a bounded, reentrant FreeType2.8 scan converter; no older font parser or
vendor font bytes are bundled. A selected-resource failure preserves existing
output files. Full graphics and GUI raster parity remains24/25.

## Verified text and installed Help resources

TEXTOUT now preserves raw UTF16 glyph indices. A surrogate pair uses two font
indices, following the original G1. Numeric and collection arguments use the
calculator's grouped formatting. Bitmap clipping happens before the glyph's
left bearing; background clearing uses the measured endpoint, one-pixel margins
and the logical cell heights of all seven fonts. Six unchanged original programs
match every return value and every pixel of their complete320x240 frames. Actual
IDE Run and calculator Enter interactions reproduce all six frames. The previous
three font and thirteen menu reference frames continue to match exactly.

DRAWMENU uses the light-menu texture from the user's installed resource archive.
All thirteen original positional/list, Unicode, collection, control-character
and clipping frames match without masks or tolerance. Invalid selected texture
records fail before changing the screen. The bundled-font mode remains an
explicit approximation. These PPL softkey results do not claim every Native
application menu is already identical.

`ppl-engine::help` reads eight original language books from installed resources,
with bounded archive/section/index/tree/text validation. It retains duplicate
names, stable identities, localized unnamed entries, the explicit vacant entry,
raw UTF16, formatting markers and grouped interface strings. All18,808 index entries
and6,962,095 decoded units match an independent resource decoder. Original
firmware Home Help and topic-tree navigation were exercised; the visible Home
paragraphs agree with the decoded book. Public fixtures contain hashes and
structural metadata rather than vendor prose or assets. This is the working data
loader: Native Help UI integration and original markup/math rendering remain required.
Tracing the original ARM reader established that the auxiliary records are
interface string groups. The Native loader now also decodes all281groups and
29,181strings, retaining336,829 UTF16 units. The original ARM decoder independently
produces every identical string byte and terminator; only its allocation boundary
is supplied by the test. Earlier opaque-image naming was corrected.

The combined numeric GUI/text/Help checkpoint passes2676default and2744 genuine
Giac workspace tests,0failed,6existing ignores. Warnings-denied all-target Clippy,
debug builds and formatting pass. The optimized desktop composite also passes94actual GUI processes and40
assertion invocations, including both fonts and separate-process reopen. An outer
SIGTERM after46successful processes was preserved; the remaining48processes and
all assertions completed in the same profiles. Its initial cause is unestablished.

Native application menus now share the installed texture. Two complete320x20
Function Numeric/Statistics Numeric softkey strips match the original firmware,
including label ink and popup corners. The primary GUI suite passes732tests,
0failed,2existing ignores. Affected Home/settings/Statistics/Numeric/gallery
flows pass18actual GUI processes and10assertion invocations across both fonts and
reopen. The first six processes passed before another outer SIGTERM; the remaining
twelve and all assertions passed in the preserved isolated profiles. The signal's
source remains unestablished. Other application menu classifications remain open.

## Firmware remains distinct

Desktop text now uses the installed firmware's original Paste/text-event methods.
A digest-admitted serializable plan owns source/event/derived argument buffers;
borrowed guest clipboard/editor data stays guest-owned. Actual math-editor source
and caret queries preserve quoted Unicode while established math aliases retain
their intended operators. Other firmware builds retain the explicit restricted
translator until their ABI is established.

The actual GUI now types a note name and pastes multiline Unicode text longer
than2048 characters, saves it, and obtains an exact genuine PPL Notes readback.
Separate genuine tests replace/save/execute a Program Editor source (return17),
insert digits into a SIN template, preserveⅈ×😀 inside a physically moved quoted
caret, and enter12.5 into running INPUT (return25). Queued text is required to be
present in a saved producer; after its helper is killed, a fresh resume and real
Copy returns314159265 exactly once. Complete caret phases of three expressions
match actual physical Copy/Paste with zero differing pixels. A final denominator
digit typed by keys leaves a different caret position (51 pixels); that evidence
is preserved rather than masked.

The production live session now stops on unhandled services. PE imports no longer
receive silent zero-return stubs: unresolved calls fault with their DLL/symbol;
a later dependency rebinds its real ARM/Thumb export. Original-ROM comparisons
verify complete ANSI/wide132-byte application metadata for PE/ELF/FM/FI modules,
actual named BIN/ROMSPC resource parsing, checksums/raw sizes, missing files and
bounds. Five legacy resource exports are themselves genuine no-op/not-found
exports; their measured behavior is kept separate from the real internal parser.
Original device registry execution establishes that FACTORY/FTL are absent;
OPEN_EXISTING fails instead of returning fabricated factory data.

`prime-emulator service-inventory` prints all742 actual slots. The refreshed
[current inventory](current-service-inventory.json) has55 implemented,31 partial
registrations and656 unhandled slots. This closes finding30's recorded used-ABI
and accurate-inventory criteria, not every possible OS API. Unhandled slots fault
if invoked. Nine strict genuine firmware workflows, executable original-ROM
controls, virtual USB and desktop clipboard/environment/live-resume regressions
pass. Physical USB, optional drivers, broader stream dispositions and electrically
cycle-accurate hardware remain outside the established coverage.

Local firmware execution used legitimate existing assets and disposable copied
profiles. No physical calculator was accessed. No firmware assets, vendor binaries
or opaque heap captures are included in the repository.

## Original verification record

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

## Verified VM-owned Numeric Setup and pointer input

Numeric settings now belong to each application in the VM. PPL and catalog
writes use the same validation and state for NumStart, NumStep, NumZoom, NumType
and the custom NumIndep rows. Function, Parametric and Polar default to
0/0.1/2/0; Sequence defaults to1/1/2/0, following original-firmware controls.
Rejected assignments preserve the previous state. Word cells normalize to real
custom rows. Version4 memory retains decimal values such as1E400 and1E−400,
migrates earlier global numeric settings, and validates restoration atomically.
Separate-process restoration passes. Graph, Function and spreadsheet workers
reject attempted numeric-state writes from called programs.

The integrated pointer host preserves the actual press origin and New/Drag
classification through movement and release. Original signed16-bit coordinate
words and unsigned16-bit event words retain SYSTEM base affinity. Actual PPL
programs driven by real desktop LCD events pass at normal and1.5scale. Unsupported
additional gesture generation is not represented as verified behavior.

This checkpoint passes2,615 default and2,682 genuine-Giac workspace tests,
with0failures and6existing ignores, plus all-target warnings-denied feature
Clippy, debug builds and formatting. The later Function Numeric21 and numeric backend25 checkpoint below supersedes
those two implementation gaps. GUI exact-number profile/history integration
and remaining app, compatibility and display behavior stay open under1/24/25.

## Remaining required work

### Verified current numeric backend

Native `--numeric-version 15751` now selects the newer exact real model.
Finite decimal source digits, bounded rational coefficients and multiples of π
retain their arithmetic identity through compiled PPL, EXPR, installed programs,
collections, typed JSON arguments, CAS sharing and version5 memory. Overflow of
the exact coefficient representation produces an explicitly decimal result;
subsequent operations preserve that provenance. Pi rounding and comparisons use
outward-bounded intervals. Legacy15157 arithmetic remains independently selectable.
Earlier saved profiles keep their previous arithmetic model.

Active-app angle and complex settings now affect actual Home/PPL arithmetic
without rewriting the Home setting. Explicit complex arguments retain their type,
including zero imaginary parts, while packed numeric matrix entries normalize
as observed. Complex SIN/COS/TAN, EXP and LN execute with the original argument
and wide-exponent behavior. Runtime real overflow saturates with the observed
sign; strict source/configuration parsing continues to reject out-of-range data.

Ten independently executed original15751 source programs provide237 returned
fields. Four further sources have original15157 evidence and run through the
legacy Native model too. Genuine Giac sharing and separate-process exact-memory
restoration pass. The combined Function Numeric21/backend25 tree passes2643
default and2711 genuine-Giac workspace tests,0failed,6existing ignores, plus
warnings-denied all-target Clippy, builds and formatting. Subsequent dual-model
control execution also passes. New GUI profiles now select this model and preserve exact typed-history recall;
old schema4 profiles retain Legacy arithmetic. These results do
not close the remaining app, compatibility or display findings.

The matrix contains **30 implemented, 3 partial and 0 unresolved findings**.
These are engineering outcomes, not a declaration that all 33 requirements closed.
The three open findings are Native app1, overload/graphics24 and display25:

| Remaining Native limitation | Code cause and work required |
|---|---|
| Complete app parity | Thirteen fits, ten VM-owned statistics columns and five weighted analyses now execute through actual app views. Geometry Translation, shape-preserving transforms, Projection and Polygon tools now execute alongside the existing dependency-aware constructions. All ten declared one-variable chart choices dispatch to their own numerical or stem-and-leaf views. Geometry PPL bindings, remaining spreadsheet functions/formatting, and complete firmware fidelity of charts and other app tools remain required work. |
| Complete app-specific tools | Per-app STARTVIEW ordering and26 genuine/menu-based actions now execute. Full analysis/construction/spreadsheet tools remain part of app1; invalid or unavailable menu numbers return0. |
| Complete Help/catalog database | `ppl-engine::help` now decodes all eight installed language books, including exact text and metadata. Native browser integration now provides asynchronous loading, eight-language navigation, Unicode search and stable original topic identity. Original markup/math rendering remains required; firmware Help entries do not imply Native command support. |
| Full textbook/result metadata | `native/math_layout.rs` implements editable input structures; matrix cells now share row baselines and Show retains structural inspection independently of the history display mode; some result types still use textual presentation. DMS/polar value metadata now propagates and persists; exact HP softkey/rich-note formatting and structural rendering of every result type remain incomplete. |
| Remaining graphics parity | Raw UTF16 strings and binary DIM/ASC/indexing/writeback now work; selected-font TEXTOUT matches exact complete reference frames. Native point caches are portable; guest heap-address caches remain opaque. Other raster/resampling boundaries still require controls and implementation. |
| Remaining command overloads | INPUT control/default semantics now satisfy finding22 for the installed G1. Other command overloads, graphics and broader grammar/CAS/formatting compatibility remain24. Dynamic unloaded-catalog EXPR now executes in the same VM. Compiler Approximate classifications remain conservative. |

The missing Native features above remain required work. Live whole-machine Save/Resume is implemented and verified below. Broader
OS API coverage remains explicitly inventoried; scoped finding30 is closed. The application is not
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

Historical baseline publication: implementation and validation completed before
publication. The user subsequently
authorized a checkpoint commit, branch push and preservation PR. The PR targets
`feat/g1-usability-overhaul`, which supplies the existing milestone dependencies.
The local results above do not assert hosted CI success.

This isolated continuation has not been committed, pushed or published.
