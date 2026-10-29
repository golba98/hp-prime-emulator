//! Opt-in IDE automation for headless/Xvfb GUI workflow tests.
//!
//! Activated only when the `PPL_IDE_AUTOMATION` environment variable points
//! to a JSON script file. The script drives the *production* action paths
//! (the same `IdeAction` handlers the UI uses), with file paths injected so
//! native file pickers are bypassed. On `exit` the IDE dumps its observable
//! state as JSON for external assertions, then closes the window.
//!
//! This exists for the same reason as `--exit-after` on the standalone
//! window: honest, scripted smoke tests of the real event loop.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::app::IdeDesktopApp;
use crate::ide::IdeAction;

#[derive(Debug, Clone)]
pub enum AutoStep {
    /// Create a new program tab named NAME.
    NewProgram(String),
    /// Open a file directly (no native picker).
    OpenFile(PathBuf),
    /// Overwrite the active editor buffer.
    SetContent(String),
    /// Save the active tab; a path overrides the picker.
    Save(Option<PathBuf>),
    /// Save the active tab to a specific path (Save As).
    SaveAs(PathBuf),
    /// Close the active tab.
    CloseTab,
    Run,
    /// Read-only emulator / physical calculator screen capture.
    CompareScreens,
    Debug,
    Stop,
    Continue,
    StepInto,
    StepOver,
    StepOut,
    Breakpoint(usize),
    /// Type text into the VM key queue (for INPUT dialogs).
    SendKeys(String),
    /// Press one HP keycode.
    PressKey(i64),
    /// Run with an explicit entry point and/or comma-separated arguments.
    RunConfig {
        entry: Option<String>,
        args: String,
    },
    /// Wait until the VM is idle (finished/failed) with a timeout in ms.
    WaitIdle(u64),
    /// Wait until the debugger is paused, with a timeout in ms.
    WaitPaused(u64),
    /// Save the emulated screen to a PNG.
    Screenshot(PathBuf),
    /// Give the code editor keyboard focus.
    FocusEditor,
    ClickCalculator,
    ClickCalculatorKey(i64),
    Snapshot(PathBuf),
    Detach,
    NativeHome,
    NativeCatalog,
    IdeSettings,
    /// Type text into the focused widget as real keyboard input: `\n`
    /// becomes Enter and `\t` becomes Tab.
    Type(String),
    /// Press one key with modifiers (`key` uses egui key names, e.g. "Tab").
    Key {
        key: egui::Key,
        shift: bool,
        command: bool,
    },
    /// Move the cursor to a 1-based line/column.
    Goto(usize, usize),
    /// Select a char range in the active editor.
    Select(usize, usize),
    /// Set the editor font size.
    FontSize(f32),
    /// Resize the window (logical points).
    Resize(f32, f32),
    /// Do nothing for the given milliseconds (lets layout settle).
    WaitMs(u64),
    /// Capture the rendered window to a PNG.
    WindowScreenshot(PathBuf),
    /// Activate the editor tab at this index.
    SelectTab(usize),
    /// Show a bottom dock tab: "output", "problems", "variables", "callstack".
    Dock(String),
    /// Dump state JSON to the dump path and close the window.
    Exit,
    Layout(String),
    CalculatorMode(String),
    Backend(bool),
    MaxSteps(u64),
}

pub struct Automation {
    pub steps: Vec<AutoStep>,
    pub next: usize,
    pub dump_path: PathBuf,
    wait_deadline: Option<Instant>,
    dumped: bool,
    /// Input to inject at the start of the next frame.
    pending_events: Vec<egui::Event>,
    pending_pointer_release: Option<egui::Pos2>,
    /// Window screenshot requested and not yet delivered.
    pending_screenshot: Option<PathBuf>,
    /// Sizes and scale observed at each window screenshot, for the dump.
    captures: Vec<serde_json::Value>,
    last_pump_frame: Option<u64>,
}

impl Automation {
    /// Loads the automation script from `PPL_IDE_AUTOMATION`.
    pub fn from_env() -> Option<Self> {
        let path = std::env::var_os("PPL_IDE_AUTOMATION")?;
        let content = std::fs::read_to_string(path).ok()?;
        let json: serde_json::Value = serde_json::from_str(&content).ok()?;
        let dump_path = json
            .get("dump")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp/ppl-ide-automation-dump.json"));
        let mut steps = Vec::new();
        if let Some(arr) = json.get("steps").and_then(|v| v.as_array()) {
            for s in arr {
                let ty = s.get("step").and_then(|v| v.as_str()).unwrap_or("");
                let get_str = |k: &str| {
                    s.get(k)
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string()
                };
                let get_path = |k: &str| s.get(k).and_then(|v| v.as_str()).map(PathBuf::from);
                let get_ms = |k: &str| s.get(k).and_then(|v| v.as_u64()).unwrap_or(5000);
                steps.push(match ty {
                    "new_program" => AutoStep::NewProgram(get_str("name")),
                    "open" => AutoStep::OpenFile(get_path("path").unwrap_or_default()),
                    "set_content" => AutoStep::SetContent(get_str("content")),
                    "save" => AutoStep::Save(get_path("path")),
                    "save_as" => AutoStep::SaveAs(get_path("path").unwrap_or_default()),
                    "close_tab" => AutoStep::CloseTab,
                    "run" => AutoStep::Run,
                    "max_steps" => AutoStep::MaxSteps(
                        s.get("limit").and_then(|v| v.as_u64()).unwrap_or(1_000_000),
                    ),
                    "compare_screens" => AutoStep::CompareScreens,
                    "debug" => AutoStep::Debug,
                    "stop" => AutoStep::Stop,
                    "continue" => AutoStep::Continue,
                    "step_into" => AutoStep::StepInto,
                    "step_over" => AutoStep::StepOver,
                    "step_out" => AutoStep::StepOut,
                    "breakpoint" => AutoStep::Breakpoint(
                        s.get("line").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                    ),
                    "send_keys" => AutoStep::SendKeys(get_str("keys")),
                    "press_key" => {
                        AutoStep::PressKey(s.get("code").and_then(|v| v.as_i64()).unwrap_or(0))
                    }
                    "run_config" => AutoStep::RunConfig {
                        entry: s.get("entry").and_then(|v| v.as_str()).map(String::from),
                        args: get_str("args"),
                    },
                    "wait_idle" => AutoStep::WaitIdle(get_ms("timeout_ms")),
                    "wait_paused" => AutoStep::WaitPaused(get_ms("timeout_ms")),
                    "screenshot" => AutoStep::Screenshot(get_path("path").unwrap_or_default()),
                    "focus_editor" => AutoStep::FocusEditor,
                    "click_calculator" | "focus_calculator" => AutoStep::ClickCalculator,
                    "click_calculator_key" => AutoStep::ClickCalculatorKey(
                        s.get("code").and_then(|v| v.as_i64()).unwrap_or(-1),
                    ),
                    "snapshot" => AutoStep::Snapshot(get_path("path").unwrap_or_default()),
                    "detach" => AutoStep::Detach,
                    "native_home" => AutoStep::NativeHome,
                    "native_catalog" => AutoStep::NativeCatalog,
                    "ide_settings" => AutoStep::IdeSettings,
                    "type" => AutoStep::Type(get_str("text")),
                    "key" => match egui::Key::from_name(&get_str("key")) {
                        Some(key) => AutoStep::Key {
                            key,
                            shift: s.get("shift").and_then(|v| v.as_bool()).unwrap_or(false),
                            command: s.get("command").and_then(|v| v.as_bool()).unwrap_or(false),
                        },
                        None => {
                            eprintln!("automation: unknown key `{}`; skipping", get_str("key"));
                            continue;
                        }
                    },
                    "goto" => AutoStep::Goto(
                        s.get("line").and_then(|v| v.as_u64()).unwrap_or(1) as usize,
                        s.get("col").and_then(|v| v.as_u64()).unwrap_or(1) as usize,
                    ),
                    "select" => AutoStep::Select(
                        s.get("start").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                        s.get("end").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                    ),
                    "font_size" => AutoStep::FontSize(
                        s.get("size").and_then(|v| v.as_f64()).unwrap_or(14.0) as f32,
                    ),
                    "resize" => AutoStep::Resize(
                        s.get("width").and_then(|v| v.as_f64()).unwrap_or(1280.0) as f32,
                        s.get("height").and_then(|v| v.as_f64()).unwrap_or(800.0) as f32,
                    ),
                    "wait_ms" => AutoStep::WaitMs(get_ms("ms")),
                    "select_tab" => AutoStep::SelectTab(
                        s.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                    ),
                    "dock" => AutoStep::Dock(get_str("tab")),
                    "window_screenshot" => {
                        AutoStep::WindowScreenshot(get_path("path").unwrap_or_default())
                    }
                    "exit" => AutoStep::Exit,
                    "layout" => AutoStep::Layout(get_str("mode")),
                    "calculator_mode" => AutoStep::CalculatorMode(get_str("mode")),
                    "backend" => AutoStep::Backend(get_str("engine") == "firmware"),
                    other => {
                        eprintln!("automation: unknown step `{other}`; skipping");
                        continue;
                    }
                });
            }
        }
        Some(Self {
            steps,
            next: 0,
            dump_path,
            wait_deadline: None,
            dumped: false,
            pending_events: Vec::new(),
            pending_pointer_release: None,
            pending_screenshot: None,
            captures: Vec::new(),
            last_pump_frame: None,
        })
    }

    /// Runs at most one non-waiting step per frame.
    pub fn pump(&mut self, app: &mut IdeDesktopApp, ctx: &egui::Context) {
        // egui may repeat layout passes within one frame. Advancing once per
        // pass can merge independent scripted key presses into one input
        // batch, violating the recorder's one-action-per-frame contract.
        let frame = ctx.cumulative_frame_nr();
        if self.last_pump_frame == Some(frame) {
            return;
        }
        self.last_pump_frame = Some(frame);
        loop {
            if self.next >= self.steps.len() {
                // Script finished without an explicit exit: dump and close.
                self.finish(app, ctx);
                return;
            }
            if let Some(path) = self.pending_screenshot.clone() {
                // Wait for the frame capture requested by WindowScreenshot.
                let image = ctx.input(|i| {
                    i.events.iter().find_map(|e| match e {
                        egui::Event::Screenshot { image, .. } => Some(image.clone()),
                        _ => None,
                    })
                });
                match image {
                    Some(image) => {
                        if let Err(e) = save_color_image(&image, &path) {
                            eprintln!("automation: window screenshot failed: {e}");
                        }
                        let tab = app.editor.active_tab();
                        let cursor = tab.and_then(|t| {
                            egui::text_edit::TextEditState::load(
                                ctx,
                                crate::ide::editor::view::edit_id(t.id),
                            )?
                            .cursor
                            .char_range()
                            .map(|r| r.primary.index)
                        });
                        self.captures.push(serde_json::json!({
                            "path": path.display().to_string(),
                            "pixels": [image.size[0], image.size[1]],
                            "pixels_per_point": ctx.pixels_per_point(),
                            "screen_points": [ctx.screen_rect().width(), ctx.screen_rect().height()],
                            "cursor_char": cursor,
                            "layouts": tab.map(|t| t.doc.stats.layouts),
                            "analyses": tab.map(|t| t.doc.stats.analyses),
                            "visible_lines": app.editor.view.last_frame.as_ref().map(|f| {
                                [f.visible_lines.start, f.visible_lines.end]
                            }),
                        }));
                        self.pending_screenshot = None;
                    }
                    None => {
                        ctx.request_repaint();
                        return;
                    }
                }
            }
            // Consume screenshot responses even after focus loss, but never
            // advance past keyboard input that could not be delivered.
            if !ctx.input(|i| i.focused) || !self.pending_events.is_empty() {
                ctx.request_repaint_after(Duration::from_millis(50));
                return;
            }
            let step = self.steps[self.next].clone();
            if let AutoStep::WaitMs(ms) = step {
                let start = *self.wait_deadline.get_or_insert_with(Instant::now);
                if start.elapsed() >= Duration::from_millis(ms) {
                    self.wait_deadline = None;
                    self.next += 1;
                    continue;
                }
                ctx.request_repaint_after(Duration::from_millis(16));
                return;
            }
            match step {
                AutoStep::WaitIdle(_) | AutoStep::WaitPaused(_) => {
                    let deadline = self.wait_deadline.get_or_insert_with(Instant::now);
                    let ms = match &self.steps[self.next] {
                        AutoStep::WaitIdle(ms) => *ms,
                        AutoStep::WaitPaused(ms) => *ms,
                        _ => unreachable!(),
                    };
                    let done = if matches!(self.steps[self.next], AutoStep::WaitIdle(_)) {
                        !app.execution_busy()
                    } else {
                        app.controller.paused()
                    };
                    if done {
                        self.wait_deadline = None;
                        self.next += 1;
                        continue;
                    }
                    if deadline.elapsed() > Duration::from_millis(ms) {
                        eprintln!("automation: wait step {} timed out; continuing", self.next);
                        self.wait_deadline = None;
                        self.next += 1;
                        continue;
                    }
                    ctx.request_repaint_after(Duration::from_millis(30));
                    return;
                }
                other => {
                    self.next += 1;
                    self.apply(app, other, ctx);
                    // Only one action step per frame so the UI can settle.
                    ctx.request_repaint();
                    return;
                }
            }
        }
    }

    fn apply(&mut self, app: &mut IdeDesktopApp, step: AutoStep, ctx: &egui::Context) {
        match step {
            AutoStep::MaxSteps(limit) => app.dialogs.max_steps_input = limit.to_string(),
            AutoStep::CompareScreens => app.handle_action(IdeAction::CompareScreens, ctx),
            AutoStep::NewProgram(name) => {
                app.dialogs.new_prog_name = name;
                app.handle_action(IdeAction::NewProgram, ctx);
            }
            AutoStep::OpenFile(path) => {
                app.handle_action(IdeAction::OpenFile(Some(path)), ctx);
            }
            AutoStep::SetContent(content) => {
                if let Some(tab) = app.editor.active_tab_mut() {
                    tab.content = content;
                    tab.modified = true;
                    app.state.status_message = "Buffer content set".into();
                }
            }
            AutoStep::Save(path) => {
                if let Some(p) = path {
                    if let Some(tab) = app.editor.active_tab_mut() {
                        tab.path = Some(p);
                    }
                }
                app.handle_action(IdeAction::SaveActiveFile, ctx);
            }
            AutoStep::SaveAs(path) => {
                let _ = app.save_active_tab(Some(path));
            }
            AutoStep::CloseTab => {
                // Discard unsaved in automation mode: save first if modified.
                if app.editor.active_tab_modified() {
                    app.handle_action(IdeAction::SaveActiveFile, ctx);
                }
                app.editor.close_tab(app.editor.active_tab_index);
            }
            AutoStep::Run => app.handle_action(IdeAction::RunProgram, ctx),
            AutoStep::Debug => app.handle_action(IdeAction::StartDebugging, ctx),
            AutoStep::Stop => app.handle_action(IdeAction::StopProgram, ctx),
            AutoStep::Continue => app.handle_action(IdeAction::ContinueExecution, ctx),
            AutoStep::StepInto => app.handle_action(IdeAction::StepInto, ctx),
            AutoStep::StepOver => app.handle_action(IdeAction::StepOver, ctx),
            AutoStep::StepOut => app.handle_action(IdeAction::StepOut, ctx),
            AutoStep::Breakpoint(line) => {
                app.handle_action(IdeAction::ToggleBreakpoint(line), ctx);
            }
            AutoStep::SendKeys(text) => {
                for ch in text.chars() {
                    app.controller
                        .shared
                        .push_key_press(crate::dialog::KeyPress {
                            code: None,
                            ch: Some(ch),
                        });
                }
            }
            AutoStep::PressKey(code) => {
                app.controller.shared.push_key(code);
            }
            AutoStep::RunConfig { entry, args } => {
                if let Some(tab) = app.editor.active_tab_mut() {
                    tab.run_entry = entry;
                    tab.run_args = args;
                }
                app.handle_action(IdeAction::RunProgram, ctx);
            }
            AutoStep::WaitIdle(_) | AutoStep::WaitPaused(_) => unreachable!("handled in pump"),
            AutoStep::Screenshot(path) => {
                if let Some((grob, _)) = app.capture_compare_frame() {
                    match grob.save_png(&path) {
                        Ok(()) => {
                            app.state.status_message =
                                format!("Screenshot saved to {}", path.display())
                        }
                        Err(e) => app.state.status_message = format!("Screenshot failed: {e}"),
                    }
                } else {
                    app.state.status_message = "No current frame from the selected engine".into();
                }
            }
            AutoStep::FocusEditor => {
                if let Some(tab) = app.editor.active_tab() {
                    let id = crate::ide::editor::view::edit_id(tab.id);
                    ctx.memory_mut(|m| m.request_focus(id));
                }
            }
            AutoStep::ClickCalculator => {
                let pos = app.display.last_display_rect.center();
                self.pending_events.extend([
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
            }
            AutoStep::ClickCalculatorKey(code) => {
                if let Some(key) = crate::ide::keypad::KEYS.iter().find(|key| key.code == code) {
                    let lcd = app.display.last_display_rect;
                    let scale = lcd.width() / 320.0;
                    let origin = lcd.min - egui::vec2(28.0, 58.0) * scale;
                    let pos = key.rect(origin, scale).center();
                    self.pending_events.extend([
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]);
                }
            }
            AutoStep::Snapshot(path) => {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(path, self.dump_state(app, ctx));
            }
            AutoStep::Detach => {
                if let Some(parent) = self.dump_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let snapshot = self.dump_state(app, ctx);
                let _ = std::fs::write(&self.dump_path, snapshot);
                self.dumped = true;
            }
            AutoStep::NativeHome => app.handle_action(IdeAction::NativeHome, ctx),
            AutoStep::NativeCatalog => app.handle_action(IdeAction::NativeCatalog, ctx),
            AutoStep::IdeSettings => app.handle_action(IdeAction::ShowSettingsDialog, ctx),
            AutoStep::Type(text) => {
                for ch in text.chars() {
                    self.pending_events.push(match ch {
                        '\n' => key_event(egui::Key::Enter, egui::Modifiers::NONE),
                        '\t' => key_event(egui::Key::Tab, egui::Modifiers::NONE),
                        c => egui::Event::Text(c.to_string()),
                    });
                }
            }
            AutoStep::Key {
                key,
                shift,
                command,
            } => {
                let mut m = egui::Modifiers::NONE;
                m.shift = shift;
                if command {
                    m |= egui::Modifiers::COMMAND;
                }
                self.pending_events.push(key_event(key, m));
                self.pending_events.push(egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: m,
                });
            }
            AutoStep::Goto(line, col) => {
                app.handle_action(
                    IdeAction::NavigateToLocation {
                        tab: None,
                        line,
                        col,
                    },
                    ctx,
                );
            }
            AutoStep::Select(start, end) => {
                if let Some(tab) = app.editor.active_tab_mut() {
                    tab.select(start, end);
                }
            }
            AutoStep::FontSize(size) => app.state.editor_font_size = size,
            AutoStep::Resize(w, h) => {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w, h)));
            }
            AutoStep::WaitMs(_) => unreachable!("handled in pump"),
            AutoStep::WindowScreenshot(path) => {
                self.pending_screenshot = Some(path);
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            AutoStep::SelectTab(i) => {
                if i < app.editor.tabs.len() {
                    app.editor.active_tab_index = i;
                }
            }
            AutoStep::Dock(tab) => {
                use crate::ide::DockTab;
                app.state.show_bottom_dock = true;
                app.state.active_dock_tab = match tab.as_str() {
                    "problems" => DockTab::Problems,
                    "variables" => DockTab::Variables,
                    "callstack" => DockTab::CallStack,
                    _ => DockTab::Output,
                };
            }
            AutoStep::Layout(mode) => app.handle_action(
                IdeAction::SetLayout(match mode.as_str() {
                    "editor" => super::layout::WorkspaceLayout::Editor,
                    "calculator" => super::layout::WorkspaceLayout::Calculator,
                    _ => super::layout::WorkspaceLayout::Split,
                }),
                ctx,
            ),
            AutoStep::CalculatorMode(mode) => app.handle_action(
                IdeAction::SetCalculatorMode(if mode == "screen" {
                    super::layout::CalculatorMode::ScreenOnly
                } else {
                    super::layout::CalculatorMode::Full
                }),
                ctx,
            ),
            AutoStep::Backend(firmware) => {
                app.handle_action(IdeAction::SelectBackend(firmware), ctx)
            }
            AutoStep::Exit => self.finish(app, ctx),
        }
    }

    /// Dumps the observable IDE state and closes the window.
    fn finish(&mut self, app: &mut IdeDesktopApp, ctx: &egui::Context) {
        if self.dumped {
            return;
        }
        self.dumped = true;
        // Automation is not an interactive user: allow closing even with
        // unsaved buffers (the dump preserves their content).
        app.close_confirmed = true;
        let dump = self.dump_state(app, ctx);
        if let Some(parent) = self.dump_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(&self.dump_path, dump) {
            eprintln!("automation: failed to write dump: {e}");
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn dump_state(&mut self, app: &mut IdeDesktopApp, ctx: &egui::Context) -> String {
        use serde_json::json;

        let states: Vec<String> = (0..app.editor.tabs.len())
            .map(|i| format!("{:?}", app.analysis_state(i)))
            .collect();
        let tabs: Vec<_> = app
            .editor
            .tabs
            .iter_mut()
            .zip(states)
            .map(|(t, analysis_state)| {
                let diagnostics: Vec<_> = t
                    .current_diagnostics()
                    .iter()
                    .map(|d| {
                        json!({
                            "line": d.line, "col": d.col,
                            "message": d.message,
                            "code": d.code,
                            "class": d.class.label(),
                            "is_error": matches!(d.severity, ppl_syntax::diag::Severity::Error),
                        })
                    })
                    .collect();
                json!({
                    "title": t.title,
                    "modified": t.modified,
                    "path": t.path.as_ref().map(|p| p.display().to_string()),
                    "content": t.content,
                    "breakpoints": t.breakpoints.iter().collect::<Vec<_>>(),
                    "analysis_state": analysis_state,
                    "diagnostics": diagnostics,
                    "cache": {
                        "analyses": t.doc.stats.analyses,
                        "layouts": t.doc.stats.layouts,
                    },
                })
            })
            .collect();

        let state = &app.state;
        let terminal = app
            .controller
            .shared
            .terminal_lines
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let outcome = match app.controller.outcome() {
            Some(crate::ide::execution::WorkerOutcome::Success {
                value_str,
                steps,
                elapsed,
            }) => json!({
                "kind": "success", "value": value_str, "steps": steps,
                "elapsed_ms": elapsed.as_millis() as u64,
            }),
            Some(crate::ide::execution::WorkerOutcome::Error(e)) => json!({
                "kind": "error", "message": e.to_string(),
            }),
            None => json!({"kind": "none"}),
        };

        json!({
            "vm_state": state.vm_state.label(),
            "status_message": state.status_message,
            "show_compare": state.show_compare,
            "screen_comparison": app.compare.snapshot_summary(),
            "cursor_line": state.cursor_line,
            "cursor_col": state.cursor_col,
            "paused_line": state.paused_line,
            "worker_alive": app.controller.worker_alive(),
            "worker_outcome": outcome,
            "console": app.console.messages.iter().map(|m| json!({
                "text": m.text, "is_error": m.is_error, "is_diagnostic": m.is_diagnostic,
            })).collect::<Vec<_>>(),
            "terminal": terminal,
            "problems": app.problems.items.iter().map(|p| json!({
                "line": p.line, "col": p.col, "message": p.message,
                "code": p.code, "class": p.class.label(), "file": p.file,
                "is_error": matches!(p.severity, ppl_syntax::diag::Severity::Error),
            })).collect::<Vec<_>>(),
            "problems_title": app.problems.title(),
            "analysis": format!("{:?}", state.analysis),
            "editor_font_size": state.editor_font_size,
            "panel_sizes": {
                "explorer": state.explorer_width,
                "calculator": state.calculator_width,
                "bottom": state.bottom_height,
            },
            "cursor_char": app.editor.active_tab().and_then(|t| {
                let st = egui::text_edit::TextEditState::load(
                    ctx,
                    crate::ide::editor::view::edit_id(t.id),
                )?;
                st.cursor.char_range().map(|r| r.primary.index)
            }),
            "window_captures": self.captures,
            "window": {
                "focused": ctx.input(|i| i.focused),
                "screen_points": [ctx.screen_rect().width(), ctx.screen_rect().height()],
                "pixels_per_point": ctx.pixels_per_point(),
            },
            "tabs": tabs,
            "active_tab": app.editor.active_tab_index,
            "recent_files": state.recent_files.iter()
                .map(|p| p.display().to_string()).collect::<Vec<_>>(),
            "screen_generation": app.controller.shared.screen_generation
                .load(std::sync::atomic::Ordering::SeqCst),
            "display_generation": app.display.last_generation,
            "native_view_generation": app.native_display_generation(),
            "native_session":app.native_input_snapshot(),
            "calculator_mode": format!("{:?}", state.calculator_mode),
            "layout": format!("{:?}", state.layout),
            "lcd_rect": {
                "min": [app.display.last_display_rect.min.x, app.display.last_display_rect.min.y],
                "max": [app.display.last_display_rect.max.x, app.display.last_display_rect.max.y],
            },
        })
        .to_string()
    }
}

fn key_event(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

/// Writes an RGBA screenshot as PNG.
fn save_color_image(image: &egui::ColorImage, path: &std::path::Path) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(
        std::io::BufWriter::new(file),
        image.size[0] as u32,
        image.size[1] as u32,
    );
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    w.write_image_data(image.as_raw())
        .map_err(|e| e.to_string())
}

/// Injects scripted input at the start of a frame, before any widget reads
/// it (no-op when not configured).
pub fn inject_raw_input(app: &mut IdeDesktopApp, input: &mut egui::RawInput) {
    if !input.focused {
        return;
    }
    if let Some(auto) = app.automation.as_mut() {
        if let Some(pos) = auto.pending_pointer_release.take() {
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
        }
        if !auto.pending_events.is_empty() {
            let events = std::mem::take(&mut auto.pending_events);
            for event in &events {
                if let egui::Event::PointerButton {
                    pos, pressed: true, ..
                } = event
                {
                    auto.pending_pointer_release = Some(*pos);
                }
            }
            input.events.extend(events);
        }
    }
}

/// Runs the automation (no-op when not configured).
pub fn maybe_pump(app: &mut IdeDesktopApp, ctx: &egui::Context) {
    if let Some(mut auto) = app.automation.take() {
        auto.pump(app, ctx);
        if !auto.dumped {
            app.automation = Some(auto);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_layout_passes_do_not_merge_independent_input_steps() {
        let ctx = egui::Context::default();
        let mut app = IdeDesktopApp::default();
        let mut automation = Automation {
            steps: vec![
                AutoStep::Key {
                    key: egui::Key::Escape,
                    shift: false,
                    command: false,
                },
                AutoStep::Key {
                    key: egui::Key::Enter,
                    shift: false,
                    command: false,
                },
            ],
            next: 0,
            dump_path: PathBuf::new(),
            wait_deadline: None,
            dumped: false,
            pending_events: Vec::new(),
            pending_pointer_release: None,
            pending_screenshot: None,
            captures: Vec::new(),
            last_pump_frame: None,
        };
        let output = ctx.run(Default::default(), |ctx| {
            automation.pump(&mut app, ctx);
            ctx.request_discard("exercise a repeated layout pass");
        });
        assert!(output.platform_output.num_completed_passes > 1);
        assert_eq!(automation.next, 1);
        assert_eq!(automation.pending_events.len(), 2);
        assert!(automation.pending_events.iter().all(|e| matches!(
            e,
            egui::Event::Key {
                key: egui::Key::Escape,
                ..
            }
        )));
        automation.pending_events.clear();
        let _ = ctx.run(Default::default(), |ctx| automation.pump(&mut app, ctx));
        assert_eq!(automation.next, 2);
        assert!(automation.pending_events.iter().all(|e| matches!(
            e,
            egui::Event::Key {
                key: egui::Key::Enter,
                ..
            }
        )));

        app.automation = Some(automation);
        let mut input = egui::RawInput {
            focused: false,
            ..Default::default()
        };
        inject_raw_input(&mut app, &mut input);
        assert!(input.events.is_empty());
        assert_eq!(app.automation.as_ref().unwrap().pending_events.len(), 2);
        input.focused = true;
        inject_raw_input(&mut app, &mut input);
        assert_eq!(input.events.len(), 2);
        assert!(app.automation.as_ref().unwrap().pending_events.is_empty());
    }
}
