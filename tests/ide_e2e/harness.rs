//! Test harness and headless programmatic models for the HP Prime Desktop IDE E2E test suite.
//!
//! Provides isolated models for IDE state transitions, compiler diagnostics,
//! execution controller, virtual keypad, display scaling, dialog flows, and
//! headless VM execution.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ppl_compiler::CompileOptions;
use ppl_graphics::{Grob, SCREEN_H, SCREEN_W};
use ppl_gui::host::{GuiHost, GuiSharedState};
use ppl_syntax::diag::Severity;
use ppl_syntax::source::{SourceFile, Span};
use ppl_vm::machine::{DebugAction, Debugger, Limits, Machine};
use ppl_vm::settings::Settings;
use ppl_vm::value::Value;

/// IDE dock tab identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DockTab {
    #[default]
    Output,
    Problems,
    Variables,
    CallStack,
}

#[allow(dead_code)]
impl DockTab {
    pub fn title(&self) -> &'static str {
        match self {
            DockTab::Output => "Output",
            DockTab::Problems => "Problems",
            DockTab::Variables => "Variables",
            DockTab::CallStack => "Call Stack",
        }
    }
}

/// VM execution status in the IDE.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum VmExecutionState {
    #[default]
    Idle,
    Running,
    Paused,
    Terminated,
    Error(String),
}

impl VmExecutionState {
    pub fn label(&self) -> &'static str {
        match self {
            VmExecutionState::Idle => "Idle",
            VmExecutionState::Running => "Running",
            VmExecutionState::Paused => "Paused",
            VmExecutionState::Terminated => "Done",
            VmExecutionState::Error(_) => "Error",
        }
    }
}

/// User actions dispatched via menu, toolbar, or shortcuts.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdeAction {
    NewProgram,
    OpenFile(Option<PathBuf>),
    SaveActiveFile,
    SaveActiveFileAs,
    CloseActiveTab,
    RunProgram,
    StopProgram,
    RestartProgram,
    StartDebugging,
    ContinueExecution,
    StepInto,
    StepOver,
    StepOut,
    ToggleBreakpoint(usize),
    CheckSyntax,
    FormatSource,
    ToggleExplorer,
    ToggleCalculator,
    ToggleBottomDock,
    ToggleStatusBar,
    ResetLayout,
    ShowAboutDialog,
    ShowSettingsDialog,
    ShowNewProgramDialog,
    ShowGoToLineDialog,
    TakeScreenshot,
}

/// Representation of an open editor tab.
#[derive(Debug, Clone, PartialEq)]
pub struct EditorTab {
    pub title: String,
    pub path: Option<PathBuf>,
    pub content: String,
    pub modified: bool,
    pub cursor_line: usize,
    pub cursor_col: usize,
    pub breakpoints: BTreeSet<usize>,
    pub diagnostics: Vec<DiagnosticItem>,
}

impl EditorTab {
    pub fn new(title: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            path: None,
            content: content.into(),
            modified: false,
            cursor_line: 1,
            cursor_col: 1,
            breakpoints: BTreeSet::new(),
            diagnostics: Vec::new(),
        }
    }

    pub fn set_content(&mut self, new_content: impl Into<String>) {
        self.content = new_content.into();
        self.modified = true;
    }

    pub fn toggle_breakpoint(&mut self, line: usize) -> bool {
        if self.breakpoints.contains(&line) {
            self.breakpoints.remove(&line);
            false
        } else {
            self.breakpoints.insert(line);
            true
        }
    }
}

/// Clickable compiler diagnostic item in editor and problems tab.
#[derive(Debug, Clone, PartialEq)]
pub struct DiagnosticItem {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    pub line: usize,
    pub col: usize,
    pub span: Span,
}

/// Thread-safe variable snapshot for debugger tree inspection.
#[derive(Clone, Debug, PartialEq)]
pub struct VariableSnapshot {
    pub name: String,
    pub type_name: &'static str,
    pub preview: String,
    pub details: VariableDetailSnapshot,
}

#[derive(Clone, Debug, PartialEq)]
pub enum VariableDetailSnapshot {
    Scalar,
    StringValue(String),
    List(Vec<String>),
    Matrix {
        rows: usize,
        cols: usize,
        elements: Vec<String>,
    },
    Complex {
        re: String,
        im: String,
    },
    GrobPreview {
        width: usize,
        height: usize,
    },
}

impl VariableSnapshot {
    pub fn from_value(name: impl Into<String>, val: &Value) -> Self {
        let name = name.into();
        let type_name = val.type_name();
        let preview = format!("{val:?}");
        let details = match val {
            Value::Str(s) => VariableDetailSnapshot::StringValue((**s).to_string()),
            Value::List(l) => {
                let elems = l.iter().take(50).map(|v| format!("{v:?}")).collect();
                VariableDetailSnapshot::List(elems)
            }
            Value::Matrix(m) => {
                let elements = m.data.iter().take(100).map(|v| format!("{v:?}")).collect();
                VariableDetailSnapshot::Matrix {
                    rows: m.rows,
                    cols: m.cols,
                    elements,
                }
            }
            Value::Complex(c) => VariableDetailSnapshot::Complex {
                re: format!("{:?}", c.re),
                im: format!("{:?}", c.im),
            },
            Value::Grob(_) => VariableDetailSnapshot::GrobPreview {
                width: SCREEN_W,
                height: SCREEN_H,
            },
            _ => VariableDetailSnapshot::Scalar,
        };

        Self {
            name,
            type_name,
            preview,
            details,
        }
    }
}

/// Snapshot of a single call stack frame.
#[derive(Clone, Debug, PartialEq)]
pub struct StackFrameSnapshot {
    pub function: String,
    pub line: usize,
    pub col: usize,
    pub locals: Vec<VariableSnapshot>,
}

/// Snapshot of paused execution state.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct DebugPauseSnapshot {
    pub line: usize,
    pub col: usize,
    pub function: String,
    pub depth: usize,
    pub frames: Vec<StackFrameSnapshot>,
    pub globals: Vec<VariableSnapshot>,
}

#[allow(dead_code)]
pub struct IdeAppStateModel {
    pub show_explorer: bool,
    pub show_calculator: bool,
    pub show_bottom_dock: bool,
    pub show_status_bar: bool,
    pub active_dock_tab: DockTab,

    pub explorer_width: f32,
    pub calculator_width: f32,
    pub bottom_height: f32,

    pub tabs: Vec<EditorTab>,
    pub active_tab_index: usize,
    pub recent_files: Vec<PathBuf>,

    pub vm_state: VmExecutionState,
    pub step_count: u64,
    pub elapsed_time: Duration,
    pub status_message: String,
    pub pending_actions: Vec<IdeAction>,

    pub shared_state: Arc<GuiSharedState>,
}

impl std::fmt::Debug for IdeAppStateModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IdeAppStateModel")
            .field("show_explorer", &self.show_explorer)
            .field("show_calculator", &self.show_calculator)
            .field("show_bottom_dock", &self.show_bottom_dock)
            .field("show_status_bar", &self.show_status_bar)
            .field("active_dock_tab", &self.active_dock_tab)
            .field("tabs", &self.tabs)
            .field("active_tab_index", &self.active_tab_index)
            .field("vm_state", &self.vm_state)
            .field("step_count", &self.step_count)
            .field("status_message", &self.status_message)
            .finish()
    }
}

impl Default for IdeAppStateModel {
    fn default() -> Self {
        Self {
            show_explorer: true,
            show_calculator: true,
            show_bottom_dock: true,
            show_status_bar: true,
            active_dock_tab: DockTab::Output,
            explorer_width: 240.0,
            calculator_width: 360.0,
            bottom_height: 180.0,
            tabs: vec![EditorTab::new(
                "UNTITLED.hpprgm",
                default_ppl_template("UNTITLED"),
            )],
            active_tab_index: 0,
            recent_files: Vec::new(),
            vm_state: VmExecutionState::Idle,
            step_count: 0,
            elapsed_time: Duration::ZERO,
            status_message: "Ready".to_string(),
            pending_actions: Vec::new(),
            shared_state: Arc::new(GuiSharedState::new()),
        }
    }
}

impl IdeAppStateModel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn active_tab(&self) -> Option<&EditorTab> {
        self.tabs.get(self.active_tab_index)
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut EditorTab> {
        self.tabs.get_mut(self.active_tab_index)
    }

    pub fn open_tab(&mut self, title: impl Into<String>, content: impl Into<String>) -> usize {
        let tab = EditorTab::new(title, content);
        self.tabs.push(tab);
        self.active_tab_index = self.tabs.len() - 1;
        self.active_tab_index
    }

    pub fn close_tab(&mut self, index: usize) -> bool {
        if index < self.tabs.len() {
            self.tabs.remove(index);
            if self.active_tab_index >= self.tabs.len() && !self.tabs.is_empty() {
                self.active_tab_index = self.tabs.len() - 1;
            }
            true
        } else {
            false
        }
    }

    pub fn reset_layout(&mut self) {
        self.show_explorer = true;
        self.show_calculator = true;
        self.show_bottom_dock = true;
        self.show_status_bar = true;
        self.explorer_width = 240.0;
        self.calculator_width = 360.0;
        self.bottom_height = 180.0;
    }

    pub fn queue_action(&mut self, action: IdeAction) {
        self.pending_actions.push(action);
    }

    pub fn handle_action(&mut self, action: IdeAction) {
        match action {
            IdeAction::RunProgram => {
                self.vm_state = VmExecutionState::Running;
            }
            IdeAction::StopProgram => {
                self.vm_state = VmExecutionState::Terminated;
            }
            IdeAction::ResetLayout => {
                self.reset_layout();
            }
            other => {
                self.queue_action(other);
            }
        }
    }

    pub fn take_actions(&mut self) -> Vec<IdeAction> {
        std::mem::take(&mut self.pending_actions)
    }

    pub fn add_recent_file(&mut self, path: PathBuf) {
        self.recent_files.retain(|p| p != &path);
        self.recent_files.insert(0, path);
        if self.recent_files.len() > 10 {
            self.recent_files.truncate(10);
        }
    }
}

/// Returns default PPL program template.
pub fn default_ppl_template(name: &str) -> String {
    format!("EXPORT {name}()\nBEGIN\n  \nEND;\n")
}

/// Result of running a PPL program headlessly.
#[derive(Debug)]
pub struct HeadlessRunResult {
    pub success: bool,
    pub terminal_lines: Vec<String>,
    pub screen: Grob,
    pub error: Option<String>,
}

/// Runs PPL source headlessly with supplied inputs and keys.
pub fn run_ppl_headless(source: &str, _inputs: &[&str], keys: &[i64]) -> HeadlessRunResult {
    let file = SourceFile::new("<headless>", source);
    let (prog, diags) = ppl_compiler::compile_source(&file, &CompileOptions::default());
    if diags.has_errors() || prog.is_none() {
        let errs: Vec<_> = diags.items.iter().map(|d| d.message.clone()).collect();
        eprintln!("COMPILE ERROR: {:?}", errs);
        return HeadlessRunResult {
            success: false,
            terminal_lines: Vec::new(),
            screen: Grob::screen(),
            error: Some(errs.join("; ")),
        };
    }

    let prog = prog.unwrap();
    let shared_state = Arc::new(GuiSharedState::new());
    for &k in keys {
        shared_state.push_key(k);
    }

    let host = GuiHost::new(Arc::clone(&shared_state));
    let mut machine = Machine::new(
        Rc::new(prog.clone()),
        Box::new(host),
        Settings::default(),
        Limits::default(),
    );
    let entry = prog
        .exported()
        .next()
        .map(|(_, f)| f.name.clone())
        .or_else(|| prog.functions.first().map(|f| f.name.clone()))
        .unwrap_or_else(|| "EXPORT".to_string());
    let res = machine.call_function(&entry, Vec::new());

    let (success, err_msg) = match res {
        Ok(_) => (true, None),
        Err(e) => (false, Some(e.to_string())),
    };

    let terminal_lines = shared_state.terminal_lines.lock().unwrap().clone();
    let screen = shared_state.get_frame();

    HeadlessRunResult {
        success,
        terminal_lines,
        screen,
        error: err_msg,
    }
}

/// Thread-safe shared pause collector for debugger tests.
pub struct SharedCapturingDebugger {
    pub breakpoints: BTreeSet<usize>,
    pub pauses: Arc<Mutex<Vec<DebugPauseSnapshot>>>,
    pub max_pauses: usize,
    pub step_into: bool,
    source_file: SourceFile,
}

impl SharedCapturingDebugger {
    pub fn new(
        source: &str,
        breakpoints: BTreeSet<usize>,
        pauses: Arc<Mutex<Vec<DebugPauseSnapshot>>>,
        max_pauses: usize,
        step_into: bool,
    ) -> Self {
        Self {
            breakpoints,
            pauses,
            max_pauses,
            step_into,
            source_file: SourceFile::new("<debug>", source),
        }
    }
}

impl Debugger for SharedCapturingDebugger {
    fn on_statement(
        &mut self,
        machine: &Machine,
        function: &str,
        span: Span,
        depth: usize,
    ) -> DebugAction {
        let lc = self.source_file.line_col(span.start);
        let line = lc.line as usize;
        let col = lc.col as usize;

        let should_pause = self.step_into || self.breakpoints.contains(&line);
        if should_pause {
            let mut list = self.pauses.lock().unwrap();
            if list.len() < self.max_pauses {
                let frames = machine
                    .call_stack()
                    .into_iter()
                    .enumerate()
                    .map(|(i, (fn_name, s))| {
                        let flc = self.source_file.line_col(s.start);
                        let locals = machine
                            .locals(i)
                            .into_iter()
                            .map(|(n, v)| VariableSnapshot::from_value(n, &v))
                            .collect();
                        StackFrameSnapshot {
                            function: fn_name,
                            line: flc.line as usize,
                            col: flc.col as usize,
                            locals,
                        }
                    })
                    .collect();

                let globals = machine
                    .globals()
                    .into_iter()
                    .map(|(n, v)| VariableSnapshot::from_value(n, &v))
                    .collect();

                list.push(DebugPauseSnapshot {
                    line,
                    col,
                    function: function.to_string(),
                    depth,
                    frames,
                    globals,
                });
            }
        }

        DebugAction::Continue
    }
}

pub fn execute_with_debugger(
    source: &str,
    breakpoints: BTreeSet<usize>,
    max_pauses: usize,
    step_into: bool,
) -> Vec<DebugPauseSnapshot> {
    let file = SourceFile::new("<debug>", source);
    let (prog, diags) = ppl_compiler::compile_source(&file, &CompileOptions::default());
    if diags.has_errors() || prog.is_none() {
        return Vec::new();
    }

    let prog = prog.unwrap();
    let shared_state = Arc::new(GuiSharedState::new());
    let host = GuiHost::new(Arc::clone(&shared_state));
    let pauses = Arc::new(Mutex::new(Vec::new()));
    let dbg = SharedCapturingDebugger::new(
        source,
        breakpoints,
        Arc::clone(&pauses),
        max_pauses,
        step_into,
    );

    let mut machine = Machine::new(
        Rc::new(prog.clone()),
        Box::new(host),
        Settings::default(),
        Limits::default(),
    );
    machine.set_debugger(Some(Box::new(dbg)));
    let entry = prog
        .exported()
        .next()
        .map(|(_, f)| f.name.clone())
        .or_else(|| prog.functions.first().map(|f| f.name.clone()))
        .unwrap_or_else(|| "EXPORT".to_string());
    let _ = machine.call_function(&entry, Vec::new());

    let result = pauses.lock().unwrap().clone();
    result
}
