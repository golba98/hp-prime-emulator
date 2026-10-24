> Historical engineering record. Counts, commands and status below describe
> the recorded session, not a current guarantee. See the root README and latest
> milestone reports for supported interfaces and validation.

# E2E Test Infra: HP Prime G1 Rust Emulator Native Desktop IDE

## Test Philosophy
- Opaque-box, requirement-driven testing. Derived strictly from `ORIGINAL_REQUEST.md` and user specifications, with zero coupling to GUI implementation internals.
- Methodology: Category-Partition + Boundary Value Analysis (BVA) + Pairwise Combinatorial + Real-World Workload Testing.
- Progressive testability: verification operates via programmatic headless IDE scripting, CLI integration, and headless runner test harnesses.

## Feature Inventory & Test Coverage Goals
| # | Feature | Requirement | Tier 1 (Coverage) | Tier 2 (Boundary) | Tier 3 (Pairwise) | Tier 4 (Real-World) |
|---|---------|-------------|:-----------------:|:-----------------:|:-----------------:|:-------------------:|
| 1 | Engineering Dark Aesthetic | R1 | 5 | 5 | ✓ | ✓ |
| 2 | Main Menu Actions | R1 | 5 | 5 | ✓ | ✓ |
| 3 | Toolbar Controls | R1 | 5 | 5 | ✓ | ✓ |
| 4 | Resizable Multi-Panel Split | R1 | 5 | 5 | ✓ | ✓ |
| 5 | Panel State & Visibility Persistence | R1 | 5 | 5 | ✓ | ✓ |
| 6 | Multi-Tab Editor Buffer Lifecycle | R2 | 5 | 5 | ✓ | ✓ |
| 7 | File Operations (New, Open, Save, Save As) | R2 | 5 | 5 | ✓ | ✓ |
| 8 | PPL Syntax Highlighting Tokenization | R2 | 5 | 5 | ✓ | ✓ |
| 9 | Live Compiler Diagnostics & Error Gutter | R2 | 5 | 5 | ✓ | ✓ |
| 10 | 320×240 GROB Nearest-Neighbor Display | R4 | 5 | 5 | ✓ | ✓ |
| 11 | Live GROB Drawing Commands Rendering | R4 | 5 | 5 | ✓ | ✓ |
| 12 | 0–50 Virtual Keypad Grid Input | R4 | 5 | 5 | ✓ | ✓ |
| 13 | Screen Touch / Pointer Interaction | R4 | 5 | 5 | ✓ | ✓ |
| 14 | Focus Separation (Editor vs Calc) | R4 | 5 | 5 | ✓ | ✓ |
| 15 | Screenshot PNG Export | R4 | 5 | 5 | ✓ | ✓ |
| 16 | Background VM Worker Execution | R3 | 5 | 5 | ✓ | ✓ |
| 17 | Unsaved Editor Buffer In-Memory Run | R3 | 5 | 5 | ✓ | ✓ |
| 18 | Execution Controls (Run, Stop, Restart) | R3 | 5 | 5 | ✓ | ✓ |
| 19 | Cooperative Cancellation Without Leaks | R3 | 5 | 5 | ✓ | ✓ |
| 20 | Dedicated Output Console PRINT Stream | R3 | 5 | 5 | ✓ | ✓ |
| 21 | Interactive INPUT Dialog & Expression Eval | R5 | 5 | 5 | ✓ | ✓ |
| 22 | Interactive CHOOSE Option List Dialog | R5 | 5 | 5 | ✓ | ✓ |
| 23 | Interactive MSGBOX Dialog | R5 | 5 | 5 | ✓ | ✓ |
| 24 | GETKEY & ISKEYDOWN Keycode Dispatch | R5 | 5 | 5 | ✓ | ✓ |
| 25 | Non-Blocking WAIT Loop Execution | R5 | 5 | 5 | ✓ | ✓ |
| 26 | Gutter Breakpoint Toggle & Management | R6 | 5 | 5 | ✓ | ✓ |
| 27 | Statement Stepping (Into, Over, Out, Continue) | R6 | 5 | 5 | ✓ | ✓ |
| 28 | Execution Pause Line Highlighting | R6 | 5 | 5 | ✓ | ✓ |
| 29 | Variable Inspection Tree (Scalars, Lists, Mats) | R6 | 5 | 5 | ✓ | ✓ |
| 30 | Call Stack Frame Inspection | R6 | 5 | 5 | ✓ | ✓ |
| 31 | Full Workflow Integration | AC | 5 | 5 | ✓ | ✓ |
| 32 | Quality, Lint & Performance Gates | Gate | 5 | 5 | ✓ | ✓ |

## Test Architecture
- Test runner: programmatic integration tests under `tests/ide_e2e/` (or `crates/ppl-gui/tests/`) running headlessly or via `cargo test --test ide_e2e`.
- Verification format: programmatic assertions on `IdeAppState`, `GuiSharedState`, compiler diagnostics, and execution traces.
- Pass/fail semantics: `cargo test --test ide_e2e` exits 0 with all test cases passing.
- Output signal: creates `TEST_READY.md` upon completion.

## Real-World Application Scenarios (Tier 4)
| # | Scenario | Features Exercised | Complexity |
|---|----------|--------------------|------------|
| 1 | Complete Scratchpad Development (`TEST`) | F6, F7, F8, F9, F16, F17, F18, F20 | Medium |
| 2 | Fireworks Animation Simulation (`FIREWORKS`) | F10, F11, F16, F18, F19, F20 | High |
| 3 | Interactive Input Quadratic Solver | F6, F16, F20, F21, F23 | High |
| 4 | Stepping & Breakpoint Variable Debugging | F6, F8, F16, F26, F27, F28, F29, F30 | High |
| 5 | Syntax Error Recovery & Click-to-Fix | F6, F8, F9, F17 | Medium |
| 6 | Menu Navigation & Multi-Tab Switching | F2, F4, F5, F6, F7 | Medium |
| 7 | Calculator Keypad Interactive Game Loop | F10, F12, F14, F16, F24, F25 | High |
| 8 | Dialog Cancellation & Variable Invariance | F21, F22, F23, F29 | Medium |
| 9 | Rapid Run-Stop-Restart Stress Loop | F16, F18, F19, F20 | High |
| 10 | High-DPI Scaling & Resizing Invariance | F1, F4, F10, F14 | Medium |

## Coverage Thresholds
- Tier 1: ≥5 per feature (≥160 test cases)
- Tier 2: ≥5 boundary cases per feature (≥160 test cases)
- Tier 3: pairwise interaction coverage (≥32 combination test cases)
- Tier 4: ≥10 realistic application scenarios
- **Total Minimum: ≥362 test cases**
