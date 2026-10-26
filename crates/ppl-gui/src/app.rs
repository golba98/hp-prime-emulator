//! Native desktop IDE built on `eframe`/`egui`.
//!
//! `IdeDesktopApp` wires the editor, diagnostics, virtual calculator,
//! execution controller, and debugger panels into one eframe application.
//! The plain winit calculator window lives in [`crate::window`].

#[path = "app_check.rs"]
mod checking_integration;
#[path = "app_firmware.rs"]
mod firmware_integration;
#[path = "app_workspace.rs"]
mod workspace;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::dialog::KeyPress;
use crate::ide::console::ConsolePanel;
use crate::ide::debugger::DebuggerPanel;
use crate::ide::diagnostics::{self, DiagnosticItem, DiagnosticsWorker};
use crate::ide::dialogs::DialogHost;
use crate::ide::display::CalculatorDisplay;
use crate::ide::editor::{EditorArea, EditorTab, SaveError, TabId};
use crate::ide::execution::ExecutionController;
use crate::ide::execution::WorkerOutcome;
use crate::ide::explorer::ProgramExplorer;
use crate::ide::keypad::VirtualKeypad;
use crate::ide::menu::MenuBar;
use crate::ide::problems::ProblemsPanel;
use crate::ide::status::StatusBar;
use crate::ide::toolbar::ToolBar;
use crate::ide::{AnalysisState, DockTab, IdeAction, IdeAppState, IdePanel};
use crate::theme;

/// Native Desktop IDE Application implementing `eframe::App`.
pub struct IdeDesktopApp {
    pub state: IdeAppState,
    pub firmware: Option<crate::ide::firmware::FirmwareClient>,
    pub firmware_status: String,
    pub program_manager: crate::ide::programs::ProgramManager,
    explicit_check: Option<checking_integration::CheckJob>,
    firmware_problems: Vec<(DiagnosticItem, u64)>,
    native_compile: Option<NativeCompileJob>,
    native_stopping: bool,
    native_restart: Option<egui::Context>,
    firmware_state: crate::ide::VmExecutionState,
    native_vm_state: crate::ide::VmExecutionState,
    firmware_request: u64,
    last_touch: Option<ppl_vm::host::Touch>,
    touch_start_time: Option<std::time::Instant>,
    deferred_touch_end: Option<(ppl_vm::host::Touch, std::time::Instant)>,
    prev_shift_held: bool,
    prev_alpha_held: bool,
    pub menu_bar: MenuBar,
    pub toolbar: ToolBar,
    pub explorer: ProgramExplorer,
    pub editor: EditorArea,
    pub display: CalculatorDisplay,
    pub keypad: VirtualKeypad,
    pub console: ConsolePanel,
    pub problems: ProblemsPanel,
    pub debugger: DebuggerPanel,
    pub status_bar: StatusBar,
    pub dialogs: DialogHost,
    pub controller: ExecutionController,
    /// Optional scripted automation (see `ide::automation`).
    pub automation: Option<crate::ide::automation::Automation>,
    /// Background compiler for live diagnostics.
    diagnostics: DiagnosticsWorker,
    /// Live-diagnostic bookkeeping per tab.
    diag_requests: HashMap<TabId, DiagRequest>,
    /// Runtime problems from the last run, with the source revision that
    /// ran; shown only while the tab still has that revision.
    runtime_problems: Vec<(DiagnosticItem, u64)>,
    /// The program currently (or last) running.
    running: Option<RunningProgram>,
    /// Tabs and revisions the Problems panel was last built from.
    problems_key: Vec<(TabId, u64)>,
    /// (tab, revision) the outline was last built from.
    outline_key: Option<(TabId, u64)>,
    /// Last debugger pause generation seen by the UI.
    last_pause_gen: u64,
    /// Set once the user confirmed quitting with unsaved programs.
    pub(crate) close_confirmed: bool,
    /// Last calculator input focus state, for focus transition detection.
    pub(crate) last_calc_input_focus: bool,
}

struct NativeCompileJob {
    tab: TabId,
    revision: u64,
    file: ppl_syntax::SourceFile,
    run_entry: Option<String>,
    run_args: Option<String>,
    max_steps: Option<u64>,
    debug: bool,
    stop_at_entry: bool,
    run: bool,
    result: std::sync::mpsc::Receiver<diagnostics::Analysis>,
}

/// Quiet period after an edit before live diagnostics recompile.
const DIAGNOSTICS_DEBOUNCE: Duration = Duration::from_millis(250);

/// The exact source a run was compiled from.
#[derive(Debug, Clone)]
struct RunningProgram {
    tab: TabId,
    revision: u64,
    file: ppl_syntax::SourceFile,
}

#[derive(Debug, Clone, Copy)]
struct DiagRequest {
    /// Revision last sent to the worker (0: never).
    requested: u64,
    /// Revision last observed, and when it was first seen.
    seen: u64,
    seen_at: Instant,
}

impl Default for IdeDesktopApp {
    fn default() -> Self {
        Self {
            state: IdeAppState::default(),
            firmware: None,
            firmware_status: "Firmware not started".into(),
            program_manager: Default::default(),
            explicit_check: None,
            firmware_problems: Vec::new(),
            native_compile: None,
            native_stopping: false,
            native_restart: None,
            firmware_state: crate::ide::VmExecutionState::Idle,
            native_vm_state: crate::ide::VmExecutionState::Idle,
            firmware_request: 0,
            last_touch: None,
            touch_start_time: None,
            deferred_touch_end: None,
            prev_shift_held: false,
            prev_alpha_held: false,
            menu_bar: MenuBar::new(),
            toolbar: ToolBar::new(),
            explorer: ProgramExplorer::new(),
            editor: EditorArea::new(),
            display: CalculatorDisplay::new(),
            keypad: VirtualKeypad::new(),
            console: ConsolePanel::new(),
            problems: ProblemsPanel::new(),
            debugger: DebuggerPanel::new(),
            status_bar: StatusBar::new(),
            dialogs: DialogHost::new(),
            controller: ExecutionController::new(),
            automation: None,
            diagnostics: DiagnosticsWorker::new(None),
            diag_requests: HashMap::new(),
            runtime_problems: Vec::new(),
            running: None,
            problems_key: Vec::new(),
            outline_key: None,
            last_pause_gen: 0,
            close_confirmed: false,
            last_calc_input_focus: false,
        }
    }
}

pub fn default_layout_config_path() -> Option<PathBuf> {
    if let Ok(config_home) = std::env::var("XDG_CONFIG_HOME") {
        Some(
            PathBuf::from(config_home)
                .join("hp-prime-emulator")
                .join("layout.json"),
        )
    } else if let Ok(home) = std::env::var("HOME") {
        Some(
            PathBuf::from(home)
                .join(".config")
                .join("hp-prime-emulator")
                .join("layout.json"),
        )
    } else {
        None
    }
}

/// Maps an egui key to an HP Prime keycode (0–50 grid).
pub(crate) fn egui_key_to_hp(key: egui::Key) -> Option<i64> {
    crate::keymap::egui_key_to_hp(key)
}

impl IdeDesktopApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply_theme(&cc.egui_ctx);
        let mut app = Self::default();
        let mut loaded = false;
        if let Some(storage) = cc.storage {
            if let Some(json) = storage.get_string("ide_panel_state") {
                if let Ok(p) = serde_json::from_str::<crate::ide::PersistentPanelState>(&json) {
                    app.state.apply_persistent(&p);
                    loaded = true;
                }
            }
        }
        if !loaded {
            if let Some(path) = default_layout_config_path() {
                if let Ok(content) = std::fs::read_to_string(path) {
                    if let Ok(p) =
                        serde_json::from_str::<crate::ide::PersistentPanelState>(&content)
                    {
                        app.state.apply_persistent(&p);
                    }
                }
            }
        }
        // VM screen updates must repaint the window.
        let ctx = cc.egui_ctx.clone();
        app.controller
            .shared
            .set_redraw_callback(move || ctx.request_repaint());
        let ctx = cc.egui_ctx.clone();
        app.controller
            .bridge
            .set_wake_ui(move || ctx.request_repaint());
        // Diagnostics results arrive from a worker thread; repaint to show them.
        let ctx = cc.egui_ctx.clone();
        app.diagnostics = DiagnosticsWorker::new(Some(Box::new(move || ctx.request_repaint())));
        app
    }

    pub fn execution_busy(&self) -> bool {
        if self.state.firmware_backend {
            matches!(self.firmware_state, crate::ide::VmExecutionState::Running)
        } else {
            self.controller.worker_alive() || self.native_compile.is_some() || self.native_stopping
        }
    }

    pub fn open_file(&mut self, path: PathBuf) {
        // Decodes UTF-8 / UTF-16 / containers and remembers the format, so
        // saving writes the file back the way it was.
        if let Ok(tab) = EditorTab::open(path.clone()) {
            self.editor.open_tab(tab);
            self.state.recent_files.retain(|p| p != &path);
            self.state.recent_files.insert(0, path.clone());
            if self.state.recent_files.len() > 10 {
                self.state.recent_files.truncate(10);
            }
            self.state.status_message = format!(
                "Opened {}",
                path.file_name().and_then(|n| n.to_str()).unwrap_or("file")
            );
        } else {
            self.state.status_message = format!("Could not open {}", path.display());
        }
    }

    /// Saves the active tab (optionally to `path`), without dialogs. The
    /// file keeps its original encoding and line endings.
    pub(crate) fn save_active_tab(&mut self, path: Option<PathBuf>) -> Result<(), SaveError> {
        let Some(tab) = self.editor.active_tab_mut() else {
            return Err(SaveError::NoPath);
        };
        let result = tab.save(path);
        self.state.status_message = match &result {
            Ok(()) => format!("Saved {}", tab.title),
            Err(e) => format!("Not saved: {e}"),
        };
        result
    }

    /// Asks for a file name and saves the active tab there.
    fn save_active_tab_as(&mut self) {
        let suggested = self
            .editor
            .active_tab()
            .map(|t| t.title.clone())
            .unwrap_or_else(|| "Untitled.ppl".into());
        if let Some(file) = rfd::FileDialog::new()
            .add_filter("PPL Program", &["hpprgm", "ppl", "txt"])
            .set_file_name(suggested)
            .save_file()
        {
            let _ = self.save_active_tab(Some(file));
        }
    }

    /// Populates the run configuration dialog from the active buffer's
    /// exported functions and opens it.
    fn open_run_config(&mut self) {
        let Some(tab) = self.editor.active_tab_mut() else {
            self.state.status_message = "No program open to configure".to_string();
            return;
        };
        tab.sync();
        let entries = tab
            .doc
            .analysis()
            .symbols
            .functions
            .iter()
            .filter(|f| f.exported)
            .map(|f| (f.name.clone(), f.params.len()))
            .collect::<Vec<_>>();
        self.dialogs.run_config_selected = entries
            .iter()
            .position(|(name, _)| Some(name) == tab.run_entry.as_ref())
            .unwrap_or(0);
        self.dialogs.run_config_args = tab.run_args.clone();
        self.dialogs.run_config_entries = entries;
        self.dialogs.show_run_config = true;
    }

    /// Compiles the active buffer's current text and, only if it compiles,
    /// runs exactly that program on the background worker. The compile
    /// result also becomes the tab's diagnostics for that revision, so the
    /// Problems panel and the executed bytecode always match.
    fn run_source(&mut self, debug: bool, stop_at_entry: bool, ctx: &egui::Context) {
        self.begin_native_compile(true, debug, stop_at_entry, ctx);
    }

    fn begin_native_compile(
        &mut self,
        run: bool,
        debug: bool,
        stop_at_entry: bool,
        ctx: &egui::Context,
    ) {
        if self.native_compile.is_some() || (run && self.controller.worker_alive()) {
            self.state.status_message = "A program is already running (use Stop first)".into();
            return;
        }
        let Some(tab) = self.editor.active_tab_mut() else {
            return;
        };
        let revision = tab.revision();
        let tab_id = tab.id;
        let file = ppl_syntax::SourceFile::new(&tab.title, &tab.content);
        let run_entry = tab.run_entry.clone();
        let run_args = Some(tab.run_args.trim().to_owned()).filter(|a| !a.is_empty());
        self.controller.bridge.set_breakpoints(&tab.breakpoints);
        let (tx, result) = std::sync::mpsc::channel();
        let source = file.clone();
        let wake = ctx.clone();
        std::thread::spawn(move || {
            let analysis = diagnostics::analyze(&source.name, &source.text, revision);
            let _ = tx.send(analysis);
            wake.request_repaint();
        });
        self.native_compile = Some(NativeCompileJob {
            tab: tab_id,
            revision,
            file,
            run_entry,
            run_args,
            max_steps: self
                .dialogs
                .max_steps_input
                .trim()
                .parse::<u64>()
                .ok()
                .filter(|n| *n > 0),
            debug,
            stop_at_entry,
            run,
            result,
        });
        self.mark_requested(tab_id, revision);
        if run {
            self.state.vm_state = crate::ide::VmExecutionState::Running;
        }
        self.state.status_message = "Compiling current source…".into();
    }

    fn poll_native_compile(&mut self) {
        let Some(analysis) = self
            .native_compile
            .as_ref()
            .and_then(|j| j.result.try_recv().ok())
        else {
            return;
        };
        let job = self.native_compile.take().unwrap();
        let NativeCompileJob {
            tab: tab_id,
            revision,
            file,
            run_entry,
            run_args,
            max_steps,
            debug,
            stop_at_entry,
            run,
            ..
        } = job;
        let Some(i) = self.editor.index_of(tab_id) else {
            self.state.vm_state = crate::ide::VmExecutionState::Idle;
            return;
        };
        let tab = &mut self.editor.tabs[i];
        if tab.revision() != revision {
            self.state.vm_state = crate::ide::VmExecutionState::Idle;
            self.state.status_message =
                "Source changed while compiling; run the current revision again".into();
            return;
        }
        if !run {
            Self::set_analysis(tab, revision, analysis.items, analysis.failure);
            self.rebuild_problems();
            self.state.status_message = format!(
                "Compile: {} error(s), {} warning(s)",
                self.problems.error_count(),
                self.problems.warning_count()
            );
            return;
        }
        let errors = analysis.error_count();
        let failure = analysis.failure.clone();
        let program = analysis.program;
        Self::set_analysis(tab, revision, analysis.items, analysis.failure);
        self.mark_requested(tab_id, revision);
        self.runtime_problems.clear();
        self.rebuild_problems();

        let program = match (program, failure) {
            (Some(p), None) => p,
            (_, failure) => {
                // Never fall back to an earlier program: drop the finished
                // worker so its outcome is not reported again.
                let _ = self.controller.join_current();
                self.state.vm_state = crate::ide::VmExecutionState::Idle;
                self.state.step_count = 0;
                self.state.elapsed_time = Duration::ZERO;
                let msg = match failure {
                    Some(f) => format!("Run blocked: {f}"),
                    None => {
                        format!("Run blocked: {errors} compile error(s); fix them and run again")
                    }
                };
                self.console.print_error(&msg);
                self.state.status_message = msg;
                self.state.active_dock_tab = DockTab::Problems;
                self.state.show_bottom_dock = true;
                let first = self.editor.tabs[i]
                    .current_diagnostics()
                    .iter()
                    .find(|d| d.severity == ppl_syntax::Severity::Error)
                    .map(|d| (d.line, d.col));
                if let Some((line, col)) = first {
                    self.editor.tabs[i].goto_line(line, col);
                }
                return;
            }
        };
        let entry = match program.pick_entry(run_entry.as_deref()) {
            Ok(e) => e,
            Err(msg) => {
                self.state.vm_state = crate::ide::VmExecutionState::Idle;
                self.console.print_error(&msg);
                self.state.status_message = msg;
                return;
            }
        };

        let result = self.controller.start_compiled(
            program,
            &file,
            &entry,
            run_args.as_deref(),
            debug,
            stop_at_entry,
            max_steps,
        );
        match result {
            crate::ide::execution::RunStart::Started => {
                self.running = Some(RunningProgram {
                    tab: tab_id,
                    revision,
                    file,
                });
                self.state.vm_state = crate::ide::VmExecutionState::Running;
                self.state.paused_line = 0;
                self.state.status_message = if debug {
                    "Debug session started; paused at first statement".to_string()
                } else {
                    format!("Running {entry}...")
                };
                self.console.print_diagnostic(if debug {
                    "--- Debug session started ---"
                } else {
                    "--- Run started ---"
                });
                self.state.active_dock_tab = DockTab::Output;
                self.state.show_bottom_dock = true;
            }
            crate::ide::execution::RunStart::AlreadyRunning => {
                self.state.status_message = "A program is already running".to_string();
            }
        }
    }

    /// Records that `revision` of `tab` has been (or is being) analysed, so
    /// the background worker does not compile it again.
    fn mark_requested(&mut self, tab: TabId, revision: u64) {
        let req = self.diag_requests.entry(tab).or_insert(DiagRequest {
            requested: 0,
            seen: 0,
            seen_at: Instant::now(),
        });
        req.requested = revision;
        req.seen = revision;
    }

    /// Stores a compiler result for `revision`; ignored unless that is
    /// still the tab's current revision.
    fn set_analysis(
        tab: &mut EditorTab,
        revision: u64,
        mut items: Vec<DiagnosticItem>,
        failure: Option<String>,
    ) -> bool {
        if tab.revision() != revision {
            return false;
        }
        for item in &mut items {
            item.tab = Some(tab.id);
            item.file = tab.title.clone();
        }
        items.sort_by_key(|i| (i.line, i.col));
        tab.analysis = Some(crate::ide::editor::TabAnalysis {
            revision,
            items,
            failure,
        });
        true
    }

    /// The compiler's verdict on tab `i`'s current text.
    pub(crate) fn analysis_state(&mut self, i: usize) -> AnalysisState {
        let tab = &mut self.editor.tabs[i];
        let revision = tab.revision();
        if let Some(a) = tab.current_analysis() {
            if let Some(f) = &a.failure {
                return AnalysisState::AnalysisFailed(f.clone());
            }
            let count = |sev| a.items.iter().filter(|d| d.severity == sev).count();
            let (errors, warnings) = (
                count(ppl_syntax::Severity::Error),
                count(ppl_syntax::Severity::Warning),
            );
            return match (errors, warnings) {
                (0, 0) => AnalysisState::Valid,
                (0, w) => AnalysisState::ValidWithWarnings(w),
                (errors, warnings) => AnalysisState::Invalid { errors, warnings },
            };
        }
        match self.diag_requests.get(&tab.id) {
            Some(r) if r.requested == revision => AnalysisState::Analyzing,
            Some(_) => AnalysisState::Queued,
            None => AnalysisState::NotAnalyzed,
        }
    }

    /// Updates the per-file analysis status (Problems header, status bar).
    fn refresh_analysis_status(&mut self) {
        let mut pending = Vec::new();
        let mut failures = Vec::new();
        for i in 0..self.editor.tabs.len() {
            let state = self.analysis_state(i);
            let title = self.editor.tabs[i].title.clone();
            match &state {
                AnalysisState::AnalysisFailed(f) => failures.push((title, f.clone())),
                s if !s.is_settled() => pending.push(title),
                _ => {}
            }
            if i == self.editor.active_tab_index {
                self.state.analysis = state;
                self.state.encoding_label = self.editor.tabs[i].format.label();
            }
        }
        if self.editor.tabs.is_empty() {
            self.state.analysis = AnalysisState::NotAnalyzed;
        }
        self.problems.pending = pending;
        self.problems.failures = failures;
    }

    /// Rebuilds the Problems list from every open tab's current analysis
    /// plus runtime errors of runs whose source is still unchanged.
    fn rebuild_problems(&mut self) {
        let mut items: Vec<DiagnosticItem> = Vec::new();
        for t in self.editor.tabs.iter_mut() {
            items.extend(t.current_diagnostics().iter().cloned());
        }
        let current: Vec<(TabId, u64)> = self
            .editor
            .tabs
            .iter_mut()
            .map(|t| (t.id, t.revision()))
            .collect();
        items.extend(
            self.runtime_problems
                .iter()
                .filter(|(p, rev)| p.tab.is_none_or(|id| current.contains(&(id, *rev))))
                .map(|(p, _)| p.clone()),
        );
        items.extend(
            self.firmware_problems
                .iter()
                .filter(|(item, rev)| item.tab.is_some_and(|id| current.contains(&(id, *rev))))
                .map(|(item, _)| item.clone()),
        );
        self.problems.set_diagnostics(items);
        self.problems.active_tab_index = self.editor.active_tab_index;
        self.problems_key = current;
        self.refresh_analysis_status();
    }

    /// Runs an Edit-menu command by focusing the editor and replaying the
    /// equivalent input this frame, so menu and keyboard behave identically.
    fn apply_edit_command(&mut self, ctx: &egui::Context, cmd: crate::ide::EditCommand) {
        use crate::ide::EditCommand as E;
        use egui::{Event, Key, Modifiers};
        let Some(tab) = self.editor.active_tab() else {
            return;
        };
        ctx.memory_mut(|m| m.request_focus(crate::ide::editor::view::edit_id(tab.id)));
        let key = |key: Key, modifiers: Modifiers| Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let event = match cmd {
            E::Undo => key(Key::Z, Modifiers::COMMAND),
            E::Redo => key(Key::Y, Modifiers::COMMAND),
            E::Cut => Event::Cut,
            E::Copy => Event::Copy,
            E::SelectAll => key(Key::A, Modifiers::COMMAND),
            E::Indent => key(Key::Tab, Modifiers::NONE),
            E::Outdent => key(Key::Tab, Modifiers::SHIFT),
            E::ToggleComment => key(Key::Slash, Modifiers::COMMAND),
            E::TriggerCompletion => key(Key::Space, Modifiers::COMMAND),
            E::Paste => {
                // The platform delivers the clipboard as an Event::Paste.
                ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                return;
            }
        };
        ctx.input_mut(|i| i.events.push(event));
    }

    /// Live diagnostics: applies finished checks and queues a check of the
    /// active buffer once it has been unchanged for a short quiet period.
    fn schedule_diagnostics(&mut self, ctx: &egui::Context) {
        let mut changed = false;
        for result in self.diagnostics.poll() {
            // Results for closed tabs or older revisions are superseded.
            if let Some(i) = self.editor.index_of(result.tab) {
                changed |= Self::set_analysis(
                    &mut self.editor.tabs[i],
                    result.revision,
                    result.items,
                    result.failure,
                );
            }
        }

        // Every open tab is kept checked, so Problems covers all of them.
        // The tab being edited waits for a quiet period; others (and tabs
        // never checked) are queued at once.
        let active = self.editor.active_tab_index;
        let now = Instant::now();
        for (i, tab) in self.editor.tabs.iter_mut().enumerate() {
            let revision = tab.revision();
            let req = self.diag_requests.entry(tab.id).or_insert(DiagRequest {
                requested: 0,
                seen: 0,
                seen_at: now,
            });
            if req.requested == revision {
                continue;
            }
            if req.seen != revision {
                req.seen = revision;
                req.seen_at = now;
            }
            let waited = now.duration_since(req.seen_at);
            if i != active || req.requested == 0 || waited >= DIAGNOSTICS_DEBOUNCE {
                self.diagnostics
                    .request(tab.id, revision, &tab.title, &tab.content);
                req.requested = revision;
            } else {
                ctx.request_repaint_after(DIAGNOSTICS_DEBOUNCE - waited);
            }
        }

        // Closed tabs drop out of the list; any edit hides the edited tab's
        // old diagnostics at once (they no longer match its text).
        let key: Vec<(TabId, u64)> = self
            .editor
            .tabs
            .iter_mut()
            .map(|t| (t.id, t.revision()))
            .collect();
        self.diag_requests
            .retain(|id, _| key.iter().any(|(k, _)| k == id));
        if changed || key != self.problems_key {
            self.rebuild_problems();
        } else {
            self.refresh_analysis_status();
        }
    }

    /// Drains worker terminal output, outcomes, and debugger pauses into the UI.
    fn poll_execution(&mut self) {
        if self.native_stopping {
            if self.controller.worker_alive() {
                return;
            }
            let _ = self.controller.join_current();
            self.native_stopping = false;
            if let Some(ctx) = self.native_restart.take() {
                self.begin_native_compile(true, false, false, &ctx);
            }
        }
        self.poll_native_compile();
        if self.native_compile.as_ref().is_some_and(|j| j.run) {
            return;
        }
        // Terminal output -> console.
        for line in self.controller.drain_terminal() {
            self.console.print_output(line);
        }

        // Worker outcome.
        if self.controller.worker_alive() {
            // Still executing.
        } else if let Some(outcome) = self.controller.outcome() {
            if !matches!(
                self.state.vm_state,
                crate::ide::VmExecutionState::Terminated | crate::ide::VmExecutionState::Error(_)
            ) {
                match outcome {
                    WorkerOutcome::Success {
                        value_str,
                        steps,
                        elapsed,
                    } => {
                        self.state.vm_state = crate::ide::VmExecutionState::Terminated;
                        self.state.step_count = steps;
                        self.state.elapsed_time = elapsed;
                        self.console.print_diagnostic(format!(
                            "--- Program finished: returned {value_str} ({steps} steps) ---"
                        ));
                        self.state.status_message = "Program finished".to_string();
                    }
                    WorkerOutcome::Error(e) => {
                        let msg = e.to_string();
                        if let Some(run) = &self.running {
                            let mut item = DiagnosticItem::runtime(&e, &run.file);
                            item.tab = Some(run.tab);
                            if let Some(i) = self.editor.index_of(run.tab) {
                                item.file = self.editor.tabs[i].title.clone();
                            }
                            self.runtime_problems = vec![(item, run.revision)];
                            self.rebuild_problems();
                        }
                        self.state.vm_state = crate::ide::VmExecutionState::Error(msg.clone());
                        self.console.print_error(&msg);
                        self.state.status_message = "Runtime error".to_string();
                        self.state.active_dock_tab = DockTab::Problems;
                    }
                }
            }
        } else if !matches!(
            self.state.vm_state,
            crate::ide::VmExecutionState::Idle | crate::ide::VmExecutionState::Error(_)
        ) {
            // Stopped / cancelled without an outcome.
            self.state.vm_state = crate::ide::VmExecutionState::Idle;
            self.state.paused_line = 0;
        }

        // Debugger pauses.
        let pause_gen = self
            .controller
            .bridge
            .pause_generation
            .load(std::sync::atomic::Ordering::SeqCst);
        let paused_line = self.controller.paused_line();
        if pause_gen != self.last_pause_gen {
            self.last_pause_gen = pause_gen;
            if paused_line != 0 {
                self.debugger
                    .set_snapshot(self.controller.bridge.take_pause_snapshot());
                self.state.paused_line = paused_line;
                self.state.vm_state = crate::ide::VmExecutionState::Paused;
                self.state.status_message = format!("Paused at line {paused_line}");
                self.state.active_dock_tab = DockTab::Variables;
                self.state.show_bottom_dock = true;
            }
        } else if paused_line == 0
            && matches!(self.state.vm_state, crate::ide::VmExecutionState::Paused)
        {
            // Resumed.
            self.state.vm_state = crate::ide::VmExecutionState::Running;
            self.state.paused_line = 0;
            self.debugger.set_snapshot(None);
            self.state.status_message = "Continuing...".to_string();
        }
    }

    /// Updates the calculator display texture from the VM framebuffer.
    fn update_display_texture(&mut self, ctx: &egui::Context) {
        self.display.set_smooth(self.state.smooth_display);
        if self.state.firmware_backend {
            if let Some(frame) = self
                .firmware
                .as_ref()
                .and_then(|f| f.latest.lock().unwrap().clone())
            {
                self.display
                    .update_xrgb(ctx, &frame.pixels, frame.generation);
            }
            return;
        }
        self.display.set_smooth(self.state.smooth_display);
        let screen_gen = self
            .controller
            .shared
            .screen_generation
            .load(std::sync::atomic::Ordering::SeqCst);
        if self.display.last_generation == screen_gen && self.display.texture.is_some() {
            return;
        }
        let grob = self
            .controller
            .shared
            .screen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        self.display.update_from_grob(ctx, &grob, screen_gen);
    }

    pub fn handle_action(&mut self, action: IdeAction, _ctx: &egui::Context) {
        match action {
            IdeAction::NewProgram => {
                let raw_name = self.dialogs.new_prog_name.trim();
                let prog_name = if raw_name.is_empty() {
                    "UNTITLED".to_string()
                } else {
                    raw_name.to_string()
                };
                let export_name = prog_name
                    .strip_suffix(".ppl")
                    .or_else(|| prog_name.strip_suffix(".hpprgm"))
                    .unwrap_or(&prog_name)
                    .to_string();
                let file_name = if prog_name.ends_with(".ppl") || prog_name.ends_with(".hpprgm") {
                    prog_name.clone()
                } else {
                    format!("{prog_name}.ppl")
                };
                let template = format!("EXPORT {export_name}()\nBEGIN\n  \nEND;\n");
                self.editor.new_tab(file_name, template);
                self.state.status_message = format!("Created new program {export_name}");
                self.dialogs.new_prog_name = "MYPROG".to_string();
            }
            IdeAction::OpenFile(path_opt) => {
                if let Some(path) = path_opt {
                    self.open_file(path);
                } else if let Some(file) = rfd::FileDialog::new()
                    .add_filter("PPL Program", &["hpprgm", "ppl", "txt"])
                    .pick_file()
                {
                    self.open_file(file);
                }
            }
            IdeAction::SaveActiveFile => {
                // Untitled programs and binary containers need a new name.
                if let Err(SaveError::NoPath | SaveError::Container) = self.save_active_tab(None) {
                    self.save_active_tab_as();
                }
            }
            IdeAction::SaveActiveFileAs => self.save_active_tab_as(),
            IdeAction::CloseActiveTab => {
                if self.editor.active_tab_modified() {
                    self.dialogs.show_confirm_close = true;
                } else {
                    self.editor.close_tab(self.editor.active_tab_index);
                }
            }
            IdeAction::RunProgram if self.state.firmware_backend => {
                self.request_firmware(true, _ctx)
            }
            IdeAction::ManagePrograms => {
                self.program_manager.open = true;
            }
            IdeAction::CheckSyntax if self.state.firmware_backend => {
                self.request_checked_source(_ctx);
                self.state.active_dock_tab = DockTab::Problems;
                self.state.show_bottom_dock = true;
            }
            IdeAction::StartDebugging
            | IdeAction::StepInto
            | IdeAction::StepOver
            | IdeAction::StepOut
            | IdeAction::ContinueExecution
                if self.state.firmware_backend =>
            {
                self.state.status_message =
                    "Firmware stepping is not supported; select Native PPL to debug".into();
            }
            IdeAction::StopProgram if self.state.firmware_backend => {
                if let Some(f) = &self.firmware {
                    f.send(ppl_engine::session::Command::Stop);
                }
            }
            IdeAction::RestartProgram if self.state.firmware_backend => {
                self.request_firmware(true, _ctx)
            }
            IdeAction::RunProgram => {
                if self.state.vm_state == crate::ide::VmExecutionState::Paused {
                    self.handle_action(IdeAction::ContinueExecution, _ctx);
                } else {
                    self.run_source(false, false, _ctx);
                }
            }
            IdeAction::StopProgram => {
                self.native_compile = None;
                self.native_restart = None;
                self.native_stopping = true;
                self.controller.stop();
                self.state.vm_state = crate::ide::VmExecutionState::Idle;
                self.state.paused_line = 0;
                self.debugger.set_snapshot(None);
                self.console
                    .print_diagnostic("--- Execution stopped by user ---");
                self.state.status_message = "Stopped".into();
            }
            IdeAction::RestartProgram => {
                self.native_compile = None;
                self.native_restart = Some(_ctx.clone());
                self.native_stopping = true;
                self.controller.stop();
                self.state.vm_state = crate::ide::VmExecutionState::Running;
                self.state.paused_line = 0;
                self.debugger.set_snapshot(None);
                self.state.status_message = "Restarting…".into();
            }
            IdeAction::StartDebugging => {
                self.run_source(true, true, _ctx);
            }
            IdeAction::ContinueExecution => {
                if self.controller.paused() {
                    self.controller
                        .bridge
                        .post_command(crate::ide::execution::DebugCommand::Continue);
                } else {
                    self.state.status_message = "Not paused".to_string();
                }
            }
            IdeAction::StepInto => {
                self.controller
                    .bridge
                    .post_command(crate::ide::execution::DebugCommand::StepInto);
            }
            IdeAction::StepOver => {
                self.controller
                    .bridge
                    .post_command(crate::ide::execution::DebugCommand::StepOver);
            }
            IdeAction::StepOut => {
                self.controller
                    .bridge
                    .post_command(crate::ide::execution::DebugCommand::StepOut);
            }
            IdeAction::ToggleBreakpoint(line) => {
                if let Some(tab) = self.editor.active_tab_mut() {
                    if !tab.breakpoints.remove(&line) {
                        tab.breakpoints.insert(line);
                        self.state.status_message = format!("Breakpoint set at line {line}");
                    } else {
                        self.state.status_message = format!("Breakpoint removed at line {line}");
                    }
                    self.controller.bridge.set_breakpoints(&tab.breakpoints);
                }
            }
            IdeAction::CheckSyntax => {
                self.request_checked_source(_ctx);
                self.begin_native_compile(false, false, false, _ctx);
                self.state.active_dock_tab = DockTab::Problems;
                self.state.show_bottom_dock = true;
                self.state.status_message = "Compiling current source…".into();
            }
            IdeAction::FormatSource => {
                self.state.status_message =
                    "Source formatting is not available in this version".to_string();
            }
            IdeAction::ToggleExplorer => {
                self.state.show_explorer = !self.state.show_explorer;
            }
            IdeAction::ToggleCalculator => {
                self.state.show_calculator = !self.state.show_calculator;
            }
            IdeAction::ToggleBottomDock => {
                self.state.show_bottom_dock = !self.state.show_bottom_dock;
            }
            IdeAction::ToggleStatusBar => {
                self.state.show_status_bar = !self.state.show_status_bar;
            }
            IdeAction::SetLayout(mode) => {
                self.keypad.release(&mut self.state);
                self.controller.shared.clear_held_keys();
                if let Some(f) = &self.firmware {
                    f.send(ppl_engine::session::Command::ReleaseAll);
                }
                self.state.clear_held_keys();
                if self.state.layout != mode {
                    if self.state.layout != crate::ide::layout::WorkspaceLayout::Calculator {
                        self.state.previous_layout = self.state.layout;
                    }
                    self.state.layout = mode;
                }
                self.state.calc_input_focus = false;
            }
            IdeAction::SetCalculatorMode(mode) => {
                self.keypad.release(&mut self.state);
                self.state.calculator_mode = mode;
                self.state.show_calculator = true;
            }
            IdeAction::SelectBackend(firmware) => {
                if !self.state.firmware_backend {
                    self.native_vm_state = self.state.vm_state.clone();
                }
                self.controller.shared.clear_held_keys();
                self.state.clear_held_keys();
                if let Some(f) = &self.firmware {
                    f.send(ppl_engine::session::Command::ReleaseAll);
                }
                self.state.firmware_backend = firmware;
                self.state.vm_state = if firmware {
                    self.firmware_state.clone()
                } else {
                    self.native_vm_state.clone()
                };
                self.display.texture = None;
                self.last_touch = None;
                if firmware {
                    self.ensure_firmware(_ctx);
                }
                self.display.last_generation = u64::MAX;
            }
            IdeAction::ShowShortcuts => {
                self.dialogs.show_shortcuts = true;
            }
            IdeAction::ResetLayout => {
                self.state.reset_layout();
            }
            IdeAction::ShowAboutDialog => {
                self.dialogs.show_about = true;
            }
            IdeAction::ShowSettingsDialog => {
                self.dialogs.show_settings = true;
            }
            IdeAction::ShowNewProgramDialog => {
                self.dialogs.show_new_program = true;
            }
            IdeAction::ShowGoToLineDialog => {
                self.dialogs.show_go_to_line = true;
            }
            IdeAction::ShowFind => self.editor.open_find(_ctx, false),
            IdeAction::ShowFindReplaceDialog => self.editor.open_find(_ctx, true),
            IdeAction::Edit(cmd) => self.apply_edit_command(_ctx, cmd),
            IdeAction::ShowRunConfigDialog => self.open_run_config(),
            IdeAction::ZoomEditorFont(delta) => {
                let size = &mut self.state.editor_font_size;
                let range = crate::ide::EDITOR_FONT_RANGE;
                if delta == 0 {
                    *size = crate::ide::syntax::CodeStyle::DEFAULT_FONT_SIZE;
                } else {
                    *size = (*size + delta as f32).clamp(*range.start(), *range.end());
                }
                self.state.status_message = format!("Editor font size {}", *size);
            }
            IdeAction::TakeScreenshot => {
                if let Some(file) = rfd::FileDialog::new()
                    .add_filter("PNG Image", &["png"])
                    .set_file_name("screenshot.png")
                    .save_file()
                {
                    let grob = self
                        .controller
                        .shared
                        .screen
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .clone();
                    match grob.save_png(&file) {
                        Ok(()) => {
                            self.state.status_message =
                                format!("Screenshot saved to {}", file.display());
                        }
                        Err(e) => {
                            self.state.status_message = format!("Screenshot failed: {e}");
                        }
                    }
                }
            }
            IdeAction::NavigateToLocation { tab, line, col } => {
                if let Some(i) = tab.and_then(|id| self.editor.index_of(id)) {
                    self.editor.active_tab_index = i;
                }
                if let Some(tab) = self.editor.active_tab_mut() {
                    tab.goto_line(line, col);
                }
                self.state.cursor_line = line;
                self.state.cursor_col = col;
                self.state.calc_input_focus = false;
                self.state.status_message = format!("Jumped to line {line}, col {col}");
            }
        }
    }
}

impl eframe::App for IdeDesktopApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let p = self.state.to_persistent();
        if let Ok(json) = serde_json::to_string(&p) {
            storage.set_string("ide_panel_state", json.clone());
            storage.flush();
        }
        if let Some(path) = default_layout_config_path() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(json) = serde_json::to_string_pretty(&p) {
                let temporary = path.with_extension("json.tmp");
                if std::fs::write(&temporary, json).is_ok() {
                    let _ = std::fs::rename(temporary, path);
                }
            }
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let prev_calc_focus = self.state.calc_input_focus;

        // Scripted automation input (Xvfb tests) enters before any widget.
        crate::ide::automation::inject_input(self, ctx);

        // Handle keyboard shortcuts
        self.menu_bar.handle_shortcuts(ctx, &mut self.state);

        // Process any queued actions
        let actions = self.state.take_actions();
        for action in actions {
            self.handle_action(action, ctx);
        }

        // Live diagnostics (background, debounced) and the outline. Change
        // detection is by revision: no per-frame copy of the buffer.
        self.schedule_diagnostics(ctx);
        if let Some(tab) = self.editor.active_tab_mut() {
            let key = (tab.id, tab.revision());
            if self.outline_key != Some(key) {
                self.outline_key = Some(key);
                let doc = tab.document();
                let symbols = doc.analysis().symbols.clone();
                self.explorer.update_outline_from(&tab.content, &symbols);
            }
        }

        // Forward VM input while the calculator panel holds keyboard focus.
        self.forward_calc_keys(ctx);

        // Top Panel: Menu Bar
        egui::TopBottomPanel::top("menu_bar")
            .exact_height(24.0)
            .show(ctx, |ui| {
                self.menu_bar.render(ui, &mut self.state);
            });

        // Top Panel: Action Toolbar
        egui::TopBottomPanel::top("toolbar")
            .exact_height(40.0)
            .show(ctx, |ui| {
                self.toolbar.render(ui, &mut self.state);
            });

        // Bottom Panel: Status Bar
        if self.state.show_status_bar {
            egui::TopBottomPanel::bottom("status_bar")
                .exact_height(22.0)
                .show(ctx, |ui| {
                    self.status_bar.render(ui, &mut self.state);
                });
        }

        self.render_workspace(ctx);
        self.forward_touch(ctx);

        // Focus transition: clear held keys when focus is lost.
        if (self.last_calc_input_focus || prev_calc_focus) && !self.state.calc_input_focus {
            self.keypad.release(&mut self.state);
            self.controller.shared.clear_held_keys();
            self.controller.shared.set_touch(None);
            self.state.clear_held_keys();
            if let Some(f) = &self.firmware {
                f.send(ppl_engine::session::Command::ReleaseAll);
                if let Some(t) = self.last_touch.take() {
                    f.send(ppl_engine::session::Command::Touch {
                        status: 8,
                        x: t.x as u16,
                        y: t.y as u16,
                    });
                }
            }
            self.last_touch = None;
            self.deferred_touch_end = None;
            self.touch_start_time = None;
            self.prev_shift_held = false;
            self.prev_alpha_held = false;
        }
        self.last_calc_input_focus = self.state.calc_input_focus;

        // Poll the worker (terminal output, outcomes, debugger pauses).
        let selected_firmware_state = self.firmware_state.clone();
        if self.state.firmware_backend {
            self.state.vm_state = self.native_vm_state.clone();
        }
        self.poll_execution();
        self.native_vm_state = self.state.vm_state.clone();
        if self.state.firmware_backend {
            self.state.vm_state = selected_firmware_state;
        }
        self.poll_firmware();
        self.poll_checked_source();
        let active_source = self
            .editor
            .active_tab()
            .map(|tab| (tab.title.as_str(), tab.content.as_str()));
        self.program_manager.show(ctx, active_source);

        // Modal Dialogs (also editor-aware: find/replace, go-to-line).
        self.dialogs.render(ctx, &mut self.state, &mut self.editor);
        if self.dialogs.close_all_confirmed && !self.close_confirmed {
            self.close_confirmed = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // Workers wake the UI on output, display changes, completion and debug pauses.

        // Unsaved-changes guard on window close.
        if ctx.input(|i| i.viewport().close_requested()) {
            let unsaved = self.editor.tabs.iter().any(|t| t.modified);
            if unsaved && !self.close_confirmed {
                if !self.dialogs.show_confirm_close {
                    self.dialogs.show_confirm_close = true;
                    self.dialogs.confirm_close_all = true;
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
        }

        if !self.state.pending_actions.is_empty() {
            ctx.request_repaint();
        }
        // Scripted automation (Xvfb smoke tests).
        crate::ide::automation::maybe_pump(self, ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Stop the worker on the way out (the handle's Drop also interrupts).
        self.controller.stop();
        self.controller.reap_async();
        self.firmware = None;
    }
}

/// A dock tab header in the same style as the editor tab strip.
fn dock_tab_button(ui: &mut egui::Ui, title: &str, active: bool) -> bool {
    use crate::theme::{ACCENT_BLUE, TEXT_MUTED, TEXT_NORMAL};
    let font = egui::FontId::proportional(12.5);
    let w = ui.fonts(|f| {
        f.layout_no_wrap(title.to_string(), font.clone(), egui::Color32::WHITE)
            .size()
            .x
    });
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w + 24.0, 24.0), egui::Sense::click());
    let painter = ui.painter();
    if resp.hovered() && !active {
        painter.rect_filled(rect, 0.0, crate::theme::BG_WIDGET);
    }
    if active {
        painter.rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(rect.min.x, rect.max.y - 2.0),
                egui::vec2(rect.width(), 2.0),
            ),
            0.0,
            ACCENT_BLUE,
        );
    }
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        title,
        font,
        if active { TEXT_NORMAL } else { TEXT_MUTED },
    );
    resp.clicked()
}

/// Launches the full native desktop IDE window.
pub fn launch_ide(file: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let automation = crate::ide::automation::Automation::from_env();
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1440.0, 960.0])
        .with_min_inner_size([800.0, 500.0])
        .with_title("HP Prime PPL Emulator & Native IDE");
    if let (Ok(x_str), Ok(y_str)) = (
        std::env::var("PPL_WINDOW_POS_X"),
        std::env::var("PPL_WINDOW_POS_Y"),
    ) {
        if let (Ok(x), Ok(y)) = (x_str.parse::<f32>(), y_str.parse::<f32>()) {
            viewport = viewport.with_position([x, y]);
        }
    }
    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "HP Prime PPL Emulator & Native IDE",
        native_options,
        Box::new(move |cc| {
            let mut app = IdeDesktopApp::new(cc);
            app.automation = automation;
            if let Some(f) = file {
                app.open_file(f);
            }
            Ok(Box::new(app))
        }),
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "app_compile_tests.rs"]
mod compile_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_ide_desktop_app_defaults() {
        let app = IdeDesktopApp::default();
        assert!(app.state.show_explorer);
        assert!(app.state.show_calculator);
        assert!(app.state.show_bottom_dock);
        assert!(app.state.show_status_bar);
        assert_eq!(app.state.active_dock_tab, DockTab::Output);
        assert_eq!(app.state.vm_state, crate::ide::VmExecutionState::Idle);
        assert_eq!(app.editor.tabs.len(), 1);
        assert_eq!(app.editor.tabs[0].title, "DEMO.ppl");
    }

    #[test]
    fn test_ide_desktop_app_panel_toggles() {
        let mut app = IdeDesktopApp::default();
        let ctx = egui::Context::default();

        app.handle_action(IdeAction::ToggleExplorer, &ctx);
        assert!(!app.state.show_explorer);

        app.handle_action(IdeAction::ToggleCalculator, &ctx);
        assert!(!app.state.show_calculator);

        app.handle_action(IdeAction::ToggleBottomDock, &ctx);
        assert!(!app.state.show_bottom_dock);

        app.handle_action(IdeAction::ToggleStatusBar, &ctx);
        assert!(!app.state.show_status_bar);

        // Toggle back
        app.handle_action(IdeAction::ToggleExplorer, &ctx);
        assert!(app.state.show_explorer);
    }

    #[test]
    fn test_ide_desktop_app_layout_reset() {
        let mut app = IdeDesktopApp::default();
        app.state.show_explorer = false;
        app.state.show_calculator = false;
        app.state.explorer_width = 100.0;

        app.state.reset_layout();
        assert!(app.state.show_explorer);
        assert!(app.state.show_calculator);
        assert_eq!(app.state.explorer_width, 220.0);
    }

    #[test]
    fn test_ide_desktop_app_tab_lifecycle_and_breakpoints() {
        let mut app = IdeDesktopApp::default();
        let ctx = egui::Context::default();

        app.handle_action(IdeAction::NewProgram, &ctx);
        assert_eq!(app.editor.tabs.len(), 2);
        assert_eq!(app.editor.active_tab_index, 1);

        // Toggle breakpoint
        app.handle_action(IdeAction::ToggleBreakpoint(4), &ctx);
        assert!(app.editor.active_tab().unwrap().breakpoints.contains(&4));
        app.handle_action(IdeAction::ToggleBreakpoint(4), &ctx);
        assert!(!app.editor.active_tab().unwrap().breakpoints.contains(&4));

        // Close tab
        app.handle_action(IdeAction::CloseActiveTab, &ctx);
        assert_eq!(app.editor.tabs.len(), 1);
        assert_eq!(app.editor.active_tab_index, 0);
    }

    #[test]
    fn test_ide_desktop_app_syntax_check_diagnostics() {
        let mut app = IdeDesktopApp::default();
        let ctx = egui::Context::default();

        // Valid program
        app.handle_action(IdeAction::CheckSyntax, &ctx);
        for _ in 0..1000 {
            app.poll_native_compile();
            if app.native_compile.is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(app.problems.items.len(), 0);

        // Invalid program
        app.editor.active_tab_mut().unwrap().content =
            "EXPORT ERR() BEGIN LOCAL x := ; END;".to_string();
        app.handle_action(IdeAction::CheckSyntax, &ctx);
        for _ in 0..1000 {
            app.poll_native_compile();
            if app.native_compile.is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!app.problems.items.is_empty());
        assert_eq!(app.state.active_dock_tab, DockTab::Problems);
        assert!(app.state.show_bottom_dock);
    }

    #[test]
    fn test_ide_desktop_app_vm_state_flow() {
        let mut app = IdeDesktopApp::default();
        let ctx = egui::Context::default();

        app.handle_action(IdeAction::RunProgram, &ctx);
        assert_eq!(app.state.vm_state, crate::ide::VmExecutionState::Running);

        app.handle_action(IdeAction::StopProgram, &ctx);
        assert_eq!(app.state.vm_state, crate::ide::VmExecutionState::Idle);
        assert!(
            app.controller
                .is_interrupted
                .load(std::sync::atomic::Ordering::SeqCst)
        );
    }

    #[test]
    fn test_persistent_panel_state_roundtrip() {
        let state = IdeAppState {
            show_explorer: false,
            show_calculator: true,
            show_bottom_dock: false,
            show_status_bar: false,
            explorer_width: 300.0,
            calculator_width: 400.0,
            bottom_height: 220.0,
            active_dock_tab: DockTab::Variables,
            ..Default::default()
        };

        let persistent = state.to_persistent();
        let json = serde_json::to_string(&persistent).expect("serialize");
        let restored: crate::ide::PersistentPanelState =
            serde_json::from_str(&json).expect("deserialize");

        let mut new_state = IdeAppState::default();
        new_state.apply_persistent(&restored);

        assert!(!new_state.show_explorer);
        assert!(new_state.show_calculator);
        assert!(!new_state.show_bottom_dock);
        assert!(!new_state.show_status_bar);
        assert_eq!(new_state.explorer_width, 300.0);
        assert_eq!(new_state.calculator_width, 400.0);
        assert_eq!(new_state.bottom_height, 220.0);
        assert_eq!(new_state.active_dock_tab, DockTab::Variables);
    }

    #[test]
    fn test_new_program_dialog_name_respected() {
        let mut app = IdeDesktopApp::default();
        let ctx = egui::Context::default();
        app.dialogs.new_prog_name = "FACTORIAL".to_string();
        app.handle_action(IdeAction::NewProgram, &ctx);
        let tab = app.editor.active_tab().unwrap();
        assert_eq!(tab.title, "FACTORIAL.ppl");
        assert!(tab.content.contains("EXPORT FACTORIAL()"));
    }

    #[test]
    fn test_ide_desktop_app_unsaved_buffer_run() {
        let mut app = IdeDesktopApp::default();
        let ctx = egui::Context::default();

        let tab = app.editor.active_tab_mut().unwrap();
        tab.path = None; // Unsaved
        tab.modified = true;
        tab.content = "EXPORT FOO() BEGIN PRINT(100+23); RETURN 123; END;".to_string();

        app.handle_action(IdeAction::RunProgram, &ctx);
        assert_eq!(app.state.vm_state, crate::ide::VmExecutionState::Running);

        for _ in 0..200 {
            app.poll_execution();
            if matches!(app.state.vm_state, crate::ide::VmExecutionState::Terminated) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        assert_eq!(app.state.vm_state, crate::ide::VmExecutionState::Terminated);
        assert!(app.console.plain_text().contains("123"));
    }

    #[test]
    fn test_valid_ppl_with_unemulated_commands_is_not_an_error() {
        let mut app = IdeDesktopApp::default();
        let ctx = egui::Context::default();
        app.editor.active_tab_mut().unwrap().content = "EXPORT ASK()\nBEGIN\n  LOCAL r;\n  INPUT(r,\"T\",\"r=\",\"help\",1,5);\n  IF r > 99 THEN RETURN SOLVE(X^2-1, X, 2); END;\n  RETURN append({1}, r);\nEND;\n".to_string();
        app.handle_action(IdeAction::CheckSyntax, &ctx);
        for _ in 0..1000 {
            app.poll_native_compile();
            if app.native_compile.is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let classes: Vec<_> = app.problems.items.iter().map(|p| p.class).collect();
        assert!(!classes.is_empty());
        assert!(
            classes
                .iter()
                .all(|c| *c == ppl_syntax::DiagnosticClass::Compatibility),
            "only compatibility diagnostics: {:#?}",
            app.problems.items
        );
        assert_eq!(app.problems.error_count(), 0);
        let tab_id = app.editor.active_tab().unwrap().id;
        assert!(app.problems.items.iter().all(|p| p.tab == Some(tab_id)));
        assert!(app.problems.items.iter().all(|p| p.file == "DEMO.ppl"));
    }

    #[test]
    fn test_runtime_error_is_reported_as_runtime_problem() {
        let mut app = IdeDesktopApp::default();
        let ctx = egui::Context::default();
        app.editor.active_tab_mut().unwrap().content =
            "EXPORT EV()\nBEGIN\n  PRINT(1);\n  RETURN EVAL(\"1+1\");\nEND;\n".to_string();
        app.handle_action(IdeAction::RunProgram, &ctx);
        for _ in 0..300 {
            app.poll_execution();
            if matches!(app.state.vm_state, crate::ide::VmExecutionState::Error(_)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let runtime: Vec<_> = app
            .problems
            .items
            .iter()
            .filter(|p| p.class == ppl_syntax::DiagnosticClass::Runtime)
            .collect();
        assert_eq!(runtime.len(), 1, "{:#?}", app.problems.items);
        assert_eq!(runtime[0].line, 4);
        assert!(
            runtime[0].message.contains("EVAL"),
            "{}",
            runtime[0].message
        );
        // The compatibility warning about EVAL is a separate entry.
        assert!(
            app.problems
                .items
                .iter()
                .any(|p| p.class == ppl_syntax::DiagnosticClass::Compatibility
                    && p.code == "unsupported-command")
        );
    }

    #[test]
    fn test_problem_navigation_switches_to_the_right_tab() {
        let mut app = IdeDesktopApp::default();
        let ctx = egui::Context::default();
        let first = app.editor.active_tab().unwrap().id;
        app.editor.new_tab("OTHER.ppl", "EXPORT O()\nBEGIN\nEND;\n");
        assert_eq!(app.editor.active_tab_index, 1);
        app.handle_action(
            IdeAction::NavigateToLocation {
                tab: Some(first),
                line: 3,
                col: 3,
            },
            &ctx,
        );
        assert_eq!(app.editor.active_tab_index, 0);
        let tab = app.editor.active_tab().unwrap();
        // Line 3 of DEMO.ppl starts at char 20; col 3 is char 22.
        assert_eq!(tab.pending_jump, Some(22));
    }

    #[test]
    fn test_live_diagnostics_run_in_background_and_follow_edits() {
        let mut app = IdeDesktopApp::default();
        let ctx = egui::Context::default();
        let frame = |app: &mut IdeDesktopApp| {
            let _ = ctx.run(Default::default(), |ctx| app.schedule_diagnostics(ctx));
        };
        app.editor.active_tab_mut().unwrap().content =
            "EXPORT PROG()\nBEGIN\n  RETURN undeclared;\nEND;\n".to_string();
        let mut found = false;
        for _ in 0..400 {
            frame(&mut app);
            if app.problems.error_count() > 0 {
                found = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(found, "first check of a tab runs without waiting");
        assert_eq!(app.problems.items[0].code, "undeclared-identifier");

        // Fix the error: results for the old revision are discarded and the
        // new revision is checked after the quiet period.
        app.editor.active_tab_mut().unwrap().content =
            "EXPORT PROG()\nBEGIN\n  RETURN 1;\nEND;\n".to_string();
        let mut cleared = false;
        for _ in 0..400 {
            frame(&mut app);
            if app.problems.items.is_empty() {
                cleared = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(cleared, "{:#?}", app.problems.items);
    }

    #[test]
    fn test_problems_cover_inactive_tabs() {
        let mut app = IdeDesktopApp::default();
        let ctx = egui::Context::default();
        app.editor.new_tab(
            "BROKEN.ppl",
            "EXPORT BROKEN()\nBEGIN\n  PRINT(\"open);\nEND;\n",
        );
        app.editor.new_tab("OK.ppl", "EXPORT OKAY()\nBEGIN\nEND;\n");
        // The broken tab is never activated.
        let mut seen = false;
        for _ in 0..400 {
            let _ = ctx.run(Default::default(), |ctx| app.schedule_diagnostics(ctx));
            if app.problems.items.iter().any(|p| p.file == "BROKEN.ppl") {
                seen = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(seen, "{:#?}", app.problems.items);
        let item = app
            .problems
            .items
            .iter()
            .find(|p| p.code == "unclosed-string")
            .unwrap();
        assert_eq!(
            (item.line, item.class),
            (3, ppl_syntax::DiagnosticClass::Syntax)
        );
    }

    #[test]
    fn test_ide_desktop_app_restart_program() {
        let mut app = IdeDesktopApp::default();
        let ctx = egui::Context::default();

        let tab = app.editor.active_tab_mut().unwrap();
        tab.content = "EXPORT TEST() BEGIN RETURN 42; END;".to_string();

        app.handle_action(IdeAction::RunProgram, &ctx);
        app.handle_action(IdeAction::StopProgram, &ctx);
        assert_eq!(app.state.vm_state, crate::ide::VmExecutionState::Idle);

        // Restart must clear interrupt and successfully run
        app.handle_action(IdeAction::RestartProgram, &ctx);
        assert_eq!(app.state.vm_state, crate::ide::VmExecutionState::Running);

        for _ in 0..200 {
            app.poll_execution();
            if matches!(app.state.vm_state, crate::ide::VmExecutionState::Terminated) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        assert_eq!(app.state.vm_state, crate::ide::VmExecutionState::Terminated);
    }

    #[test]
    fn test_ide_desktop_app_f5_continues_when_paused() {
        let mut app = IdeDesktopApp::default();
        let ctx = egui::Context::default();

        app.state.vm_state = crate::ide::VmExecutionState::Paused;
        // In paused state, RunProgram (dispatched by F5) should route to ContinueExecution
        app.handle_action(IdeAction::RunProgram, &ctx);
        // If controller wasn't actually paused, status message is set to "Not paused"
        // confirming that ContinueExecution was dispatched rather than RunProgram failing with "Already running".
        assert_eq!(app.state.status_message, "Not paused");
    }

    #[test]
    fn test_run_config_dialog_lists_exported_functions() {
        let mut app = IdeDesktopApp::default();
        let ctx = egui::Context::default();

        let tab = app.editor.active_tab_mut().unwrap();
        tab.content = "EXPORT SQUARE(x)\nBEGIN\n  RETURN x*x;\nEND;\nEXPORT MAIN()\nBEGIN\n  RETURN SQUARE(3);\nEND;"
            .to_string();

        app.handle_action(IdeAction::ShowRunConfigDialog, &ctx);
        assert!(app.dialogs.show_run_config);
        assert_eq!(
            app.dialogs.run_config_entries,
            vec![("SQUARE".to_string(), 1), ("MAIN".to_string(), 0),]
        );
    }

    #[test]
    fn test_run_config_executes_parameterized_function() {
        let mut app = IdeDesktopApp::default();
        let ctx = egui::Context::default();

        let tab = app.editor.active_tab_mut().unwrap();
        tab.content = "EXPORT SQUARE(x)\nBEGIN\n  RETURN x*x;\nEND;".to_string();
        // Simulate the dialog's Save & Run: configured entry + arguments.
        tab.run_entry = Some("SQUARE".to_string());
        tab.run_args = "5".to_string();

        app.handle_action(IdeAction::RunProgram, &ctx);
        assert_eq!(app.state.vm_state, crate::ide::VmExecutionState::Running);

        for _ in 0..200 {
            app.poll_execution();
            if matches!(app.state.vm_state, crate::ide::VmExecutionState::Terminated) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        assert_eq!(app.state.vm_state, crate::ide::VmExecutionState::Terminated);
        assert!(app.console.plain_text().contains("returned 25"));
    }

    #[test]
    fn test_run_config_reports_bad_argument_errors() {
        let mut app = IdeDesktopApp::default();
        let ctx = egui::Context::default();

        let tab = app.editor.active_tab_mut().unwrap();
        tab.content = "EXPORT SQUARE(x)\nBEGIN\n  RETURN x*x;\nEND;".to_string();
        tab.run_entry = Some("SQUARE".to_string());
        tab.run_args = "{unclosed".to_string();

        app.handle_action(IdeAction::RunProgram, &ctx);
        for _ in 0..200 {
            app.poll_execution();
            if matches!(
                app.state.vm_state,
                crate::ide::VmExecutionState::Error(_) | crate::ide::VmExecutionState::Terminated
            ) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(matches!(
            app.state.vm_state,
            crate::ide::VmExecutionState::Error(_)
        ));
        assert!(app.console.plain_text().contains("bad argument"));
    }

    #[test]
    fn test_run_config_switches_between_exported_functions() {
        let mut app = IdeDesktopApp::default();
        let ctx = egui::Context::default();

        let tab = app.editor.active_tab_mut().unwrap();
        tab.content = "EXPORT ADD(a, b)\nBEGIN\n  RETURN a+b;\nEND;\nEXPORT DOUBLE(a)\nBEGIN\n  RETURN 2*a;\nEND;"
            .to_string();

        // First run: ADD(2,3) -> 5.
        tab.run_entry = Some("ADD".to_string());
        tab.run_args = "2, 3".to_string();
        app.handle_action(IdeAction::RunProgram, &ctx);
        for _ in 0..200 {
            app.poll_execution();
            if app.controller.outcome().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = app.controller.join_current();
        assert!(app.console.plain_text().contains("returned 5"));

        // Second run: DOUBLE(21) -> 42.
        let tab = app.editor.active_tab_mut().unwrap();
        tab.run_entry = Some("DOUBLE".to_string());
        tab.run_args = "21".to_string();
        app.state.vm_state = crate::ide::VmExecutionState::Idle;
        app.handle_action(IdeAction::RunProgram, &ctx);
        for _ in 0..200 {
            app.poll_execution();
            if app.controller.outcome().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = app.controller.join_current();
        assert!(app.console.plain_text().contains("returned 42"));
    }

    #[test]
    fn test_dual_backend_state_preservation() {
        let ctx = egui::Context::default();
        let mut app = IdeDesktopApp::default();
        app.editor.active_tab_mut().unwrap().content =
            "EXPORT TEST()\nBEGIN\n PRINT(\"Dual-backend\");\nEND;\n".into();
        app.editor.active_tab_mut().unwrap().breakpoints.insert(3);
        app.console.print_diagnostic("Pre-switch log");

        // Verify initial Native PPL state
        assert!(!app.state.firmware_backend);
        app.state.vm_state = crate::ide::VmExecutionState::Paused;

        // Switch to G1 Firmware
        app.handle_action(IdeAction::SelectBackend(true), &ctx);
        assert!(app.state.firmware_backend);
        assert_eq!(app.state.vm_state, crate::ide::VmExecutionState::Idle);
        // Breakpoints, console, and display reset sentinel are preserved
        assert!(app.editor.active_tab().unwrap().breakpoints.contains(&3));
        assert!(app.console.plain_text().contains("Pre-switch log"));
        assert_eq!(app.display.last_generation, u64::MAX);

        // Simulate firmware execution state change
        app.firmware_state = crate::ide::VmExecutionState::Running;
        app.poll_firmware();
        assert_eq!(app.state.vm_state, crate::ide::VmExecutionState::Running);

        // Switch back to Native PPL
        app.handle_action(IdeAction::SelectBackend(false), &ctx);
        assert!(!app.state.firmware_backend);
        // Native state restored to Paused!
        assert_eq!(app.state.vm_state, crate::ide::VmExecutionState::Paused);
        assert!(app.editor.active_tab().unwrap().breakpoints.contains(&3));
        assert!(app.console.plain_text().contains("Pre-switch log"));
        assert_eq!(app.display.last_generation, u64::MAX);
    }
}

#[cfg(test)]
mod layout_and_persistence_tests {
    use super::*;
    use crate::ide::layout::WorkspaceLayout;

    #[test]
    fn test_layout_transitions_and_escape_restore() {
        let mut app = IdeDesktopApp::default();
        let ctx = egui::Context::default();

        assert_eq!(app.state.layout, WorkspaceLayout::Split);
        assert_eq!(app.state.previous_layout, WorkspaceLayout::Split);

        // 1. Transition Split -> Calculator
        app.handle_action(IdeAction::SetLayout(WorkspaceLayout::Calculator), &ctx);
        assert_eq!(app.state.layout, WorkspaceLayout::Calculator);
        assert_eq!(app.state.previous_layout, WorkspaceLayout::Split);

        // 2. Escape restores Split when calc_input_focus is false
        app.state.calc_input_focus = false;
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        let _ = ctx.run(raw, |_| {});
        app.menu_bar.handle_shortcuts(&ctx, &mut app.state);
        let actions = app.state.take_actions();
        assert_eq!(actions, vec![IdeAction::SetLayout(WorkspaceLayout::Split)]);
        for a in actions {
            app.handle_action(a, &ctx);
        }
        assert_eq!(app.state.layout, WorkspaceLayout::Split);

        // 3. Transition Editor Focus -> Calculator
        app.handle_action(IdeAction::SetLayout(WorkspaceLayout::Editor), &ctx);
        assert_eq!(app.state.layout, WorkspaceLayout::Editor);
        assert_eq!(app.state.previous_layout, WorkspaceLayout::Split);

        app.handle_action(IdeAction::SetLayout(WorkspaceLayout::Calculator), &ctx);
        assert_eq!(app.state.layout, WorkspaceLayout::Calculator);
        assert_eq!(app.state.previous_layout, WorkspaceLayout::Editor);

        // 4. Repeated Calculator calls never overwrite previous_layout
        app.handle_action(IdeAction::SetLayout(WorkspaceLayout::Calculator), &ctx);
        assert_eq!(app.state.previous_layout, WorkspaceLayout::Editor);

        // 5. Escape restores Editor
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        let _ = ctx.run(raw, |_| {});
        app.menu_bar.handle_shortcuts(&ctx, &mut app.state);
        let actions = app.state.take_actions();
        assert_eq!(actions, vec![IdeAction::SetLayout(WorkspaceLayout::Editor)]);
        for a in actions {
            app.handle_action(a, &ctx);
        }
        assert_eq!(app.state.layout, WorkspaceLayout::Editor);
    }

    #[test]
    fn test_escape_does_not_restore_layout_when_calc_focused() {
        let mut app = IdeDesktopApp::default();
        let ctx = egui::Context::default();

        app.handle_action(IdeAction::SetLayout(WorkspaceLayout::Calculator), &ctx);
        app.state.calc_input_focus = true;

        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        let _ = ctx.run(raw, |_| {});
        app.menu_bar.handle_shortcuts(&ctx, &mut app.state);
        let actions = app.state.take_actions();
        assert!(
            actions.is_empty(),
            "Escape must be consumed by calculator, not restore layout"
        );
    }

    #[test]
    fn test_clamping_isolation_and_persistence() {
        let state = IdeAppState {
            calculator_width: 450.0,
            explorer_width: 250.0,
            ..Default::default()
        };

        // Narrow width clamping limits
        let limits = crate::ide::layout::PanelLimits::for_width(700.0, true);
        assert!(limits.calculator_max < 450.0);

        // Persistent serialization must retain the original preferred values
        let p = state.to_persistent();
        assert_eq!(p.calculator_width, 450.0);
        assert_eq!(p.explorer_width, 250.0);
    }

    #[test]
    fn test_sanitization_of_corrupt_previous_layout() {
        let mut state = IdeAppState::default();
        let mut p = state.to_persistent();
        p.previous_layout = WorkspaceLayout::Calculator;
        state.apply_persistent(&p);
        assert_eq!(state.previous_layout, WorkspaceLayout::Split);
    }
}
