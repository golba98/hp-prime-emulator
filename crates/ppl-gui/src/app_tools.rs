//! Native tools integration and durable profile lifecycle.
use super::*;
use crate::ide::native_tools::{NativeTools, Page, Request};
use serde_json::json;
use std::sync::mpsc;

fn native_profile_ui(
    native: &crate::ide::native::NativeUiState,
    tools: &crate::ide::native_tools::ToolMemory,
) -> serde_json::Value {
    json!({"history":native.history,"home_input":native.home_input,"tools":tools,"cas_mode":native.cas_mode})
}

pub(super) struct ModelTask {
    thread: Option<std::thread::JoinHandle<()>>,
    _owner: crate::ide::execution::worker::SessionLease,
    rx: mpsc::Receiver<Result<serde_json::Value, String>>,
    calculation: crate::ide::native_app_ui::Calculation,
    source: serde_json::Value,
    statistics2_columns_revision: u64,
}
pub(super) struct CasTask {
    catalog: bool,
    deleted: Option<String>,
    worksheet_assignment: Option<(String, serde_json::Value)>,
    thread: Option<std::thread::JoinHandle<()>>,
    owner: crate::ide::execution::worker::SessionLease,
    rx: mpsc::Receiver<(
        ppl_cas::CasSession,
        String,
        Result<ppl_cas::CasEvaluation, ppl_cas::CasError>,
    )>,
}
impl IdeDesktopApp {
    pub(super) fn route_native_tool_action(
        &mut self,
        action: crate::ide::native::keys::KeyAction,
        ctx: &egui::Context,
    ) -> bool {
        use crate::ide::native::keys::KeyAction;
        if let Some(requests) = self.native_tools.help_action(action) {
            for request in requests {
                self.handle_native_tool_request(request, ctx);
            }
            ctx.request_repaint();
            return true;
        }
        if let Some(requests) = self
            .native_tools
            .home_settings_action(action, &self.controller.session_snapshot().settings)
        {
            for request in requests {
                self.handle_native_tool_request(request, ctx);
            }
            self.focus_native_calculator(ctx);
            ctx.request_repaint();
            return true;
        }
        if let Some(requests) = self.native_tools.geometry_action(action) {
            for request in requests {
                self.handle_native_tool_request(request, ctx);
            }
            ctx.request_repaint();
            return true;
        }
        if let Some(requests) = self.native_tools.spreadsheet_action(action) {
            for request in requests {
                self.handle_native_tool_request(request, ctx);
            }
            ctx.request_repaint();
            return true;
        }
        if let Some(requests) = self.native_tools.statistics_action(action) {
            for request in requests {
                self.handle_native_tool_request(request, ctx);
            }
            ctx.request_repaint();
            return true;
        }
        if let Some(requests) = self
            .native_tools
            .function_numeric_action(action, &self.controller.session_snapshot())
        {
            for request in requests {
                self.handle_native_tool_request(request, ctx);
            }
            ctx.request_repaint();
            return true;
        }
        if let Some(requests) = self
            .native_tools
            .function_view_action(action, &self.controller.session_snapshot())
        {
            for request in requests {
                self.handle_native_tool_request(request, ctx);
            }
            ctx.request_repaint();
            return true;
        }
        if let Some(requests) = self.native_tools.graph_symbolic_action(action) {
            for request in requests {
                self.handle_native_tool_request(request, ctx);
            }
            ctx.request_repaint();
            return true;
        }
        if let Some(requests) = self.native_tools.gallery_action(action) {
            for request in requests {
                self.handle_native_tool_request(request, ctx);
            }
            ctx.request_repaint();
            return true;
        }
        if let Some(requests) = self.native_tools.template_action(action) {
            for request in requests {
                self.handle_native_tool_request(request, ctx);
            }
            ctx.request_repaint();
            return true;
        }
        if self.native_tools.picker_owns_navigation(ctx) {
            if let Some(requests) = self.native_tools.picker_action(action) {
                for request in requests {
                    self.handle_native_tool_request(request, ctx);
                }
                ctx.request_repaint();
                return true;
            }
        }
        if self.native_tools.page == Some(Page::Characters)
            && ctx.memory(|memory| memory.focused()).is_none()
            && !egui::Popup::is_any_open(ctx)
        {
            use crate::ide::native::expression::EditCommand as E;
            let delta = match action {
                KeyAction::Edit(E::Left(false)) => Some(-1),
                KeyAction::Edit(E::Right(false)) => Some(1),
                KeyAction::History(delta) => Some(if delta < 0 { -16 } else { 16 }),
                _ => None,
            };
            if let Some(delta) = delta {
                self.native_tools.memory.characters.move_selection(delta);
                self.native_tools.dirty();
                ctx.request_repaint();
                return true;
            }
            if action == KeyAction::Evaluate {
                let text = self.native_tools.memory.characters.selected();
                self.handle_native_tool_request(Request::Insert(text), ctx);
                self.focus_native_calculator(ctx);
                return true;
            }
        }
        if self.native_tools.graph3d_key(action) {
            ctx.request_repaint();
            return true;
        }
        if self
            .native_tools
            .statistics_plot_action(action, &self.controller.session_snapshot())
        {
            ctx.request_repaint();
            return true;
        }
        if matches!(
            self.native_tools.page,
            Some(Page::Plot | Page::PlotDetail | Page::PlotTable)
        ) {
            use crate::ide::native::expression::EditCommand as E;
            use crate::ide::native_plot_controls::TraceDirection as D;
            let direction = match action {
                KeyAction::Edit(E::Left(false)) => Some(D::Left),
                KeyAction::Edit(E::Right(false)) => Some(D::Right),
                KeyAction::History(delta) => Some(if delta < 0 { D::Up } else { D::Down }),
                _ => None,
            };
            if let Some(direction) = direction {
                self.native_tools.trace_step(direction);
                ctx.request_repaint();
                return true;
            }
            if matches!(action, KeyAction::Insert("+" | "-")) {
                let factor = if action == KeyAction::Insert("+") {
                    0.5
                } else {
                    2.
                };
                if let Some(app) = self
                    .native_tools
                    .memory
                    .graphs
                    .get_mut(&self.native_tools.memory.selected_app)
                {
                    app.window.zoom(factor);
                    self.native_tools.plot(self.controller.session_snapshot());
                } else {
                    self.native_tools
                        .memory
                        .model_windows
                        .entry(self.native_tools.memory.selected_app.clone())
                        .or_default()
                        .zoom(factor);
                    if matches!(
                        self.native_tools.memory.selected_app.as_str(),
                        "Solve" | "Statistics 2Var"
                    ) {
                        self.native_tools.plot(self.controller.session_snapshot());
                    }
                }
                self.native_tools.dirty();
                ctx.request_repaint();
                return true;
            }
        }
        if self.native_tools.page == Some(Page::Base) && ctx.memory(|m| m.focused()).is_none() {
            use crate::ide::native::expression::EditCommand as E;
            let handled = if let Some(word) = &mut self.native_tools.integer_edit {
                match action {
                    KeyAction::Edit(E::Left(_)) => {
                        word.shift(true);
                        true
                    }
                    KeyAction::Edit(E::Right(_)) => {
                        word.shift(false);
                        true
                    }
                    KeyAction::History(delta) => {
                        if let Err(error) = word.resize(if delta < 0 {
                            word.bits.saturating_add(1)
                        } else {
                            word.bits.saturating_sub(1)
                        }) {
                            self.native_tools.error = Some(error);
                        }
                        true
                    }
                    KeyAction::Insert("+") => {
                        word.cycle(true);
                        true
                    }
                    KeyAction::Insert("-") => {
                        word.cycle(false);
                        true
                    }
                    KeyAction::Sign => {
                        word.negate();
                        true
                    }
                    _ => false,
                }
            } else {
                false
            };
            if handled {
                if let Some(word) = &self.native_tools.integer_edit {
                    self.native_tools.integer_decimal_draft = word.decimal();
                    self.native_tools.integer_hex_draft = format!("{:X}", word.raw);
                }
                self.native_tools.dirty();
                ctx.request_repaint();
                return true;
            }
            if action == KeyAction::Evaluate {
                self.handle_native_tool_request(Request::SaveInteger, ctx);
                return true;
            }
            if action == KeyAction::Back {
                self.handle_native_tool_request(Request::CancelInteger, ctx);
                return true;
            }
        }
        let event = match action {
            KeyAction::Insert(text) => Some(egui::Event::Text(text.into())),
            KeyAction::Template(_) | KeyAction::Fraction => {
                let template = if let KeyAction::Template(template) = action {
                    template
                } else {
                    crate::ide::native::expression::TemplateKind::Fraction
                };
                let mut editor = crate::ide::native::expression::ExpressionEditor::default();
                editor.insert_template(template);
                let left = editor.text()[editor.cursor()..].chars().count();
                ctx.input_mut(|input| {
                    input.events.push(egui::Event::Text(editor.text().into()));
                    for _ in 0..left {
                        input.events.push(egui::Event::Key {
                            key: egui::Key::ArrowLeft,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::NONE,
                        });
                    }
                });
                ctx.request_repaint();
                return true;
            }
            KeyAction::History(delta)
                if ctx.memory(|m| m.focused()).is_none()
                    && matches!(
                        self.native_tools.page,
                        Some(
                            Page::Programs
                                | Page::Notes
                                | Page::CasVariables
                                | Page::Variables
                                | Page::Lists
                                | Page::Matrices
                        )
                    ) =>
            {
                let snapshot = self.controller.session_snapshot();
                let page = self.native_tools.page.unwrap();
                let objects = match page {
                    Page::Programs => snapshot.programs,
                    Page::Notes => snapshot.notes,
                    Page::CasVariables => snapshot.cas.bindings,
                    _ => snapshot
                        .globals
                        .iter()
                        .filter(|(_, value)| match page {
                            Page::Lists => matches!(value, ppl_vm::PortableValue::List(_)),
                            Page::Matrices => matches!(value, ppl_vm::PortableValue::Matrix { .. }),
                            _ => true,
                        })
                        .map(|(name, value)| (name.clone(), value.display(&snapshot.settings)))
                        .collect(),
                };
                let names = objects.keys().collect::<Vec<_>>();
                if !names.is_empty() {
                    let current = names
                        .iter()
                        .position(|name| ***name == self.native_tools.memory.name)
                        .unwrap_or(0);
                    let next = if delta < 0 {
                        current.saturating_sub(1)
                    } else {
                        (current + 1).min(names.len() - 1)
                    };
                    let name = names[next].clone();
                    self.native_tools.memory.name = name.clone();
                    self.native_tools.memory.original_name = name.clone();
                    self.native_tools.memory.content = objects[&name].clone();
                    if page == Page::Notes {
                        self.native_tools
                            .load_note_bullets(ctx, snapshot.note_bullets.get(&name));
                    } else {
                        self.native_tools.clear_note_bullets();
                    }
                    self.native_tools.dirty();
                }
                return true;
            }
            KeyAction::Evaluate
                if ctx.memory(|m| m.focused()).is_none()
                    && matches!(
                        self.native_tools.page,
                        Some(Page::Variables | Page::CasVariables | Page::Lists | Page::Matrices)
                    ) =>
            {
                self.open_math_entry(crate::ide::native_tools::MathTarget::Object, ctx);
                return true;
            }
            KeyAction::Evaluate
                if ctx.memory(|m| m.focused()).is_none()
                    && matches!(self.native_tools.page, Some(Page::Programs | Page::Notes)) =>
            {
                ctx.memory_mut(|m| m.request_focus(egui::Id::new("native_catalog_content")));
                return true;
            }
            KeyAction::Polar => Some(egui::Event::Text("∠".into())),
            KeyAction::Sexagesimal => {
                self.native_tools.open(Page::Sexagesimal);
                return true;
            }
            KeyAction::Sign => Some(egui::Event::Text("-".into())),
            KeyAction::Evaluate | KeyAction::Approximate | KeyAction::History(_) => {
                let key = match action {
                    KeyAction::History(n) if n < 0 => egui::Key::ArrowUp,
                    KeyAction::History(_) => egui::Key::ArrowDown,
                    _ => egui::Key::Enter,
                };
                Some(egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                })
            }
            KeyAction::Copy => Some(egui::Event::Copy),
            KeyAction::Paste => {
                ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                return true;
            }
            KeyAction::Programs => {
                self.native_tools.open(Page::Programs);
                return true;
            }
            KeyAction::Back => {
                self.native_tools.close();
                self.set_native_view(NativeView::Home, ctx);
                return true;
            }
            KeyAction::Home => {
                self.native_tools.close();
                self.switch_native_environment(false, ctx);
                return true;
            }
            KeyAction::On => {
                self.native_tools.cancel_calculations();
                self.native_tools.close();
                self.set_native_view(NativeView::Home, ctx);
                return true;
            }
            KeyAction::Tool(_) | KeyAction::Off => return false,
            KeyAction::Edit(command) => {
                use crate::ide::native::expression::EditCommand as E;
                let key = match command {
                    E::Left(_) => egui::Key::ArrowLeft,
                    E::Right(_) => egui::Key::ArrowRight,
                    E::Home(_) => egui::Key::Home,
                    E::End(_) => egui::Key::End,
                    E::Backspace => egui::Key::Backspace,
                    E::Delete => egui::Key::Delete,
                    E::SelectAll => egui::Key::A,
                    E::Undo | E::Redo => egui::Key::Z,
                    E::Clear => {
                        ctx.input_mut(|i| {
                            i.events.push(egui::Event::Key {
                                key: egui::Key::A,
                                physical_key: None,
                                pressed: true,
                                repeat: false,
                                modifiers: egui::Modifiers::COMMAND,
                            });
                        });
                        egui::Key::Backspace
                    }
                };
                Some(egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: match command {
                        E::Left(true) | E::Right(true) | E::Home(true) | E::End(true) => {
                            egui::Modifiers::SHIFT
                        }
                        E::SelectAll | E::Undo => egui::Modifiers::COMMAND,
                        E::Redo => egui::Modifiers {
                            shift: true,
                            ..egui::Modifiers::COMMAND
                        },
                        _ => egui::Modifiers::NONE,
                    },
                })
            }
            _ => return false,
        };
        if let Some(event) = event {
            ctx.input_mut(|i| i.events.push(event));
            ctx.request_repaint();
        }
        true
    }
    pub(crate) fn switch_native_environment(&mut self, cas: bool, ctx: &egui::Context) {
        if self.math_entry.is_some() {
            self.finish_math_entry(false, ctx);
        }
        if self.cas_task.is_some() || self.controller.running() {
            return;
        }
        self.native_tools.close();
        if self.state.native.cas_mode != cas {
            let memory = &mut self.native_tools.memory;
            if self.state.native.cas_mode {
                memory.cas_history = self.state.native.history.clone();
                memory.cas_draft = self.state.native.home_input.clone();
            } else {
                memory.home_history = self.state.native.history.clone();
                memory.home_draft = self.state.native.home_input.clone();
            }
            self.state.native.history = if cas {
                memory.cas_history.clone()
            } else {
                memory.home_history.clone()
            };
            let text = if cas {
                memory.cas_draft.clone()
            } else {
                memory.home_draft.clone()
            };
            self.state.native.edit(|e| e.set_text(&text));
        }
        self.state.native.history_position = None;
        self.state.native.history_viewer = None;
        self.state.native.cas_mode = cas;
        self.state.native.alpha_default_lower = cas;
        self.set_native_view(NativeView::Home, ctx);
    }
    pub(super) fn consume_native_tool(&mut self, ctx: &egui::Context) {
        use crate::ide::native::keys::Tool;
        let Some(tool) = self.state.native.pending_tool.take() else {
            return;
        };
        let page = match tool {
            Tool::Cas => {
                self.switch_native_environment(true, ctx);
                return;
            }
            Tool::Apps => Page::Apps,
            Tool::Symbolic => Page::Symbolic,
            Tool::Plot => Page::Plot,
            Tool::Numeric => Page::Numeric,
            Tool::PlotSetup => Page::PlotSetup,
            Tool::NumericSetup => Page::NumericSetup,
            Tool::HomeSettings => Page::Settings,
            Tool::SymbolicSetup => Page::SymbolicSetup,
            Tool::CasSettings => Page::CasSettings,
            Tool::Help => {
                if self.native_tools.page == Some(Page::Toolbox) {
                    self.native_tools.memory.help_topic =
                        self.native_tools.memory.commands.selected.clone();
                    self.native_tools.memory.help_query =
                        self.native_tools.memory.help_topic.clone();
                }
                Page::Help
            }
            Tool::Info => Page::Info,
            Tool::Variables => {
                if self.state.native.cas_mode {
                    Page::CasVariables
                } else {
                    Page::Variables
                }
            }
            Tool::Toolbox | Tool::Menu => Page::Toolbox,
            Tool::View => Page::Views,
            Tool::Templates => Page::Templates,
            Tool::Sexagesimal => Page::Sexagesimal,
            Tool::Characters | Tool::Symbols | Tool::Relations => Page::Characters,
            Tool::Lists => Page::Lists,
            Tool::Matrices => Page::Matrices,
            Tool::Notes => Page::Notes,
            Tool::Memory => Page::Memory,
            Tool::Units => Page::Units,
            Tool::Define => Page::Define,
            Tool::Base => {
                let snapshot = self.controller.session_snapshot();
                let selected = if self.state.native.history_result {
                    self.state.native.history_position.map(|index| {
                        self.state
                            .native
                            .history
                            .get(index)
                            .filter(|e| !e.error)
                            .and_then(|e| e.typed_result.as_ref())
                            .and_then(crate::ide::native::home_format::recover_value)
                    })
                } else {
                    None
                };
                let value = match selected {
                    Some(Some(value)) => value,
                    Some(None) => {
                        self.state.status_message =
                            "The selected result has no recoverable integer value".into();
                        return;
                    }
                    None => snapshot.globals["Ans"].clone(),
                };
                match crate::ide::native_integer::IntegerEdit::from_value(
                    &value,
                    &snapshot.settings,
                ) {
                    Ok(word) => {
                        let key = format!("{:?}", value);
                        let mut word = self
                            .native_tools
                            .memory
                            .integer_edits
                            .get(&key)
                            .cloned()
                            .unwrap_or(word);
                        word.sync_system_base(snapshot.settings.int_base);
                        self.native_tools.integer_decimal_draft = word.decimal();
                        self.native_tools.integer_hex_draft = format!("{:X}", word.raw);
                        self.native_tools.integer_key = key;
                        self.native_tools.integer_original = Some(word.clone());
                        self.native_tools.integer_edit = Some(word);
                    }
                    Err(error) => {
                        self.state.native.error = Some(error.clone());
                        self.state.status_message = error;
                        return;
                    }
                }
                Page::Base
            }
            Tool::User => {
                self.state.native.user_keys.mode.activate();
                self.state.native.touch_view();
                ctx.request_repaint();
                return;
            }
        };
        self.native_tools.open(page);
        self.focus_native_calculator(ctx);
        if self.native_tools.page != Some(page) {
            return;
        }
        if page == Page::Plot {
            self.native_tools.plot(self.controller.session_snapshot());
        } else if page == Page::Numeric {
            self.native_tools.table(&self.controller.session_snapshot());
        }
    }
    pub(super) fn stop_calculator_tasks(&mut self) {
        self.native_tools.cancel_calculations();
        self.controller.stop();
        for thread in [
            self.cas_task.take().and_then(|mut t| t.thread.take()),
            self.model_task.take().and_then(|mut t| t.thread.take()),
        ]
        .into_iter()
        .flatten()
        {
            let deadline = Instant::now() + Duration::from_secs(2);
            while !thread.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(2));
            }
            if thread.is_finished() {
                let _ = thread.join();
            } else {
                eprintln!("Calculator worker did not stop within 2 seconds");
            }
        }
    }
    fn promote_legacy_graph3d_plot_settings(&self) -> Result<(), String> {
        use crate::ide::native_graph3d::{Book, NAME};

        let stored = &self.native_tools.memory.graph3d;
        let defaults = Book::default();
        if stored.bounds == defaults.bounds && stored.ticks == defaults.ticks {
            return Ok(());
        }
        if self
            .controller
            .session_snapshot()
            .app_plot_settings
            .contains_key(NAME)
        {
            return Ok(());
        }
        let settings = stored.plot_settings()?;
        self.controller.mutate_memory(|memory| {
            memory
                .set_app_plot_settings(NAME, settings)
                .map_err(|error| error.to_string())
        })
    }

    fn promote_legacy_geometry_window(&mut self) -> Result<(), String> {
        if self
            .controller
            .session_snapshot()
            .geometry_plot_settings
            .is_some()
        {
            return Ok(());
        }
        let Some(window) = self
            .native_tools
            .memory
            .model_windows
            .get("Geometry")
            .cloned()
        else {
            return Ok(());
        };
        // Keep the original camera even if it exceeds the new coordinate range.
        self.native_tools
            .memory
            .geometry_legacy_window
            .get_or_insert(window.clone());
        let settings = NativeTools::geometry_settings_for_legacy_window(&window)?;
        self.controller.mutate_memory(|memory| {
            memory
                .set_geometry_plot_settings(settings.clone())
                .map_err(|error| error.to_string())
        })?;
        self.native_tools.geometry_settings_changed(&settings);
        Ok(())
    }

    pub(super) fn initialize_native_memory(&mut self) {
        let Some(path) = crate::ide::native_profile::default_directory() else {
            self.profile_problem =
                Some("No Native profile directory is configured; memory is unsaved".into());
            return;
        };
        match self.controller.open_native_profile(&path) {
            Ok(ui) => {
                if let Some(history) = ui["history"].as_array() {
                    for entry in history {
                        if let (Some(expression), Some(result), Some(error)) = (
                            entry["expression"].as_str(),
                            entry["result"].as_str(),
                            entry["error"].as_bool(),
                        ) {
                            self.state
                                .native
                                .history
                                .push_back(crate::ide::native::HomeEntry {
                                    home_display: entry
                                        .get("home_display")
                                        .and_then(|v| serde_json::from_value(v.clone()).ok()),
                                    expression: expression.into(),
                                    result: result.into(),
                                    error,
                                    expression_deleted: entry["expression_deleted"]
                                        .as_bool()
                                        .unwrap_or(false),
                                    result_deleted: entry["result_deleted"]
                                        .as_bool()
                                        .unwrap_or(false),
                                    typed_result: entry
                                        .get("typed_result")
                                        .filter(|v| !v.is_null())
                                        .cloned(),
                                    presentation: entry
                                        .get("presentation")
                                        .and_then(|value| {
                                            serde_json::from_value(value.clone()).ok()
                                        })
                                        .unwrap_or_else(|| {
                                            crate::ide::native::HomeEntry::legacy_presentation(
                                                result,
                                                &entry["typed_result"],
                                            )
                                        }),
                                });
                        }
                    }
                }
                self.state.native.home_input = ui["home_input"].as_str().unwrap_or_default().into();
                if let Some(tools) = ui.get("tools") {
                    match serde_json::from_value(tools.clone()) {
                        Ok(memory) => self.native_tools.memory = memory,
                        Err(e) => self.native_tools.error = Some(format!("Saved tool state: {e}")),
                    }
                }
                for (name, app) in crate::ide::native_tools::ToolMemory::default().graphs {
                    self.native_tools.memory.graphs.entry(name).or_insert(app);
                }
                if let Err(error) = self.promote_legacy_geometry_window() {
                    self.native_tools.error = Some(error);
                }
                if let Err(error) = self.promote_legacy_graph3d_plot_settings() {
                    self.native_tools.error = Some(error);
                }
                self.sync_native_statistics_memory();
                self.sync_native_presentation();
                self.state.native.cas_mode = ui["cas_mode"].as_bool().unwrap_or(false);
                self.state.native.alpha_default_lower = self.state.native.cas_mode;
                self.state.native.touch_view();
            }
            Err(error) => {
                self.console.print_error(&error);
                self.profile_problem = Some(format!("Native memory is unsaved: {error}"));
                self.state.status_message = error;
            }
        }
    }
    pub(super) fn persist_native_ui(&mut self) {
        if let Err(error) = self.native_tools.memory.validate() {
            self.state.status_message = format!("Calculator app draft was not saved: {error}");
            return;
        }
        let Some(profile) = self.controller.native_profile() else {
            return;
        };
        let ui = native_profile_ui(&self.state.native, &self.native_tools.memory);
        if ui != self.profile_ui_last {
            profile.update("ui", ui.clone());
            self.profile_ui_last = ui;
        }
        if let Some(error) = profile.error() {
            self.state.status_message = format!("Native memory not saved: {error}");
        }
    }
    fn sync_native_presentation(&mut self) {
        let settings = self.controller.session_snapshot().settings;
        let mut repainted = false;
        let stamp = format!(
            "{:?}:{}:{}",
            settings.home_format(),
            self.state.native.cas_mode,
            self.state.native.history.len()
        );
        if !self.state.native.cas_mode && self.state.native.history_format_stamp != stamp {
            for entry in &mut self.state.native.history {
                repainted |= crate::ide::native::home_format::refresh(entry, &settings);
            }
        }
        self.state.native.history_format_stamp = stamp;
        let pixels = self.native_tools.memory.presentation.font_size.pixels()
            - if self.native_fonts.selected.is_some() {
                0.
            } else {
                1.
            };
        if self.state.native.history_font_px != pixels {
            self.state.native.history_font_px = pixels;
            repainted = true;
        }
        if repainted {
            self.state.native.touch_view();
        }
        // GuiHost::local_time uses the same UTC SystemTime breakdown.
        let seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let clock = format!("{:02}:{:02}", (seconds / 3600) % 24, (seconds / 60) % 60);
        if self.state.native.status_clock != clock {
            self.state.native.status_clock = clock;
            self.state.native.touch_view();
        }
        if self.state.native.app_title != self.native_tools.memory.selected_app {
            self.state.native.app_title = self.native_tools.memory.selected_app.clone();
            self.state.native.touch_view();
        }
        if self.state.native.presentation != self.native_tools.memory.presentation {
            self.state.native.presentation = self.native_tools.memory.presentation;
            self.state.native.touch_view();
        }
    }
    pub(super) fn poll_native_tools(&mut self, ctx: &egui::Context) {
        self.poll_app_variables(ctx);
        self.native_tools
            .spreadsheet_busy(self.controller.operation_active());
        self.native_tools
            .geometry_busy(self.controller.operation_active());
        self.native_tools
            .graph_symbolic_busy(self.controller.operation_active());
        self.native_tools
            .function_numeric_busy(self.controller.operation_active());
        self.native_tools
            .statistics_busy(self.controller.operation_active());
        self.sync_native_statistics_memory();
        self.sync_native_presentation();
        let seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if !self.state.firmware_backend {
            ctx.request_repaint_after(std::time::Duration::from_secs(60 - seconds % 60));
        }
        let requests = std::mem::take(
            &mut *self
                .controller
                .shared
                .calculator_views
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
        );
        for mut message in requests {
            if message.epoch
                != self
                    .controller
                    .shared
                    .operation_epoch
                    .load(std::sync::atomic::Ordering::Acquire)
            {
                if message.await_plot {
                    self.native_tools.presentation_pending = false;
                }
                let _ = message
                    .reply
                    .send(Err("Stale calculator view request".into()));
                continue;
            }
            if message.await_plot {
                if self.native_tools.calculating() {
                    self.controller
                        .shared
                        .calculator_views
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push_back(message);
                } else {
                    self.native_tools.presentation_pending = false;
                    let opened_invalid_surface = matches!(
                        message.request,
                        ppl_vm::host::CalculatorView::StartView { number: 1, .. }
                    ) && self.native_tools.memory.selected_app
                        == crate::ide::native_graph3d::NAME
                        && self.native_tools.error.is_some()
                        && self.native_tools.error
                            == self.native_tools.memory.graph3d.range_error();
                    let opened_graph3d_without_draw = matches!(
                        message.request,
                        ppl_vm::host::CalculatorView::StartView {
                            number: 1,
                            draw: false
                        }
                    ) && self.native_tools.memory.selected_app
                        == crate::ide::native_graph3d::NAME;
                    let error = self
                        .native_tools
                        .error
                        .clone()
                        .filter(|_| !opened_invalid_surface && !opened_graph3d_without_draw);
                    let outcome =
                        error.map_or_else(|| Ok(self.native_tools.view_outcome(true)), Err);
                    let _ = message.reply.send(outcome);
                }
                continue;
            }
            let committed = self.controller.session_snapshot();
            let committed_generation = committed.generation;
            let mut snapshot = message.context.clone().unwrap_or(committed);
            if message.context.is_some() {
                self.native_tools
                    .apply_view_context(&snapshot, committed_generation);
            }
            let result = match message.request.clone() {
                ppl_vm::host::CalculatorView::StartApp(name) => {
                    if self.native_tools.memory.graphs.contains_key(&name)
                        || crate::ide::native_app_ui::MODEL_APPS.contains(&name.as_str())
                    {
                        self.native_tools.memory.selected_app = name;
                        let page = crate::ide::native_tools::NativeTools::entry_page(
                            &self.native_tools.memory.selected_app,
                        );
                        self.native_tools.open(page);
                        if page == Page::Plot {
                            let geometry = self.native_tools.memory.selected_app == "Geometry";
                            if geometry && snapshot.active_app != "Geometry" {
                                // STARTAPP carries the previous app's live frame,
                                // whose revision is independent of committed memory.
                                snapshot.active_app = "Geometry".into();
                                self.native_tools
                                    .geometry_apply_context(&snapshot, committed_generation);
                            }
                            self.native_tools.plot(snapshot);
                            if geometry {
                                message.await_plot = true;
                                self.native_tools.presentation_pending = true;
                                self.controller
                                    .shared
                                    .calculator_views
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .push_back(message);
                                ctx.request_repaint();
                                continue;
                            }
                        }
                        Ok(true)
                    } else {
                        Ok(false)
                    }
                }
                ppl_vm::host::CalculatorView::StartView { number, draw } => {
                    if number >= 8 {
                        let action = crate::ide::native_tools::special_views::view(
                            &self.native_tools.memory.selected_app,
                            number,
                        );
                        let accepted =
                            self.native_tools
                                .open_special(number, draw, snapshot.clone());
                        if accepted
                            && (draw || action
                                == Some(
                                    crate::ide::native_tools::special_views::SpecialView::Autoscale,
                                ))
                        {
                            message.await_plot = true;
                            self.native_tools.presentation_pending = true;
                            self.controller
                                .shared
                                .calculator_views
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .push_back(message);
                        } else {
                            let _ = message
                                .reply
                                .send(Ok(self.native_tools.view_outcome(accepted)));
                        }
                        ctx.request_repaint();
                        continue;
                    }
                    let page = match number {
                        -1 => None,
                        -2 => Some(Page::Settings),
                        -3 => Some(Page::Memory),
                        -4 => Some(Page::Apps),
                        -5 => Some(Page::Matrices),
                        -6 => Some(Page::Lists),
                        -7 => Some(Page::Programs),
                        -8 => Some(Page::Notes),
                        0 => Some(Page::Symbolic),
                        1 => Some(Page::Plot),
                        2 => Some(Page::Numeric),
                        3 => Some(Page::SymbolicSetup),
                        4 => Some(Page::PlotSetup),
                        5 => Some(Page::NumericSetup),
                        6 => Some(Page::Info),
                        7 => Some(Page::Views),
                        _ => {
                            let _ = message.reply.send(Ok(Default::default()));
                            continue;
                        }
                    };
                    if let Some(page) = page {
                        if !self.native_tools.supports_page(page) {
                            let _ = message.reply.send(Ok(Default::default()));
                            continue;
                        }
                        self.native_tools.open(page);
                        let geometry_redraw = self.native_tools.memory.selected_app == "Geometry"
                            && matches!(page, Page::Symbolic | Page::Plot | Page::Numeric);
                        let graph3d_view = self.native_tools.memory.selected_app
                            == crate::ide::native_graph3d::NAME
                            && matches!(page, Page::Numeric | Page::Plot);
                        // Graph 3D populates Plot even when STARTVIEW omits
                        // the draw flag or explicitly passes zero.
                        if geometry_redraw || (page == Page::Plot && (draw || graph3d_view)) {
                            self.native_tools.plot(snapshot.clone());
                        } else if page == Page::Numeric {
                            self.native_tools.table(&snapshot);
                        }
                        let function_table = page == Page::Numeric
                            && self.native_tools.memory.selected_app == "Function";
                        if geometry_redraw || function_table || graph3d_view {
                            message.await_plot = true;
                            self.native_tools.presentation_pending = true;
                            self.controller
                                .shared
                                .calculator_views
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .push_back(message);
                            ctx.request_repaint();
                            continue;
                        }
                    } else {
                        self.native_tools.close();
                    }
                    Ok(true)
                }
            };
            let _ = message
                .reply
                .send(result.map(|changed| self.native_tools.view_outcome(changed)));
            ctx.request_repaint();
        }
        if let Some(task) = &self.model_task {
            match task.rx.try_recv() {
                Ok(result) => {
                    let task = self.model_task.take().unwrap();
                    if let crate::ide::native_app_ui::Calculation::GeometryPlotField {
                        name,
                        source,
                    } = &task.calculation
                    {
                        if self.geometry_plot_edit_matches(name, source) {
                            let result = result.and_then(|value| {
                                if self.controller.shared.is_interrupted() {
                                    return Err("Geometry Plot Setup edit interrupted".into());
                                }
                                let value = ppl_vm::session::decode_value(&value)?;
                                let mut settings =
                                    self.controller.session_snapshot().geometry_plot_settings();
                                settings
                                    .set(name, &value.to_value())
                                    .map_err(|error| error.to_string())?;
                                self.controller.commit_native_operation(
                                    &task._owner,
                                    |memory| {
                                        memory
                                            .set_geometry_plot_settings(settings.clone())
                                            .map_err(|error| error.to_string())
                                    },
                                )?;
                                Ok(settings)
                            });
                            match result {
                                Ok(settings) => {
                                    self.native_tools.geometry_settings_changed(&settings);
                                    self.finish_math_entry(false, ctx);
                                }
                                Err(error) => {
                                    self.state.native.edit_title = Some(error.clone());
                                    self.state.native.touch_view();
                                    self.native_tools.error = Some(error);
                                }
                            }
                        }
                    } else if self.native_tools.memory.models.input_fingerprint() == task.source {
                        let result = result.and_then(|mut value| {
                            if matches!(&task.calculation, crate::ide::native_app_ui::Calculation::Statistics1Book(..)) {
                                let book: crate::ide::native_statistics::OneBook = serde_json::from_value(value["book"].take()).map_err(|e| e.to_string())?;
                                book.validate()?;
                                self.native_tools.memory.models.statistics1_book = Some(book);
                                value = value["summaries"].take();
                            }
                            if matches!(&task.calculation, crate::ide::native_app_ui::Calculation::StatisticsDefinitionValue { .. }) {
                                let source = value.as_str().ok_or("Invalid statistics expression result")?;
                                self.native_tools.statistics_definition_evaluated(source.to_owned());
                            }
                            if matches!(&task.calculation,crate::ide::native_app_ui::Calculation::GeometryEdit{..}|crate::ide::native_app_ui::Calculation::GeometryMeasurement{..}) {
                                let book:crate::ide::native_geometry::Book=serde_json::from_value(value["book"].take()).map_err(|e|e.to_string())?;
                                book.validate()?;
                                self.native_tools.memory.models.geometry_book=Some(book);
                                self.native_tools.geometry_changed();
                                value=value["scene"].take();
                            }
                            if matches!(&task.calculation,
                                crate::ide::native_app_ui::Calculation::GeometryBook(_)
                                | crate::ide::native_app_ui::Calculation::GeometryEdit { .. }
                                | crate::ide::native_app_ui::Calculation::GeometryMeasurement { .. })
                            {
                                let scene: crate::ide::native_geometry::Scene =
                                    serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
                                if let Some(book) = &mut self.native_tools.memory.models.geometry_book {
                                    book.cache_scene(&scene)?;
                                }
                            }
                            use crate::ide::native_app_ui::Calculation;
                            if matches!(&task.calculation, Calculation::SpreadsheetCopy(..)) {
                                let block = serde_json::from_value(value.take()).map_err(|e| e.to_string())?;
                                let text = self.native_tools.spreadsheet_copied(block);
                                ctx.copy_text(text);
                            }
                            if let Calculation::SpreadsheetChoices(_, address, _) = &task.calculation {
                                let choices = serde_json::from_value(value.take()).map_err(|e| e.to_string())?;
                                self.native_tools.spreadsheet_choices_opened(address, choices);
                            }
                            if matches!(
                                &task.calculation,
                                Calculation::SpreadsheetEntry(..)
                                    | Calculation::SpreadsheetPaste { .. }
                                    | Calculation::SpreadsheetPasteCollection { .. }
                                    | Calculation::SpreadsheetSort { .. }
                                    | Calculation::SpreadsheetSelectChoice(..)
                            ) {
                                let sheet: crate::ide::native_app_models::Spreadsheet =
                                    serde_json::from_value(value["sheet"].take())
                                        .map_err(|e| e.to_string())?;
                                let selection = if matches!(&task.calculation, Calculation::SpreadsheetPasteCollection { .. }) {
                                    serde_json::from_value(value["selection"].take()).map_err(|e| e.to_string())?
                                } else {
                                    None
                                };
                                sheet.validate()?;
                                let models = &mut self.native_tools.memory.models;
                                models.cell_source = sheet
                                    .cells
                                    .get(&models.selected_cell)
                                    .cloned()
                                    .unwrap_or_default();
                                models.spreadsheet = sheet;
                                value = value["values"].take();
                                if let Some(block) = selection {
                                    self.native_tools.spreadsheet_select_block(block);
                                }
                                if let Calculation::SpreadsheetEntry(_, address, ..) = &task.calculation {
                                    self.native_tools.spreadsheet_entry_changed(address);
                                } else {
                                    self.native_tools.spreadsheet_changed(false);
                                }
                            }
                            if let crate::ide::native_app_ui::Calculation::StatisticsCell {
                                two,
                                column,
                                row,
                                ..
                            } = &task.calculation
                            {
                                let number = ppl_vm::session::decode_value(&value)?;
                                self.write_statistics_typed_cell(
                                    *two,
                                    *column,
                                    *row,
                                    number,
                                    Some(&task._owner),
                                )?;
                            }
                            if let crate::ide::native_app_ui::Calculation::StatisticsColumn {
                                two,
                                column,
                                ..
                            } = &task.calculation
                            {
                                let data = ppl_vm::session::decode_value(&value)?;
                                self.write_statistics_typed_column(
                                    *two,
                                    *column, data,
                                    Some(&task._owner),
                                )?;
                            }
                            Ok(value)
                        });
                        match result {
                            Ok(value) => {
                                self.native_tools.error = None;
                                let app_name = task.calculation.result_key();
                                if matches!(&task.calculation, crate::ide::native_app_ui::Calculation::Plot(..) | crate::ide::native_app_ui::Calculation::Statistics1AutoScale(_) | crate::ide::native_app_ui::Calculation::Statistics2AutoScale(_))
                                {
                                    if let Err(error) =
                                        self.native_tools.model_plot_completed(task.calculation.app_name(), &value)
                                    {
                                        self.native_tools.error = Some(error);
                                    }
                                }
                                if let crate::ide::native_app_ui::Calculation::Finance(_, unknown) =
                                    task.calculation
                                {
                                    if let Some(n) = value.as_f64() {
                                        use crate::ide::native_app_models::FinanceUnknown as U;
                                        self.native_tools
                                            .memory
                                            .models
                                            .results
                                            .remove("Finance:plot");
                                        self.native_tools
                                            .memory
                                            .models
                                            .results
                                            .remove("Finance:amortization");
                                        let model = &mut self.native_tools.memory.models.finance;
                                        match unknown {
                                            U::Periods => model.periods = n,
                                            U::AnnualRate => model.annual_rate = n,
                                            U::PresentValue => model.present_value = n,
                                            U::Payment => model.payment = n,
                                            U::FutureValue => model.future_value = n,
                                        }
                                    }
                                }
                                if matches!(
                                    task.calculation,
                                    crate::ide::native_app_ui::Calculation::Statistics2Book(_)
                                ) && let Ok(results) = serde_json::from_value(value.clone())
                                    && let Some(book) =
                                        &mut self.native_tools.memory.models.statistics2_book
                                {
                                    book.remember_fit_equations(
                                        &results,
                                        task.statistics2_columns_revision,
                                    );
                                }
                                if !matches!(
                                    task.calculation,
                                    crate::ide::native_app_ui::Calculation::SpreadsheetCopy(..)
                                        | crate::ide::native_app_ui::Calculation::SpreadsheetChoices(..)
                                ) {
                                    self.native_tools
                                        .memory
                                        .models
                                        .results
                                        .insert(app_name, value);
                                }
                            }
                            Err(error) => self.native_tools.error = Some(error),
                        }
                    }
                    self.state.vm_state = crate::ide::VmExecutionState::Idle;
                    self.native_tools.statistics_busy(false);
                    self.native_tools.geometry_busy(false);
                    self.native_tools.dirty();
                    ctx.request_repaint();
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.state.vm_state = crate::ide::VmExecutionState::Running;
                    ctx.request_repaint_after(Duration::from_millis(16));
                }
                Err(_) => {
                    self.model_task = None;
                    self.native_tools.error = Some("App worker stopped without a result".into());
                    self.state.vm_state = crate::ide::VmExecutionState::Idle;
                }
            }
        }
        if let Some(task) = &self.cas_task {
            match task.rx.try_recv() {
                Ok((session, expression, mut result)) => {
                    let task = self.cas_task.take().unwrap();
                    if self.controller.shared.is_interrupted() {
                        result = Err(ppl_cas::CasError::Cancelled);
                    }
                    let mut worksheet = None;
                    if let (Some((cell, source)), Ok(evaluation)) =
                        (&task.worksheet_assignment, &result)
                    {
                        let live = &self.native_tools.memory.models.spreadsheet;
                        if &json!(live) == source {
                            let version =
                                self.controller.session_snapshot().settings.numeric_version;
                            let value = ppl_vm::PortableValue::from(
                                &ppl_vm::cas::from_cas_in_model(evaluation.value.clone(), version),
                            );
                            match live.write_attribute(cell, 0, &value) {
                                Ok(sheet) => worksheet = Some(sheet),
                                Err(error) => {
                                    result = Err(ppl_cas::CasError::Protocol(error.to_string()))
                                }
                            }
                        } else {
                            result = Err(ppl_cas::CasError::Protocol(
                                "Spreadsheet changed during CAS assignment".into(),
                            ));
                        }
                    }
                    if let Ok(evaluation) = &result {
                        let cas = session.clone();
                        if let Err(error) =
                            self.controller
                                .commit_native_operation(&task.owner, |memory| {
                                    for (name, value) in &evaluation.home_updates {
                                        memory
                                            .set_variable(
                                                name,
                                                ppl_vm::PortableValue::from(
                                                    &ppl_vm::cas::from_cas_home(value.clone()),
                                                ),
                                            )
                                            .map_err(|e| e.to_string())?;
                                    }
                                    memory.cas = cas;
                                    Ok(())
                                })
                        {
                            result = Err(ppl_cas::CasError::Protocol(error));
                        }
                    }
                    if result.is_ok()
                        && let Some(sheet) = worksheet
                    {
                        self.apply_spreadsheet_write(sheet);
                    }
                    let catalog = task.catalog;
                    let deleted = task.deleted.clone();
                    drop(task);
                    self.native_tools.cas_busy = false;
                    self.state.vm_state = crate::ide::VmExecutionState::Idle;
                    self.native_vm_state = crate::ide::VmExecutionState::Idle;
                    match result {
                        Ok(evaluation) => {
                            let value = evaluation.value;
                            if let Some(name) = deleted {
                                if self.native_tools.memory.name == name {
                                    self.native_tools.memory.name.clear();
                                    self.native_tools.memory.content.clear();
                                    self.native_tools.memory.original_name.clear();
                                }
                            }
                            if !catalog {
                                self.state.native.add_history(
                                    expression.clone(),
                                    value.to_string(),
                                    false,
                                    serde_json::to_value(&value).ok(),
                                );
                                self.state.native.edit(|e| e.set_text(""));
                            }
                            self.native_tools.error = None;
                        }
                        Err(error) => {
                            if !catalog {
                                self.state.native.add_history(
                                    expression,
                                    error.to_string(),
                                    true,
                                    None,
                                );
                            }
                            self.native_tools.error = Some(error.to_string());
                        }
                    }
                    self.native_tools.dirty();
                    ctx.request_repaint();
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.state.vm_state = crate::ide::VmExecutionState::Running;
                    ctx.request_repaint_after(Duration::from_millis(16));
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.cas_task = None;
                    self.native_tools.cas_busy = false;
                    self.native_tools.error = Some("CAS worker stopped without a result".into());
                    self.state.vm_state = crate::ide::VmExecutionState::Idle;
                }
            }
        }
    }
    pub(super) fn native_tools_visible(&self) -> bool {
        !self.state.firmware_backend
            && !self.state.native.sleeping
            && self.native_tools.page.is_some()
            && (self.state.layout == crate::ide::layout::WorkspaceLayout::Calculator
                || (self.state.layout == crate::ide::layout::WorkspaceLayout::Split
                    && self.state.show_calculator
                    && !self.state.show_compare))
            && self
                .controller
                .shared
                .active_dialog
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_none()
    }
    pub(super) fn render_native_tools(&mut self, ctx: &egui::Context) {
        if !self.native_tools_visible() {
            return;
        }
        if self.state.firmware_backend
            || self.state.native.sleeping
            || self
                .controller
                .shared
                .active_dialog
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some()
        {
            return;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && self.native_tools.page.is_some() {
            if self.cas_task.is_some() {
                self.controller.stop();
            } else {
                self.native_tools.close();
                self.focus_native_calculator(ctx);
            }
        }
        if self.cas_task.is_some() && ctx.input(|i| i.key_pressed(egui::Key::F12)) {
            self.controller.stop();
        }
        let numeric_field = self.function_numeric_math_entry();
        self.native_tools.function_numeric_editor(
            numeric_field.map(|_| self.state.native.displayed_editor()),
            numeric_field,
        );
        let spreadsheet_form = self.spreadsheet_form();
        self.native_tools.spreadsheet_editor(
            if self.spreadsheet_math_entry() {
                Some(self.state.native.displayed_editor())
            } else {
                None
            },
            spreadsheet_form,
            self.spreadsheet_cas_mode(),
        );
        self.native_tools
            .graph_symbolic_editor(if self.graph_math_entry() {
                Some(self.state.native.displayed_editor())
            } else {
                None
            });
        let mut requests = self.native_tools.show(
            ctx,
            self.display.last_display_rect,
            &self.controller.session_snapshot(),
        );
        if let Some(calculation) = self.native_tools.pending_model_plot.take() {
            requests.push(Request::Calculate(calculation));
        }
        for request in requests {
            self.handle_native_tool_request(request, ctx);
        }
        self.sync_native_presentation();
    }
    fn store_graph_configuration(
        memory: &mut ppl_vm::SessionContext,
        graph: &crate::ide::native_apps::GraphApp,
    ) -> Result<(), String> {
        graph.window.validate()?;
        memory
            .set_variable(
                "Axes",
                ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(i64::from(graph.window.axes))),
            )
            .map_err(|e| e.to_string())?;
        for (name, value) in [
            ("Xmin", graph.window.xmin),
            ("Xmax", graph.window.xmax),
            ("Ymin", graph.window.ymin),
            ("Ymax", graph.window.ymax),
            ("Tmin", graph.window.tmin),
            ("Tmax", graph.window.tmax),
        ] {
            let value = ppl_math::Real::parse(&value.to_string()).ok_or("Invalid app setting")?;
            memory
                .set_variable(name, ppl_vm::PortableValue::Real(value))
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    fn store_function_numeric_configuration(
        memory: &mut ppl_vm::SessionContext,
        state: &ppl_vm::AppNumericSettings,
    ) -> Result<(), String> {
        state.validate()?;
        for (name, value) in [
            ("NumStart", state.start),
            ("NumStep", state.step),
            ("NumZoom", state.zoom),
            ("NumType", ppl_math::Real::from_i64(state.kind)),
        ] {
            memory
                .set_variable(name, ppl_vm::PortableValue::Real(value))
                .map_err(|e| e.to_string())?;
        }
        memory
            .set_variable(
                "NumIndep",
                ppl_vm::PortableValue::List(
                    state
                        .independent
                        .iter()
                        .copied()
                        .map(ppl_vm::PortableValue::Real)
                        .collect(),
                ),
            )
            .map_err(|e| e.to_string())
    }
    pub(crate) fn handle_native_tool_request(&mut self, request: Request, ctx: &egui::Context) {
        if let Some(problem) = &self.profile_problem {
            if matches!(
                &request,
                Request::DefineFunction { .. }
                    | Request::EditCollection { .. }
                    | Request::RenameObject { .. }
                    | Request::SaveProgram(..)
                    | Request::SaveNote(..)
                    | Request::SaveBulletedNote(..)
                    | Request::StoreVariable(..)
                    | Request::DeleteProgram(..)
                    | Request::DeleteNote(..)
                    | Request::DeleteVariable(..)
                    | Request::StoreCasVariable(..)
                    | Request::DeleteCasVariable(..)
                    | Request::ResetApp(_)
            ) {
                self.native_tools.error = Some(problem.clone());
                return;
            }
        }

        let catalog_change = catalog_integration::CatalogChange::from_request(&request);
        let catalog_worker = matches!(
            &request,
            Request::EditCollection { .. }
                | Request::StoreVariable(..)
                | Request::RunProgram(..)
                | Request::RunProgramConfigured { .. }
        );
        let cas_display = if let Request::CasDisplay { display, .. } = &request {
            Some(display.clone())
        } else {
            None
        };
        let catalog_cas = matches!(&request, Request::CasCatalog(..));
        let catalog_deleted = if let Request::CasCatalog(_, name) = &request {
            name.clone()
        } else {
            None
        };
        let start_app = matches!(&request, Request::StartApp(_));
        let result = match request {
            Request::CopyText(text) => {
                ctx.copy_text(text);
                Ok(())
            }
            Request::MathCursor { position, extend } => {
                if self.math_entry.is_some() {
                    self.state.native.edit(|e| e.set_cursor(position, extend));
                }
                Ok(())
            }
            Request::FinishMath(commit) => {
                self.finish_math_entry(commit, ctx);
                return;
            }
            Request::ToggleSpreadsheetCas => {
                self.toggle_spreadsheet_cas();
                Ok(())
            }
            Request::SpreadsheetHistory(origin) => {
                let active = (self.state.native.cas_mode == origin.is_cas())
                    .then_some(&self.state.native.history);
                self.native_tools.spreadsheet_open_history(origin, active);
                Ok(())
            }
            Request::TypeText(text) => {
                if self.math_entry.is_some() {
                    self.state.native.edit(|editor| editor.type_text(&text));
                    self.focus_native_calculator(ctx);
                    Ok(())
                } else {
                    Err("Calculator is busy".into())
                }
            }
            Request::StatisticsCellValue {
                two,
                column,
                row,
                value,
            } => self.write_statistics_cell(two, column, row, value, None),
            Request::StatisticsWrite { two, columns } => {
                self.write_statistics_columns(two, columns, None)
            }
            Request::StatisticsDelete { two, column, row } => {
                self.delete_statistics_cell(two, column, row)
            }
            Request::SaveInteger => {
                if let Some(word) = &self.native_tools.integer_edit {
                    self.native_tools
                        .memory
                        .integer_edits
                        .insert(self.native_tools.integer_key.clone(), word.clone());
                }
                self.native_tools.close();
                Ok(())
            }
            Request::CancelInteger => {
                self.native_tools.integer_edit = self.native_tools.integer_original.clone();
                self.native_tools.close();
                Ok(())
            }
            Request::SelectApp(name) | Request::StartApp(name) => {
                if !self.native_tools.memory.graphs.contains_key(&name)
                    && !crate::ide::native_app_ui::MODEL_APPS.contains(&name.as_str())
                {
                    self.native_tools.error = Some("Unknown Native app".into());
                    return;
                }
                let graph = self.native_tools.memory.graphs.get(&name).cloned();
                let model_window = if crate::ide::native_app_views::supports(
                    &name,
                    crate::ide::native_app_views::AppView::Plot,
                ) {
                    Some(
                        self.native_tools
                            .memory
                            .model_windows
                            .get(&name)
                            .cloned()
                            .unwrap_or_else(|| NativeTools::default_model_window(&name)),
                    )
                } else {
                    None
                };
                // Scoped VM settings are authoritative. Older UI-only profiles
                // retain their saved window/histogram when first promoted to a
                // scoped record; fresh apps start at their measured defaults.
                let scoped_plot = if graph.is_none() {
                    self.controller
                        .session_snapshot()
                        .app_plot_settings
                        .get(&name)
                        .cloned()
                        .or_else(|| {
                            let mut settings = ppl_vm::AppPlotSettings::for_app(&name)?;
                            if let Some(window) = &model_window {
                                for (field, value) in [
                                    ("Xmin", window.xmin),
                                    ("Xmax", window.xmax),
                                    ("Ymin", window.ymin),
                                    ("Ymax", window.ymax),
                                ] {
                                    settings
                                        .set(
                                            field,
                                            &ppl_vm::Value::Real(
                                                ppl_math::Real::from_f64(value).ok()?,
                                            ),
                                        )
                                        .ok()?;
                                }
                                settings.axes = window.axes;
                            }
                            if name == "Statistics 1Var"
                                && let Some(histogram) = self.native_tools.memory.models.histogram
                            {
                                for (field, value) in [
                                    ("Hmin", histogram.minimum),
                                    ("Hmax", histogram.maximum),
                                    ("Hwidth", histogram.width),
                                ] {
                                    settings
                                        .set(
                                            field,
                                            &ppl_vm::Value::Real(
                                                ppl_math::Real::from_f64(value).ok()?,
                                            ),
                                        )
                                        .ok()?;
                                }
                            }
                            Some(settings)
                        })
                } else {
                    None
                };
                if name == "Geometry"
                    && let Err(error) = self.promote_legacy_geometry_window()
                {
                    // The legacy camera is still saved separately. Open the
                    // app with a valid viewport so it can still be edited.
                    self.state.status_message = error;
                }
                let selected = name.clone();

                let result = self.controller.mutate_memory(|memory| {
                    memory.active_app = name;
                    if selected == "Geometry" {
                        memory
                            .set_geometry_plot_settings(memory.geometry_plot_settings())
                            .map_err(|error| error.to_string())?;
                    } else if let Some(graph) = graph {
                        Self::store_graph_configuration(memory, &graph)?;
                    } else if let Some(settings) = scoped_plot {
                        memory
                            .set_app_plot_settings(&selected, settings)
                            .map_err(|e| e.to_string())?;
                    } else if let Some(window) = model_window {
                        for (name, value) in [
                            ("Xmin", window.xmin),
                            ("Xmax", window.xmax),
                            ("Ymin", window.ymin),
                            ("Ymax", window.ymax),
                        ] {
                            memory
                                .set_variable(
                                    name,
                                    ppl_vm::PortableValue::Real(
                                        ppl_math::Real::from_f64(value)
                                            .map_err(|e| e.to_string())?,
                                    ),
                                )
                                .map_err(|e| e.to_string())?;
                        }
                    }
                    Ok(())
                });
                if result.is_ok() {
                    self.native_tools.memory.selected_app = selected.clone();
                    if selected == "Geometry" {
                        let settings = self.controller.session_snapshot().geometry_plot_settings();
                        self.native_tools.geometry_settings_changed(&settings);
                    }
                    self.native_tools.gallery_started(&selected);
                    if start_app {
                        let page = crate::ide::native_tools::NativeTools::entry_page(
                            &self.native_tools.memory.selected_app,
                        );
                        self.native_tools.open(page);
                        let snapshot = self.controller.session_snapshot();
                        if page == Page::Plot {
                            self.native_tools.plot(snapshot);
                        } else if page == Page::Numeric {
                            self.native_tools.table(&snapshot);
                        }
                    }
                } else {
                    self.native_tools.memory.selected_app =
                        self.controller.session_snapshot().active_app;
                    let page = crate::ide::native_tools::NativeTools::entry_page(
                        &self.native_tools.memory.selected_app,
                    );
                    if !start_app {
                        self.native_tools.open(page);
                    }
                }
                result
            }
            Request::Graph3dPlotZoom { source, edited } => {
                let name = crate::ide::native_graph3d::NAME;
                let result = self.controller.mutate_memory(|memory| {
                    if memory.active_app != name {
                        return Err("Graph 3D is no longer active".into());
                    }
                    if memory
                        .app_plot_settings()
                        .map_err(|error| error.to_string())?
                        != *source
                    {
                        return Err("Graph 3D Plot settings changed while choosing Zoom".into());
                    }
                    memory
                        .set_app_plot_settings(name, *edited)
                        .map_err(|error| error.to_string())
                });
                if result.is_ok() {
                    self.native_tools
                        .graph3d_refresh_zoom(self.controller.session_snapshot());
                }
                result
            }
            Request::Graph3dNumericConfiguration {
                source,
                edited,
                selected_cell,
            } => {
                let result = self.controller.mutate_memory(|memory| {
                    if memory.active_app != crate::ide::native_graph3d::NAME {
                        return Err("Graph 3D is no longer active".into());
                    }
                    if memory.app_numeric_settings() != *source {
                        return Err("Graph 3D Numeric settings changed while editing".into());
                    }
                    memory
                        .set_graph3d_numeric_settings(*edited)
                        .map_err(|e| e.to_string())
                });
                if result.is_ok() {
                    self.native_tools.open(Page::Numeric);
                    self.native_tools.graph3d_select_numeric_cell(selected_cell);
                    self.native_tools.table(&self.controller.session_snapshot());
                }
                result
            }
            Request::FunctionNumericConfiguration(state) => {
                let result = self.controller.mutate_memory(|memory| {
                    if memory.active_app != "Function" {
                        return Err("Function is no longer active".into());
                    }
                    Self::store_function_numeric_configuration(memory, &state)
                });
                if result.is_ok() && self.native_tools.page == Some(Page::Numeric) {
                    self.native_tools.table(&self.controller.session_snapshot());
                } else if result.is_err() {
                    self.native_tools
                        .function_numeric_apply_context(&self.controller.session_snapshot());
                }
                result
            }
            Request::GraphConfiguration(_, _) if self.native_tools.presentation_pending => Ok(()),
            Request::PlotConfiguration(_, _) if self.native_tools.presentation_pending => Ok(()),
            Request::ResetApp(target) => {
                use crate::ide::native_tools::ResetApp;
                self.controller.mutate_memory_and_ui(|memory| {
                    match target {
                        ResetApp::Sequence => {
                            let name = target.name();
                            let settings = ppl_vm::AppPlotSettings::for_app(name)
                                .expect("Sequence has established Plot Setup defaults");
                            memory
                                .set_app_plot_settings(name, settings.clone())
                                .map_err(|error| error.to_string())?;
                            memory.app_numeric_settings.remove(name);
                            memory.app_settings.remove(name);
                            self.native_tools.reset_sequence(&settings);
                        }
                        ResetApp::Geometry => {
                            let settings = ppl_vm::GeometryPlotSettings::default();
                            memory
                                .set_geometry_plot_settings(settings.clone())
                                .map_err(|error| error.to_string())?;
                            self.native_tools.reset_geometry(&settings);
                        }
                        ResetApp::StatisticsOne | ResetApp::StatisticsTwo => {
                            use crate::ide::native_statistics::{COLUMN_COUNT, Columns};
                            let app = target.name();
                            let two = matches!(target, ResetApp::StatisticsTwo);
                            let settings = ppl_vm::AppPlotSettings::for_app(app)
                                .expect("Statistics has established Plot Setup defaults");
                            memory
                                .set_app_plot_settings(app, settings)
                                .map_err(|error| error.to_string())?;
                            memory.app_settings.remove(app);
                            for index in 0..COLUMN_COUNT {
                                let name = Columns::label(index, two);
                                memory
                                    .set_variable(&name, ppl_vm::PortableValue::List(Vec::new()))
                                    .map_err(|error| error.to_string())?;
                            }
                            self.native_tools.reset_statistics(two, &memory.snapshot());
                        }
                    }
                    let ui = native_profile_ui(&self.state.native, &self.native_tools.memory);
                    self.profile_ui_last = ui.clone();
                    Ok(ui)
                })
            }
            Request::GeometryPlotConfiguration(settings) => {
                let result = self.controller.mutate_memory(|memory| {
                    memory
                        .set_geometry_plot_settings(settings)
                        .map_err(|error| error.to_string())
                });
                if result.is_err() {
                    let committed = self.controller.session_snapshot().geometry_plot_settings();
                    self.native_tools.geometry_settings_changed(&committed);
                }
                result
            }
            Request::PlotConfiguration(name, settings) => self.controller.mutate_memory(|memory| {
                settings.validate()?;
                if ppl_vm::AppPlotSettings::for_app(&name).is_none() {
                    return Err("This app has no supported Plot Setup variables".into());
                }
                memory
                    .set_app_plot_settings(&name, settings)
                    .map_err(|e| e.to_string())
            }),
            Request::GraphConfiguration(name, graph) => self.controller.mutate_memory(|memory| {
                if memory.active_app == name {
                    Self::store_graph_configuration(memory, &graph)?;
                    if self.native_tools.page == Some(Page::NumericSetup) {
                        memory
                            .set_variable(
                                "NumStart",
                                ppl_vm::PortableValue::Real(
                                    ppl_math::Real::from_f64(graph.numeric_start)
                                        .map_err(|e| e.to_string())?,
                                ),
                            )
                            .map_err(|e| e.to_string())?;
                        memory
                            .set_variable(
                                "NumStep",
                                ppl_vm::PortableValue::Real(
                                    ppl_math::Real::from_f64(graph.numeric_step)
                                        .map_err(|e| e.to_string())?,
                                ),
                            )
                            .map_err(|e| e.to_string())?;
                    }
                }
                Ok(())
            }),
            Request::RenameObject { old, new, program } => {
                self.controller.mutate_memory(|memory| {
                    if program {
                        memory.rename_program(&old, &new)
                    } else {
                        memory.rename_note(&old, &new)
                    }
                    .map_err(|e| e.to_string())
                })
            }
            Request::EditCollection { name, matrix } => {
                let value = self
                    .controller
                    .session_snapshot()
                    .globals
                    .get(&name)
                    .cloned();
                let expected = if matrix {
                    matches!(value, Some(ppl_vm::PortableValue::Matrix { .. }))
                } else {
                    matches!(value, Some(ppl_vm::PortableValue::List(_)))
                };
                if value.is_some() && !expected {
                    Err("Object has a different type".into())
                } else {
                    // Create installs the named blank collection immediately;
                    // Cancel subsequently discards only the editor's changes.
                    let created = if value.is_none() {
                        self.controller.mutate_memory(|memory| {
                            memory
                                .set_variable(
                                    &name,
                                    if matrix {
                                        ppl_vm::PortableValue::Matrix {
                                            rows: 1,
                                            cols: 1,
                                            vector: false,
                                            data: vec![ppl_vm::PortableValue::Real(
                                                ppl_math::Real::ZERO,
                                            )],
                                        }
                                    } else {
                                        ppl_vm::PortableValue::List(Vec::new())
                                    },
                                )
                                .map_err(|e| e.to_string())
                        })
                    } else {
                        Ok(())
                    };
                    created.and_then(|()| {
                        self.start_native_catalog_variable(
                            name.clone(),
                            format!("{}({name})", if matrix { "EDITMAT" } else { "EDITLIST" }),
                        )
                    })
                }
            }
            Request::DefineFunction {
                name,
                parameters,
                body,
            } => {
                let source = format!("EXPORT {name}({parameters}) BEGIN RETURN {body}; END;");
                let (ast, diagnostics) = ppl_syntax::parse(&source);
                let valid_name = |name: &str| {
                    !name.is_empty()
                        && name.chars().enumerate().all(|(i, c)| {
                            c == '_' || c.is_alphabetic() || (i > 0 && c.is_ascii_digit())
                        })
                };
                if !valid_name(&name)
                    || parameters
                        .split(',')
                        .filter(|p| !p.trim().is_empty())
                        .any(|p| !valid_name(p.trim()))
                {
                    Err("Enter a valid function name and parameter names".into())
                } else if diagnostics.has_errors() {
                    Err(diagnostics.errors().next().unwrap().message.clone())
                } else if ast.items.len() != 1 {
                    Err("Define accepts one expression function".into())
                } else if ppl_compiler::is_system_variable(&name)
                    || ppl_compiler::builtins::lookup_case_insensitive(&name).is_some()
                {
                    Err("Choose a user function name".into())
                } else {
                    let (expression, diagnostics) = ppl_syntax::parse_expression(&body);
                    if diagnostics.has_errors() {
                        Err(diagnostics.errors().next().unwrap().message.clone())
                    } else {
                        let value =
                            ppl_vm::Value::Symbolic(std::rc::Rc::new(ppl_cas::CasValue::Symbolic(
                                ppl_compiler::cas_entry_source(&expression),
                            )));
                        let parameters = parameters
                            .split(',')
                            .filter(|name| !name.trim().is_empty())
                            .map(|name| ppl_vm::Value::str(name.trim()))
                            .collect::<Vec<_>>();
                        match ppl_vm::cas::with_parameters(&value, &parameters) {
                            Ok(function) => {
                                let portable = ppl_vm::PortableValue::from(&function);
                                self.controller.mutate_memory(|memory| {
                                    memory
                                        .set_variable(&name, portable)
                                        .map_err(|e| e.to_string())
                                })
                            }
                            Err(error) => Err(error.to_string()),
                        }
                    }
                }
            }
            Request::EditMath(target) => {
                self.open_math_entry(target, ctx);
                Ok(())
            }
            Request::InsertTemplate(template) => {
                self.native_tools.close();
                self.state.native.edit(|e| e.insert_template(template));
                self.set_native_view(NativeView::Home, ctx);
                if self.spreadsheet_math_entry() {
                    self.native_tools.open(Page::Numeric);
                } else if self.graph_math_entry() {
                    self.native_tools.open(Page::Symbolic);
                }
                Ok(())
            }
            Request::SetAppSettings(name, settings) => self.controller.mutate_memory(|memory| {
                settings.validate()?;
                memory.app_settings.insert(name, settings);
                Ok(())
            }),
            Request::SetCasSettings(settings) => self.controller.mutate_memory(|memory| {
                memory.cas.settings = settings;
                Ok(())
            }),
            Request::Calculate(calculation) => {
                if self.controller.running() || self.cas_task.is_some() || self.model_task.is_some()
                {
                    Err("Calculator is busy".into())
                } else {
                    let owner = match self.controller.reserve_native_operation() {
                        Ok(owner) => owner,
                        Err(error) => {
                            self.native_tools.error = Some(error);
                            return;
                        }
                    };
                    let source = self.native_tools.memory.models.input_fingerprint();
                    let cancel = self.controller.shared.interrupt.clone();
                    cancel.store(false, std::sync::atomic::Ordering::Release);
                    let (tx, rx) = mpsc::channel();
                    let operation = calculation.clone();
                    let snapshot = self.controller.session_snapshot();
                    let statistics2_columns_revision = snapshot.statistics2_columns_revision;
                    let repaint = ctx.clone();
                    let thread = std::thread::spawn(move || {
                        let result = operation.calculate(cancel, &snapshot);
                        let _ = tx.send(result);
                        repaint.request_repaint();
                    });
                    self.native_tools.statistics_busy(true);
                    self.native_tools.geometry_busy(true);
                    self.model_task = Some(ModelTask {
                        thread: Some(thread),
                        _owner: owner,
                        rx,
                        calculation,
                        source,
                        statistics2_columns_revision,
                    });
                    self.state.vm_state = crate::ide::VmExecutionState::Running;
                    Ok(())
                }
            }
            Request::Dismiss => {
                self.native_tools.close();
                self.set_native_view(NativeView::Home, ctx);
                Ok(())
            }
            Request::Close => {
                self.native_tools.close();
                self.switch_native_environment(false, ctx);
                Ok(())
            }
            Request::Insert(text) => {
                self.native_tools.close();
                self.state.native.edit(|e| e.insert_math_token(&text));
                self.set_native_view(NativeView::Home, ctx);
                Ok(())
            }
            Request::StoreVariable(name, text) => self.start_native_catalog_variable(name, text),
            Request::DeleteVariable(name) => self.controller.mutate_memory(|memory| {
                if memory.delete_variable(&name) {
                    Ok(())
                } else {
                    Err("Variable does not exist".into())
                }
            }),
            Request::SaveProgram(name, source) => self.controller.mutate_memory(|memory| {
                memory
                    .store_program(&name, source)
                    .map_err(|e| e.to_string())
            }),
            Request::DeleteProgram(name) => self.controller.mutate_memory(|memory| {
                if memory.delete_program(&name) {
                    Ok(())
                } else {
                    Err("Program does not exist".into())
                }
            }),
            Request::SaveNote(name, text) => self
                .controller
                .mutate_memory(|memory| memory.store_note(&name, text).map_err(|e| e.to_string())),
            Request::SaveBulletedNote(name, text, bullets) => {
                self.controller.mutate_memory(|memory| {
                    memory
                        .store_bulleted_note(&name, text, bullets)
                        .map_err(|e| e.to_string())
                })
            }
            Request::DeleteNote(name) => self.controller.mutate_memory(|memory| {
                if memory.delete_note(&name) {
                    Ok(())
                } else {
                    Err("Note does not exist".into())
                }
            }),
            Request::RunProgram(name, source) => self.run_native_catalog_program(name, source),
            Request::RunProgramConfigured {
                name,
                source,
                entry,
                args,
            } => self.run_native_catalog_program_configured(name, source, entry, args),
            Request::SetHomeSettings(settings) => self.controller.mutate_memory(|memory| {
                if !(1..=64).contains(&settings.int_bits)
                    || (settings.int_bits == 64 && settings.int_signed)
                    || !(0..=5).contains(&settings.hformat)
                    || !(0..=11).contains(&settings.hdigits)
                {
                    return Err("Invalid Home setting".into());
                }
                memory.settings = settings;
                Ok(())
            }),
            Request::SetAngle(angle) => self.controller.mutate_memory(|memory| {
                memory.settings.angle = angle;
                Ok(())
            }),
            Request::SetFormat(format, digits) => self.controller.mutate_memory(|memory| {
                memory.settings.set_format(format, digits);
                Ok(())
            }),
            Request::StoreCasVariable(name, value) => {
                if !ppl_cas::valid_variable_name(&name) || name == "Ans" {
                    Err("Enter a valid CAS variable name".into())
                } else {
                    self.handle_native_tool_request(
                        Request::CasCatalog(format!("{name}:=({value})"), None),
                        ctx,
                    );
                    return;
                }
            }
            Request::DeleteCasVariable(name) => {
                if !self
                    .controller
                    .session_snapshot()
                    .cas
                    .bindings
                    .contains_key(&name)
                    || !ppl_cas::valid_variable_name(&name)
                {
                    Err("CAS variable does not exist".into())
                } else {
                    self.handle_native_tool_request(
                        Request::CasCatalog(format!("purge({name})"), Some(name.clone())),
                        ctx,
                    );
                    return;
                }
            }
            Request::Cas(expression)
            | Request::CasCatalog(expression, _)
            | Request::CasDisplay {
                source: expression, ..
            } => {
                if self.controller.running() || self.cas_task.is_some() {
                    Err("Calculator is busy".into())
                } else if expression.trim().is_empty() {
                    Ok(())
                } else {
                    let owner = match self.controller.reserve_native_operation() {
                        Ok(owner) => owner,
                        Err(error) => {
                            self.native_tools.error = Some(error);
                            return;
                        }
                    };
                    let (parsed, diagnostics) = ppl_syntax::parse_expression(&expression);
                    let worksheet_store = if !catalog_cas && !diagnostics.has_errors() {
                        app_variable_integration::cas_worksheet_assignment(&parsed)
                    } else {
                        None
                    };
                    let operand = worksheet_store.map_or(&parsed, |(_, rhs)| rhs);
                    let qualified_cas =
                        match app_variable_integration::qualified_cas_operand(operand) {
                            Ok(found) => found,
                            Err(error) => {
                                self.reject_qualified_cas(
                                    expression.clone(),
                                    error,
                                    catalog_cas,
                                    ctx,
                                );
                                return;
                            }
                        };
                    if diagnostics.has_errors()
                        && (expression.contains("Spreadsheet.") || expression.contains("Function."))
                    {
                        self.reject_qualified_cas(
                            expression.clone(),
                            "Application access requires a valid CAS expression".into(),
                            catalog_cas,
                            ctx,
                        );
                        return;
                    }
                    if qualified_cas && app_variable_integration::cas_assignment(operand) {
                        self.reject_qualified_cas(
                            expression.clone(),
                            "CAS application assignments require a direct Spreadsheet cell target"
                                .into(),
                            catalog_cas,
                            ctx,
                        );
                        return;
                    }
                    let cas_source = if diagnostics.has_errors() {
                        expression.clone()
                    } else {
                        ppl_compiler::cas_entry_source(operand)
                    };
                    let cas_source = match ppl_cas::units::translate_home(&cas_source) {
                        Ok(Some(source)) => source,
                        Ok(None) => cas_source,
                        Err(error) => {
                            self.native_tools.error = Some(error.to_string());
                            return;
                        }
                    };
                    let snapshot = self.controller.session_snapshot();
                    let qualified_context = snapshot.clone();
                    let worksheet_assignment = worksheet_store.map(|(cell, _)| {
                        (
                            cell.to_owned(),
                            json!(self.native_tools.memory.models.spreadsheet),
                        )
                    });
                    let decimal_worksheet_read = !catalog_cas
                        && worksheet_store.is_none()
                        && !diagnostics.has_errors()
                        && app_variable_integration::cas_worksheet_value_read(operand);
                    let qualified_shared = self.controller.shared.clone();
                    let home = snapshot
                        .globals
                        .iter()
                        .filter_map(|(name, value)| {
                            ppl_vm::cas::to_cas(&value.to_value())
                                .map(|value| (name.clone(), value))
                        })
                        .collect();
                    let mut session = snapshot.cas;
                    let cancel = self.controller.shared.interrupt.clone();
                    cancel.store(false, std::sync::atomic::Ordering::Release);
                    let (tx, rx) = mpsc::channel();
                    let repaint = ctx.clone();
                    let thread = std::thread::spawn(move || {
                        let previous_answer = session.last_value.clone();
                        let result = if qualified_cas {
                            match app_variable_integration::evaluate_qualified_cas(
                                &cas_source,
                                &qualified_context,
                                qualified_shared,
                                decimal_worksheet_read,
                            ) {
                                Ok((next, result)) => {
                                    session = next;
                                    Ok(result)
                                }
                                Err(error) => Err(error),
                            }
                        } else {
                            session.evaluate_with_home(
                                &cas_source,
                                &home,
                                &Default::default(),
                                &cancel,
                            )
                        };
                        if catalog_cas {
                            session.last_value = previous_answer;
                        }
                        let _ = tx.send((session, cas_display.unwrap_or(expression), result));
                        repaint.request_repaint();
                    });
                    self.cas_task = Some(CasTask {
                        catalog: catalog_cas,
                        deleted: catalog_deleted,
                        worksheet_assignment,
                        thread: Some(thread),
                        owner,
                        rx,
                    });
                    self.native_tools.cas_busy = true;
                    self.state.vm_state = crate::ide::VmExecutionState::Running;
                    Ok(())
                }
            }
        };
        match result {
            Ok(()) => {
                self.apply_native_catalog_change(catalog_change);
                if catalog_worker {
                    self.focus_native_calculator(ctx);
                }
                self.native_tools.error = None;
                self.native_tools.dirty();
            }
            Err(error) => self.native_tools.error = Some(error),
        }
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod app_info_tests {
    use super::*;
    #[test]
    fn shift_apps_opens_the_current_app_info_note_and_help_stays_separate() {
        let mut app = IdeDesktopApp::default();
        let ctx = egui::Context::default();
        app.state.native.set_view(NativeView::Home);
        app.state.calc_input_focus = true;
        app.native_tools
            .memory
            .app_notes
            .insert("Function".into(), "graph notes".into());
        app.controller
            .mutate_memory(|memory| {
                memory
                    .store_note("NOTE", "independent".into())
                    .map_err(|e| e.to_string())
            })
            .unwrap();
        for code in [crate::keymap::KEY_SHIFT, crate::keymap::KEY_APPS] {
            app.route_native_press(KeyPress::code(code), "test", &ctx);
        }
        app.consume_native_tool(&ctx);
        assert_eq!(app.native_tools.page, Some(Page::Info));
        assert_eq!(app.native_tools.memory.app_notes["Function"], "graph notes");
        assert_eq!(app.controller.session_snapshot().notes.len(), 1);
        app.native_tools.close();
        app.route_native_press(KeyPress::code(crate::keymap::KEY_HELP), "test", &ctx);
        app.consume_native_tool(&ctx);
        assert_eq!(app.native_tools.page, Some(Page::Help));
    }
    #[test]
    fn desktop_integer_arrows_match_virtual_word_and_draft_updates() {
        let mut app = IdeDesktopApp::default();
        let ctx = egui::Context::default();
        app.state.native.set_view(NativeView::Home);
        app.state.calc_input_focus = true;
        app.native_tools.open(Page::Base);
        app.native_tools.integer_edit = Some(crate::ide::native_integer::IntegerEdit {
            raw: 3,
            bits: 4,
            base: 16,
            signed: false,
            negative: false,
            system_base: false,
        });
        let _ = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::ArrowLeft,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ctx| app.route_native_keyboard(ctx),
        );
        assert_eq!(app.native_tools.integer_edit.as_ref().unwrap().raw, 6);
        assert_eq!(app.native_tools.integer_decimal_draft, "6");
        assert_eq!(app.native_tools.integer_hex_draft, "6");
        app.route_native_press(KeyPress::code(crate::keymap::KEY_RIGHT), "virtual", &ctx);
        assert_eq!(app.native_tools.integer_edit.as_ref().unwrap().raw, 3);
        assert_eq!(app.native_tools.integer_decimal_draft, "3");
        assert_eq!(app.native_tools.integer_hex_draft, "3");
    }
    #[test]
    fn startapp_opens_its_entry_view_and_startview_six_is_app_info() {
        let mut app = IdeDesktopApp::default();
        let ctx = egui::Context::default();
        let epoch = app
            .controller
            .shared
            .operation_epoch
            .load(std::sync::atomic::Ordering::Acquire);
        for (request, expected) in [
            (
                ppl_vm::host::CalculatorView::StartApp("Finance".into()),
                Page::Numeric,
            ),
            (
                ppl_vm::host::CalculatorView::StartView {
                    number: 6,
                    draw: true,
                },
                Page::Info,
            ),
        ] {
            let (tx, rx) = std::sync::mpsc::channel();
            app.controller
                .shared
                .calculator_views
                .lock()
                .unwrap()
                .push_back(crate::host::CalculatorViewRequest {
                    request,
                    context: None,
                    await_plot: false,
                    reply: tx,
                    epoch,
                });
            app.poll_native_tools(&ctx);
            assert_eq!(
                rx.try_recv().unwrap().map(|outcome| outcome.changed),
                Ok(true)
            );
            assert_eq!(app.native_tools.memory.selected_app, "Finance");
            assert_eq!(app.native_tools.page, Some(expected));
        }
    }
}
