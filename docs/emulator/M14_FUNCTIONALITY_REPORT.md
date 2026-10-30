# M14 — Genuine G1 functionality

Date: 2026-10-30. Branch `feat/m12-full-calculator` (canonical checkout), on
top of M13. No commit, push or merge. No physical calculator was contacted.
Firmware: local G1 build 15751.

Everything here is the original firmware's own functionality, driven
through its real UI (keypad, touch, soft keys) or its own PPL runtime. No CAS,
graph, app screen or result was imitated or substituted. All of it is
**local firmware** evidence; physical parity is M15.

## 1. Outcome in one paragraph

The firmware's full functionality runs under the emulator:

- **CAS.** Integration, differentiation, factorisation, solving, exact
  fractions, exact π and expansion all compute and render correctly.
- **Function app.** It plots, traces, zooms and tabulates correctly.
- **The 18 built-in apps.** Each one navigates through its own Symbolic,
  Plot and Numeric views.
- **App computations.** Statistics 1Var and 2Var, Solve, Spreadsheet
  formulas and Linear Solver give the mathematically correct results.

No new OS service was needed: every workflow hit only the two services
already reported since M12 (`_FindResourceW`, `GetApplicationHeadInfoA`),
with no visible effect. Four PPL-level results remain open and are recorded
as expected failures for M15 to resolve against the device (§5).

## 2. CAS view (keypad, the firmware's CAS)

`m14_cas`: CAS key, then each command typed on the keypad.

- **Letters.** In the CAS view `Alpha`+letter key types lowercase (observed:
  `Alpha 7` → q, `Alpha Shift 7` → Q).
- **Templates.** `( )` opens a template and Right leaves it; `÷` opens a
  fraction; π is `Shift 3`.
- **Check.** The newest result's glyph block, cropped between the firmware's
  history separators, must equal the result of typing the expected expression,
  and must differ from a control.

| Command | Result on the firmware's CAS screen | Check |
|---|---|---|
| `int(2*x,x)` (shown as ∫2·x dx) | x² | = `x²`, ≠ `2*x` |
| `diff(sin(x),x)` | COS(x) | = `cos(x)`, ≠ `sin(x)` |
| `factor(x²−1)` | (x−1)·(x+1) | ≠ `x²−1`. The CAS reorders a typed `(x−1)·(x+1)` to `(x+1)·(x−1)`, so only the control is comparable; the result was read off the screen |
| `solve(x²=4,x)` | {−2, 2} | = `{-2,2}`, ≠ `{2,3}` |
| `1/3+1/6` | 1/2 | = `1/2`, ≠ `1/3` |
| `3·π−π` | 2·π | = `2*π`, ≠ `3*π` |
| `expand((x+1)²)` | x²+2·x+1 | = `x²+2*x+1`, ≠ `x²+1` |

## 3. Graphing and apps

**Function app** (`m14_function`). F1 = X² and F2 = X+1 are set and checked
by a firmware-run program (`F1:="X^2"; CHECK(1)`), then driven by keys:

- **Symb** shows both, checked.
- **Plot** draws both curves.
- **Right** traces: X: 0.5, F1(X): 0.25.
- **+ / −** zoom in and out.
- **Num** shows the table: F1 0, 0.01 … 0.81; F2 1, 1.1 … 1.9.

Before the functions were checked, Plot showed the firmware's own "No
functions checked." and the Num view was empty. The plot, zoomed and table
screens are compared exactly with verified references.

**All 18 built-in apps** (`m14_apps`). Each was started by the firmware's
`STARTAPP`, then opened in Symb, Plot and Num (screens in
`artifacts/m14/run*/m14_apps/`). Every app opened its own genuine screens:

| App | Observed |
|---|---|
| Function, Parametric, Polar, Sequence | Symbolic views; "No functions/sequences checked." with nothing checked |
| Advanced Graphing | V1… open sentences; plot credit "©2017 Pedagogery Software" |
| Graph 3D | FZ1(X,Y)…; draws its 3D axis box |
| Geometry | its plot, Symbolic and Numeric views |
| Spreadsheet | the grid |
| Statistics 1Var / 2Var | data columns; with no data "Error: Insufficient statistics data" |
| Inference | Z-test; plots the normal curve with P = 0.172…, and its Numeric view |
| Data Streamer | "Streamer not connected…" (needs StreamSmart hardware) |
| Solve | equations; "Check exactly one equation." with none checked |
| Linear Solver | 3×3 grid; "Infinite number of solutions" for zeros |
| Explorer | Quadratic: parabola with roots, discriminant, vertex, and table |
| Finance | TVM; "Error: Invalid N" with no inputs |
| Triangle Solver | its input form and figure |
| Python | Numeric view and the `main.py` editor |

**App computations** (`m14_appwork`). Data is entered by program or keys,
then the app's own soft key is pressed. Each value was checked by hand, and
each result screen is a verified reference compared exactly afterwards:

| App | Input | Firmware result |
|---|---|---|
| Statistics 1Var | D1 = {1,2,3,4}, Stats | n 4, Min 1, Q1 1.5, Med 2.5, Q3 3.5, Max 4, ΣX 10, ΣX² 30, x̄ 2.5, sX 1.29099444874 |
| Solve | E1: X²=4, X=1, Solve | X: 2 |
| Statistics 2Var | C1 = 1..4, C2 = 2x+1, Stats | n 4, r 1, R² 1, sCOV 3.3333…, σCOV 2.5, ΣXY 70 |
| Spreadsheet | A1 5, A2 `=A1*2` | A2 10 |
| Linear Solver 2×2 | x+y=3, x−y=1 | X: 2, Y: 1 |

## 4. Mathematical data types through the firmware runtime

`tests/runtime-m14/` has 23 cases, run by `prime-emulator app-suite`. Each is a
PPL program compiled and executed by the firmware, with its return value
decoded by the host. Expected values were written from the mathematics
before any run (`generate.py`).

Five expectations were then corrected to how the firmware encodes the same
value, each with its reason recorded in `generate.py`:

- minus as U+2212;
- the imaginary unit as the private-use U+E003;
- `COS(x)` for STRING of a CAS result in a Home program;
- the host showing a decoded real as `0.5`.

Two of my own probes were invalid uses of the app API (`STARTAPP` takes
effect after the program ends) and were removed; those app computations are
verified through the app UI in §3.

The cases cover:

- **CAS through `CAS("…")`:** factor, solve, diff, int, simplify, expand,
  exact fraction, exact π, exact trig, limit, symbolic subst.
- **Complex:** product and `ABS`.
- **Matrices and vectors:** product, `DET`, inverse, `CROSS`.
- **Lists:** a nested list.
- **Reals:** √2 to 12 digits, 1/3.
- **Statistics and probability:** `COMB`, `NORMALD_CDF`, `FNROOT`.

Results: runs 2 and 3 (`artifacts/m14/run{2,3}/maths/SUMMARY.md`): **19 PASS, 4 XFAIL, 0 unexpected**, in each run. Run 1 used the suite before the corrections above, and is kept as the record of those corrections.

## 5. Open items (expected failures, for M15)

| Case | Local firmware | Reasoning |
|---|---|---|
| `cas-int` | `STRING(CAS("int(2*x,x)"))` → `2*x^2/2` | The CAS view gives x² (§2). The difference is in what `CAS("…")` returns to a Home program |
| `cas-exact-pi` | `3*π-π` | The CAS view gives 2π |
| `cas-symbolic` | `a*2+b` | Unsimplified ordering |
| `real-third` | `RETURN 1/3` decodes as `1/3` | **Resolved in M15:** through `STRING`, `1/3` is `0.333333333333` on both local builds, as on the G1. The `1/3` is the host decoder's display of 15751's returned object. Separately, 15751 computes `(1/3)*3` as `1` where 15157, local and physical, gives `0.999999999999`: a firmware-version difference |

These are not claimed as matches. M15 runs the identical programs on the
physical G1 and on the 15157 local firmware to decide whether each is
firmware behaviour, a version difference, or an emulation or decoding fault.

## 6. Findings for later milestones

- **Typed letter case depends on the view.** Alpha alone is uppercase at
  Home and lowercase in CAS. The IDE's desktop-text path types lowercase as
  Alpha+Shift (correct at Home). Following the firmware's current view is an
  M16 input-state item.
- **Templates.** Typing `(`, `)`, `^`, `=` from a desktop keyboard maps onto
  the keypad's templates; a faithful text-to-keys translation is M16 work.
- **Blinking cursors** (Plot trace, Python editor) mean those screens are never
  still. Their checks use fixed waits, not exact references.

## 7. Test results

| Suite | Run 1 | Run 2 | Run 3 |
|---|---|---|---|
| IDE (`assert_m14.py`): CAS, Function, 18 apps, app computations | 59/59 | 59/59 | 58/59 |
| Maths (`tests/runtime-m14`) | (pre-correction) | 19 PASS + 4 XFAIL | 19 PASS + 4 XFAIL |

- **Run 1.** Two problems were fixed in the checks, not the firmware, then
  run 1 was re-asserted:
  - the CAS glyph crop now uses the firmware's own history separators, so an
    integral template's taller row compares correctly;
  - the Statistics 2Var reference was re-recorded from a settled screen,
    because the first one was captured mid-draw (`Σ…` instead of `ΣXY`).
- **Run 3's one failure** is a settle wait. The Statistics 2Var result screen
  kept changing for the 15 s window, although its frame still matched the
  verified reference exactly. It is reported as is.
- **Other checks.**
  - `cargo test --workspace --no-fail-fast`: 1592 passed, and 2 failed with
    the known `ppl-programs` journal lock; the same binary passed 18/18
    serially, and that crate is unchanged.
  - clippy and fmt: clean.
- **New unit test:** lowercase letters type with Alpha+Shift (Home
  convention).

## 8. Reproduction

```bash
cd tests/ide_xvfb_smoke
R=$PWD/../../artifacts/m14/run
for s in m14_cas m14_function m14_apps m14_appwork; do M12_RUN=$R ./run_m12.sh $s 1800; done
python3 assert_m14.py $R
cd ../.. && prime-emulator app-suite --firmware … --os-rom … --factory-a … --init-data … \
  --suite tests/runtime-m14 --out-dir $R/maths
```
