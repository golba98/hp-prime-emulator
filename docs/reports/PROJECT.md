> Historical engineering record. Counts, commands and status below describe
> the recorded session, not a current guarantee. See the root README and latest
> milestone reports for supported interfaces and validation.

# Project: HP Prime G1 Rust Emulator Native Desktop IDE

## Architecture
The Native Desktop IDE is built inside `crates/ppl-gui` as a multi-panel desktop application using `eframe 0.32` and `egui 0.32` with hardware-accelerated OpenGL (`glow`), operating over the existing workspace crates (`ppl-vm`, `ppl-compiler`, `ppl-syntax`, `ppl-graphics`, `ppl-math`, `ppl-device`).

### Component Structure & Data Flow
```text
┌────────────────────────────────────────────────────────────────────────────────┐
│                         GUI Main Thread (eframe / egui)                        │
│                                                                                │
│  ┌──────────────────────────────────────────────────────────────────────────┐  │
│  │ MenuBar & ToolBar (File, Edit, Program, Run, Debug, View, Tools, Help)   │  │
│  └──────────────────────────────────────────────────────────────────────────┘  │
│  ┌─────────────────┬──────────────────────────────────┬─────────────────────┐  │
│  │ ProgramExplorer │ EditorTabs (Multi-tab PPL)       │ CalculatorPanel     │  │
│  │ - File tree     │ - Syntax highlighting (ppl-lex)  │ - 320×240 GROB Disp │  │
│  │ - Outline       │ - Diagnostics underlines & gutter│ - Virtual Keypad    │  │
│  │ - Recent files  │ - Breakpoints line markers       │ - Screenshot export │  │
│  └─────────────────┴──────────────────────────────────┴─────────────────────┘  │
│  ┌──────────────────────────────────────────────────────────────────────────┐  │
│  │ BottomDock Tabs: [Output (PRINT)] [Problems] [Variables] [Call Stack]    │  │
│  └──────────────────────────────────────────────────────────────────────────┘  │
│  ┌──────────────────────────────────────────────────────────────────────────┐  │
│  │ StatusBar: Ln/Col | State | Steps/Time | Resolution | Ready              │  │
│  └──────────────────────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────┬─────────────────────────────────────────┘
                                       │
            Shared State Bridge (Arc<IdeSharedState> / GuiSharedState)
            - Screen Framebuffer (Mutex<Grob>) & Generation counter
            - Key Event Queue & Held Keys HashSet
            - Terminal / Output Lines (Mutex<Vec<String>>)
            - Execution Controller Channels / Signals (AtomicBool, Condvar)
            - Debug Pause Snapshot (Arc<Mutex<Option<DebugPauseSnapshot>>>)
                                       │
┌──────────────────────────────────────▼─────────────────────────────────────────┐
│                     Background VM Worker Thread (ppl-worker)                   │
│                                                                                │
│  ppl_compiler::compile_source(&file) ──► ppl_vm::Machine                       │
│  - Host Trait Implementation (GuiHost): PRINT, dialogs, keys, non-blocking WAIT│
│  - Debugger Trait (Op::Stmt): Breakpoint detection, step-into/over, snapshots   │
│  - Cooperative Cancellation: 1024-step check & condvar wait-slice interruption │
└────────────────────────────────────────────────────────────────────────────────┘
```

## Feature Inventory
| # | Feature | Description | Milestone | Source |
|---|---------|-------------|-----------|--------|
| 1 | Classic Engineering Aesthetic | Neutral dark palette (#25292D/#2D3237), crisp borders (#454B51), squared corners | M1 | R1 |
| 2 | Main Menu Bar | Menus: File, Edit, Program, Run, Debug, View, Tools, Help | M1 | R1 |
| 3 | Action Toolbar | Toolbar buttons: New, Open, Save, Compile, Run, Stop, Debug, Settings | M1 | R1 |
| 4 | Resizable Multi-Panel Layout | Split panels for Explorer, Editor, Calculator, Bottom Dock, and Status Bar | M1 | R1 |
| 5 | Panel State Persistence & Toggles | Collapsible/toggleable secondary panels remembering visibility across frames | M1 | R1 |
| 6 | Multi-Tab Editor Core | Multi-tab buffer management, modified indicator (*), close, line numbers, cursor Ln/Col | M2 | R2 |
| 7 | Native File Operations | New Program (PPL template), Open file dialog via rfd, Save, Save As, recent files | M2 | R2 |
| 8 | PPL Syntax Highlighting | Real-time token highlighting (keywords, numbers, strings, hashes, builtins, comments) | M2 | R2 |
| 9 | Live Compiler Diagnostics | Inline error underlines, gutter problem indicators, clickable diagnostic list navigating to line/col | M2 | R2 |
| 10 | 320×240 GROB Texture Embedding | Nearest-neighbor integer/aspect-ratio scaling of calculator screen without blur | M3 | R4 |
| 11 | Live GROB Drawing Updates | Immediate rendering of RECT_P, PIXON_P, LINE_P, TEXTOUT_P, BLIT_P via screen generation counter | M3 | R4 |
| 12 | Virtual Keypad Grid (0–50) | Authentic HP Prime layout, labeled buttons, distinct color palette, click-to-input | M3 | R4 |
| 13 | Screen Touch / Pointer Interaction | Mouse drag/click on display mapped to 320×240 Touch coordinates | M3 | R4 |
| 14 | Focus Separation | Editor focus isolates typing from VM; calculator/keypad focus receives PC key events | M3 | R4 |
| 15 | Screenshot PNG Export | Native dialog file export of 320×240 emulated screen to PNG via ppl-graphics | M3 | R4 |
| 16 | Background VM Worker Execution | Isolated non-blocking worker thread executing ppl-vm stack machine | M4 | R3 |
| 17 | Unsaved Buffer Execution | Instant in-memory compilation of active editor buffer without disk write | M4 | R3 |
| 18 | Execution Controls | Run, Stop, Restart directly from UI buttons and hotkeys (F5, Shift+F5, Ctrl+F5) | M4 | R3 |
| 19 | Cooperative Cancellation | Leak-free cancellation via Arc<AtomicBool> and Condvar without zombie threads | M4 | R3 |
| 20 | Dedicated Output Console | Captures PRINT(...) stream with autoscroll, distinct styling from system diagnostics | M4 | R3 |
| 21 | Interactive INPUT Dialog | Native/Grob modal popup for INPUT with field validation and value return | M5 | R5 |
| 22 | Interactive CHOOSE Dialog | Selectable option list popup returning 1-based choice index | M5 | R5 |
| 23 | Interactive MSGBOX Dialog | Modal notification dialog with OK/Cancel return value | M5 | R5 |
| 24 | GETKEY & ISKEYDOWN Dispatch | Non-blocking key query and polling from virtual keypad and PC keyboard | M5 | R5 |
| 25 | Non-Blocking WAIT Builtin | Sleep or key wait with 100ms interrupt check slices, zero UI freezing | M5 | R5 |
| 26 | Gutter Breakpoints | Clickable line number gutter to set/clear breakpoints | M6 | R6 |
| 27 | Statement Stepping Hook | VM Debugger trait intercepting Op::Stmt for Step Into, Step Over, Step Out, Continue | M6 | R6 |
| 28 | Execution Pause Line Highlighting | Editor highlights current paused execution line with pointer indicator | M6 | R6 |
| 29 | Thread-Safe Variable Snapshot Tree | Inspection of local and global variables (scalars, strings, lists, matrices, complex) | M6 | R6 |
| 30 | Call Stack Frame Inspection | Active call frame list displaying function name, line, and scope | M6 | R6 |
| 31 | Full E2E Test Suite Acceptance | 100% pass rate across Tiers 1–4 requirement-driven opaque-box test suite | M7 | AC / Test |
| 32 | Adversarial Coverage Hardening | Tier 5 white-box stress testing and edge-case validation | M7 | Quality Gate |

## Milestones
| # | Name | Scope | Dependencies | Status |
|---|------|-------|-------------|--------|
| M1 | Native Desktop Shell & Aesthetic | Classic engineering theme, menu bar, toolbar, resizable panels, status bar | none | IN_PROGRESS |
| M2 | Multi-Tab PPL Editor & Diagnostics | Multi-tab buffer, PPL syntax highlighting, live compiler diagnostics, rfd file I/O | M1 | PLANNED |
| M3 | Emulated Display & Virtual Keypad | 320×240 GROB display, nearest-neighbor scaling, 0–50 virtual keypad, touch, focus separation, PNG export | M1 | PLANNED |
| M4 | VM Execution Controller & Output | Background worker thread, unsaved buffer run, stop/restart, cooperative interrupt, PRINT output console | M1, M2 | PLANNED |
| M5 | Interactive Host Dialogs & Events | INPUT, CHOOSE, MSGBOX modal dialogs, GETKEY, ISKEYDOWN, non-blocking WAIT | M3, M4 | PLANNED |
| M6 | Graphical Debugger & Variable Inspector| Gutter breakpoints, pause highlight, step into/over/out, VariableSnapshot tree, call stack frames | M2, M4 | PLANNED |
| M7 | Full E2E Test Pass & Adversarial Hardening | Pass 100% E2E test suite (Tiers 1–4) + Tier 5 adversarial hardening | M1–M6, TEST_READY.md | PLANNED |

## Interface Contracts

### Shell ↔ Panels (`crates/ppl-gui/src/ide/`)
```rust
pub trait IdePanel {
    fn render(&mut self, ui: &mut egui::Ui, app_state: &mut IdeAppState);
}
```

### Editor ↔ Diagnostics (`crates/ppl-gui/src/ide/editor.rs`)
```rust
pub struct EditorTab {
    pub path: Option<PathBuf>,
    pub title: String,
    pub content: String,
    pub modified: bool,
    pub diagnostics: Vec<DiagnosticItem>,
    pub breakpoints: BTreeSet<usize>,
}

pub struct DiagnosticItem {
    pub severity: ppl_syntax::diag::Severity,
    pub code: String,
    pub message: String,
    pub line: usize,
    pub col: usize,
    pub span: ppl_syntax::source::Span,
}
```

### Execution Controller ↔ Worker (`crates/ppl-gui/src/ide/controller.rs`)
```rust
pub enum ControllerCommand {
    Run { source: String, entry: Option<String>, debug: bool },
    Stop,
    Pause,
    Continue,
    StepInto,
    StepOver,
    StepOut,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DebugPauseSnapshot {
    pub line: usize,
    pub col: usize,
    pub function: String,
    pub depth: usize,
    pub frames: Vec<StackFrameSnapshot>,
    pub globals: Vec<VariableSnapshot>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VariableSnapshot {
    pub name: String,
    pub type_name: &'static str,
    pub preview: String,
    pub details: VariableDetailSnapshot,
}
```

### Display ↔ Framebuffer (`crates/ppl-gui/src/ide/display.rs`)
```rust
pub struct CalculatorDisplay {
    texture: Option<egui::TextureHandle>,
    last_generation: u64,
}
// Render nearest-neighbor scaled texture from state.screen (Mutex<Grob>)
```

## Code Layout
All new and updated IDE modules are isolated in `crates/ppl-gui`:
```text
crates/ppl-gui/
├── Cargo.toml                  # Add eframe 0.32, egui 0.32, rfd 0.15
├── src/
│   ├── lib.rs                  # Public exports: launch_ide, run_gui, IdeSharedState
│   ├── app.rs                  # eframe::App implementation (IdeDesktopApp)
│   ├── theme.rs                # Classic engineering visual palette (#25292D / #2D3237 / #454B51)
│   ├── ide/
│   │   ├── mod.rs              # IDE state & panel orchestration
│   │   ├── menu.rs             # Main menu bar & keyboard shortcuts
│   │   ├── toolbar.rs          # Action toolbar (New, Open, Save, Run, Debug, etc.)
│   │   ├── explorer.rs         # Left program explorer & outline
│   │   ├── editor.rs           # Multi-tab editor, syntax highlighting, gutter, breakpoints
│   │   ├── syntax.rs           # Token classification & layout job generation from ppl-syntax
│   │   ├── display.rs          # 320×240 GROB display with nearest-neighbor scaling & touch
│   │   ├── keypad.rs           # 0–50 virtual keypad grid & key event dispatch
│   │   ├── console.rs          # Bottom dock output tab (PRINT stream)
│   │   ├── problems.rs         # Bottom dock problems tab with click-to-nav
│   │   ├── debugger.rs         # Debugger panel, call stack, and variable tree inspector
│   │   ├── dialogs.rs          # Native / modal INPUT, CHOOSE, MSGBOX popups
│   │   ├── status.rs           # Bottom status bar (Ln/Col, VM state, metrics)
│   │   └── controller.rs       # Execution controller & worker thread coordination
│   ├── host.rs                 # GuiHost implementation of ppl_vm::Host
│   ├── worker.rs               # Worker thread running ppl_vm::Machine
│   ├── keymap.rs               # HP Prime 0–50 keycode translation
│   ├── scaler.rs               # Software scaling utilities (preserved for headless tests)
│   ├── dialog.rs               # Grob modal dialogs (preserved for headless tests)
│   └── script.rs               # Headless test scripting
```
Existing CLI entry points in `crates/ppl-cli` (`run`, `gui`, `debug`, `check`, `compare`, `device`) remain intact.
`cargo run --bin ppl -- gui [file]` routes directly to `ppl_gui::launch_ide(file)`.
