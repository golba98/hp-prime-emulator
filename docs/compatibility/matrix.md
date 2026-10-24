# Compatibility Matrix: Built-in Commands

Status definitions:
- **IMPLEMENTED**: Native Rust bytecode implementation exists in `ppl-vm`.
- **TESTED**: Unit or regression tests in `tests/` or `crates/ppl-vm/tests/parity.rs` verify behavior.
- **VERIFIED**: Empirical hardware evidence from a physical HP Prime G1 matches the emulator.
- **UNSUPPORTED**: Deliberately rejected with an explicit runtime error (e.g., CAS operations, external file systems).

---

| Command | Category | Status | Notes |
|---|---|---|---|
| `+`, `-`, `*`, `/` | Math | TESTED | Exact 12-digit BCD decimal arithmetic |
| `MOD`, `DIV` | Math | TESTED | Floored negative arithmetic (`-7 MOD 3 == 2`) |
| `^`, `SQRT` | Math | TESTED | Powers, roots, exponent bounds $\pm 499$ |
| `ROUND` | Math | TESTED | Half-away-from-zero rounding |
| `ABS`, `SIGN` | Math | TESTED | Real, Int, and Complex support |
| `SIN`, `COS`, `TAN` | Math | TESTED | Respects `HAngle` (Radians, Degrees, Gradians) |
| `ASIN`, `ACOS`, `ATAN` | Math | TESTED | Inverse trigonometric operations |
| `LN`, `EXP`, `LOG`, `ALOG` | Math | TESTED | Exponential and logarithmic operations |
| `TYPE` | Math | TESTED | Types: 0 real, 1 int, 2 str, 3 cplx, 4 mat, 6 list |
| `STRING` | Strings | TESTED | Terminal-compatible display formatting |
| `LEFT`, `RIGHT`, `MID` | Strings | TESTED | 1-based substring slicing |
| `DIM`, `INSTRING` | Strings | TESTED | String length and character index search |
| `UPPER`, `LOWER` | Strings | TESTED | Case transformation |
| `ASC`, `CHR` | Strings | TESTED | Character code extraction |
| `MAKELIST`, `SIZE` | Lists | TESTED | List generation and element count |
| `HEAD`, `TAIL`, `APPEND` | Lists | TESTED | List head/tail and insertion |
| `SORT`, `REVERSE` | Lists | TESTED | Pure function returning a new sorted/reversed list |
| `POS` | Lists | TESTED | 1-based element search in list |
| `MAKEMAT`, `IDENMAT` | Matrix | TESTED | Matrix generation and identity matrices |
| `DET`, `TRN` | Matrix | TESTED | Exact determinant and transpose |
| `EDITMAT` | Matrix | TESTED | Matrix element update |
| `RECT_P`, `RECT` | Graphics | TESTED | Solid rectangles and borders |
| `LINE_P`, `LINE` | Graphics | TESTED | Bresenham lines with clipping |
| `PIXON_P`, `PIXOFF_P` | Graphics | TESTED | Pixel toggling on G0–G9 |
| `GETPIX_P`, `GETPIX` | Graphics | TESTED | Pixel color query in integer color packing |
| `CIRCLE_P`, `FILLCIRCLE_P` | Graphics | TESTED | Circle rasterization with midpoint algorithm |
| `ARC_P` | Graphics | TESTED | Elliptical arcs |
| `TEXTOUT_P` | Graphics | TESTED | Bundled DejaVu Sans rendering |
| `BLIT_P` | Graphics | TESTED | Sub-GROB blitting with transparency masking |
| `DIMGROB_P` | Graphics | TESTED | Dynamic GROB sizing |
| `GROBW_P`, `GROBH_P` | Graphics | TESTED | GROB dimension queries |
| `INVERT_P` | Graphics | TESTED | Color inversion |
| `PRINT` | I/O | TESTED | Terminal and emulated screen output |
| `INPUT`, `CHOOSE` | I/O | TESTED | Interactive modal input with headless script feeding. INPUT honours `{{var,[types]}}`: a `[2]`-only field stores the typed text as a string without evaluating it; a rejected entry (syntax error or disallowed type) reopens the dialog with an error instead of stopping the program; Esc/ON cancels (returns 0). Typed-field behaviour follows HP staff statements on the HP Museum forum, UNVERIFIED on G1 (see `tests/fixtures/baseconv/README.md`) |
| `GETKEY`, `ISKEYDOWN` | I/O | TESTED | 0–50 matrix keycode grid |
| `WAIT` | I/O | TESTED | Millisecond pause or keypress wait |
| `MOUSE` | I/O | TESTED | Touch coordinate tracking |
| `CAS.*` | CAS | UNSUPPORTED | Symbolic operations raise explicit error |

## Real-world finding: hpcalc.org `Benchmark` (2026-10-22)

Curated import (single download, `https://www.hpcalc.org/prime/apps/benchmark.zip`,
sha256 28f112ab0ba6e85f47c4e2114d78320acb0df4f7352b45ed52dd38bdf5951528)
classified by `ppl test import`:

* `Benchmark.hpappdir/Benchmark.hpappprgm` → **ppl_parse_error**. The embedded
  PPL (extracted from the binary container) uses constructs the parser does not
  accept at file level: bare variable statements (`nbTests := 3;`) and the
  `VIEW "Setup", NumSetup()` app-view declaration. On real hardware these are
  valid inside an *app* program. Supporting `VIEW` declarations and app-level
  globals is a confirmed compatibility gap (the emulator has no app framework).
* All other archive members are app data (icons, results screenshots) and are
  quarantined unchanged with provenance.

### Parser robustness (fixed after this import crashed the toolchain)

Feeding the binary container to the parser used to (a) loop forever pushing
diagnostics for an unclaimed top-level `END` token and (b) render one
whole-line diagnostic per invalid character (quadratic; 20 GB from a 13 KB
input). Both are fixed (`crates/ppl-syntax/src/parser.rs` `robustness_tests`,
`diag.rs` line windowing); the importer now also extracts `.hpappprgm`
source and refuses to parse non-source text.
