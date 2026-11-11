//! Native tools integration and durable profile lifecycle.
use super::*;
use crate::ide::native_tools::{Page, Request};
use serde_json::json;
use std::sync::mpsc;

pub(super) struct ModelTask {
    thread: Option<std::thread::JoinHandle<()>>,
    _owner: crate::ide::execution::worker::SessionLease,
    rx: mpsc::Receiver<Result<serde_json::Value, String>>,
    calculation: crate::ide::native_app_ui::Calculation,
    source: serde_json::Value,
}
pub(super) struct CasTask {
    catalog: bool,
    deleted: Option<String>,
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
        if self.native_tools.page == Some(Page::Plot) {
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
                        word.resize(if delta < 0 {
                            word.bits.saturating_add(1)
                        } else {
                            word.bits.saturating_sub(1)
                        });
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
            KeyAction::History(delta) if self.native_tools.page == Some(Page::Apps) => {
                let names: Vec<_> = self
                    .native_tools
                    .memory
                    .graphs
                    .keys()
                    .cloned()
                    .chain(
                        crate::ide::native_app_ui::MODEL_APPS
                            .iter()
                            .map(|s| s.to_string()),
                    )
                    .collect();
                if let Some(index) = names
                    .iter()
                    .position(|name| name == &self.native_tools.memory.selected_app)
                {
                    let next = if delta < 0 {
                        index.saturating_sub(1)
                    } else {
                        (index + 1).min(names.len().saturating_sub(1))
                    };
                    self.native_tools.memory.selected_app = names[next].clone();
                    self.native_tools.dirty();
                }
                self.handle_native_tool_request(
                    Request::SelectApp(self.native_tools.memory.selected_app.clone()),
                    ctx,
                );
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
            KeyAction::Evaluate if self.native_tools.page == Some(Page::Apps) => {
                let page = crate::ide::native_tools::NativeTools::entry_page(
                    &self.native_tools.memory.selected_app,
                );
                self.native_tools.open(page);
                if page == Page::Plot {
                    self.native_tools.plot(self.controller.session_snapshot());
                } else if page == Page::Numeric {
                    self.native_tools.table(&self.controller.session_snapshot());
                }
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
            KeyAction::Home | KeyAction::Back => {
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
            Tool::Help => Page::Help,
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
                let value = &snapshot.globals["Ans"];
                match crate::ide::native_integer::IntegerEdit::from_value(value, &snapshot.settings)
                {
                    Ok(word) => {
                        let key = format!("{:?}", value);
                        let word = self
                            .native_tools
                            .memory
                            .integer_edits
                            .get(&key)
                            .cloned()
                            .unwrap_or(word);
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
                                    expression: expression.into(),
                                    result: result.into(),
                                    error,
                                    typed_result: entry
                                        .get("typed_result")
                                        .filter(|v| !v.is_null())
                                        .cloned(),
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
        let ui = json!({"history":self.state.native.history,"home_input":self.state.native.home_input,"tools":self.native_tools.memory,"cas_mode":self.state.native.cas_mode});
        if ui != self.profile_ui_last {
            profile.update("ui", ui.clone());
            self.profile_ui_last = ui;
        }
        if let Some(error) = profile.error() {
            self.state.status_message = format!("Native memory not saved: {error}");
        }
    }
    pub(super) fn poll_native_tools(&mut self, ctx: &egui::Context) {
        let requests = std::mem::take(
            &mut *self
                .controller
                .shared
                .calculator_views
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
        );
        for message in requests {
            if message.epoch
                != self
                    .controller
                    .shared
                    .input_epoch
                    .load(std::sync::atomic::Ordering::Acquire)
            {
                let _ = message
                    .reply
                    .send(Err("Stale calculator view request".into()));
                continue;
            }
            let result = match message.request {
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
                            self.native_tools.plot(self.controller.session_snapshot());
                        }
                        Ok(true)
                    } else {
                        Ok(false)
                    }
                }
                ppl_vm::host::CalculatorView::StartView { number, draw } => {
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
                            let _ = message.reply.send(Ok(false));
                            continue;
                        }
                    };
                    if let Some(page) = page {
                        if !self.native_tools.supports_page(page) {
                            let _ = message.reply.send(Ok(false));
                            continue;
                        }
                        self.native_tools.open(page);
                        if draw && page == Page::Plot {
                            self.native_tools.plot(self.controller.session_snapshot());
                        } else if page == Page::Numeric {
                            self.native_tools.table(&self.controller.session_snapshot());
                        }
                    } else {
                        self.native_tools.close();
                    }
                    Ok(true)
                }
            };
            let _ = message.reply.send(result);
            ctx.request_repaint();
        }
        if let Some(task) = &self.model_task {
            match task.rx.try_recv() {
                Ok(result) => {
                    let task = self.model_task.take().unwrap();
                    if self.native_tools.memory.models.input_fingerprint() == task.source {
                        match result {
                            Ok(value) => {
                                self.native_tools.error = None;
                                let app_name = task.calculation.result_key();
                                if let crate::ide::native_app_ui::Calculation::Plot(name, _) =
                                    &task.calculation
                                {
                                    if let Err(error) =
                                        self.native_tools.model_plot_completed(name, &value)
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
                                self.native_tools
                                    .memory
                                    .models
                                    .results
                                    .insert(app_name, value);
                            }
                            Err(error) => self.native_tools.error = Some(error),
                        }
                    }
                    self.state.vm_state = crate::ide::VmExecutionState::Idle;
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
    }
    fn store_graph_configuration(
        memory: &mut ppl_vm::SessionContext,
        graph: &crate::ide::native_apps::GraphApp,
    ) -> Result<(), String> {
        graph.window.validate()?;
        for (name, value) in [
            ("Xmin", graph.window.xmin),
            ("Xmax", graph.window.xmax),
            ("Ymin", graph.window.ymin),
            ("Ymax", graph.window.ymax),
            ("Tmin", graph.window.tmin),
            ("Tmax", graph.window.tmax),
            ("NumStart", graph.numeric_start),
            ("NumStep", graph.numeric_step),
        ] {
            let value = ppl_math::Real::parse(&value.to_string()).ok_or("Invalid app setting")?;
            memory
                .set_variable(name, ppl_vm::PortableValue::Real(value))
                .map_err(|e| e.to_string())?;
        }
        Ok(())
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
                    | Request::StoreVariable(..)
                    | Request::DeleteProgram(..)
                    | Request::DeleteNote(..)
                    | Request::DeleteVariable(..)
                    | Request::StoreCasVariable(..)
                    | Request::DeleteCasVariable(..)
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
        let catalog_cas = matches!(&request, Request::CasCatalog(..));
        let catalog_deleted = if let Request::CasCatalog(_, name) = &request {
            name.clone()
        } else {
            None
        };
        let result = match request {
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
            Request::SelectApp(name) => {
                if !self.native_tools.memory.graphs.contains_key(&name)
                    && !crate::ide::native_app_ui::MODEL_APPS.contains(&name.as_str())
                {
                    self.native_tools.error = Some("Unknown Native app".into());
                    return;
                }
                let graph = self.native_tools.memory.graphs.get(&name).cloned();
                let selected = name.clone();
                let result = self.controller.mutate_memory(|memory| {
                    memory.active_app = name;
                    if let Some(graph) = graph {
                        Self::store_graph_configuration(memory, &graph)?;
                    }
                    Ok(())
                });
                if result.is_ok() {
                    self.native_tools.memory.selected_app = selected;
                } else {
                    self.native_tools.memory.selected_app =
                        self.controller.session_snapshot().active_app;
                    let page = crate::ide::native_tools::NativeTools::entry_page(
                        &self.native_tools.memory.selected_app,
                    );
                    self.native_tools.open(page);
                }
                result
            }
            Request::GraphConfiguration(name, graph) => self.controller.mutate_memory(|memory| {
                if memory.active_app == name {
                    Self::store_graph_configuration(memory, &graph)?;
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
                    let repaint = ctx.clone();
                    let thread = std::thread::spawn(move || {
                        let result = operation.calculate(cancel, &snapshot);
                        let _ = tx.send(result);
                        repaint.request_repaint();
                    });
                    self.model_task = Some(ModelTask {
                        thread: Some(thread),
                        _owner: owner,
                        rx,
                        calculation,
                        source,
                    });
                    self.state.vm_state = crate::ide::VmExecutionState::Running;
                    Ok(())
                }
            }
            Request::Close => {
                self.native_tools.close();
                self.switch_native_environment(false, ctx);
                Ok(())
            }
            Request::Insert(text) => {
                self.native_tools.close();
                self.state.native.edit(|e| e.insert(&text));
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
                    || !(0..=3).contains(&settings.hformat)
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
            Request::Cas(expression) | Request::CasCatalog(expression, _) => {
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
                    let cas_source = if diagnostics.has_errors() {
                        expression.clone()
                    } else {
                        ppl_compiler::cas_entry_source(&parsed)
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
                        let result = session.evaluate_with_home(
                            &cas_source,
                            &home,
                            &Default::default(),
                            &cancel,
                        );
                        if catalog_cas {
                            session.last_value = previous_answer;
                        }
                        let _ = tx.send((session, expression, result));
                        repaint.request_repaint();
                    });
                    self.cas_task = Some(CasTask {
                        catalog: catalog_cas,
                        deleted: catalog_deleted,
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
            .input_epoch
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
                    reply: tx,
                    epoch,
                });
            app.poll_native_tools(&ctx);
            assert_eq!(rx.try_recv().unwrap(), Ok(true));
            assert_eq!(app.native_tools.memory.selected_app, "Finance");
            assert_eq!(app.native_tools.page, Some(expected));
        }
    }
}
