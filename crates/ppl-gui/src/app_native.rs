//! Native session presentation and the single keyboard/keypad input owner.
use super::*;
use crate::ide::native::{CatalogEntry, InputOwner, NativeCaller, NativeOperation, NativeView};
use crate::keymap::*;
use std::sync::atomic::Ordering;

impl IdeDesktopApp {
    /// Optional observation after a transition has committed. Never drives
    /// execution or requests a repaint; old typed results alone are not proof
    /// that a new operation completed.
    fn trace_native_checkpoint(&mut self, transition: &str) {
        if std::env::var_os("PPL_IDE_INPUT_TRACE").is_none() {
            return;
        }
        let mut snapshot = self.native_input_snapshot();
        snapshot.as_object_mut().unwrap().remove("trace");
        self.state.native.record(serde_json::json!({
            "action":"state_checkpoint", "transition":transition,
            "vm_state":self.state.vm_state.label(),
            "pending_operation":self.state.native.operation.is_some(),
            "calculator_focus":self.state.calc_input_focus,
            "state":snapshot,
        }));
    }
    /// A rejected or superseded compile never became a VM operation. Retire
    /// its UI ownership so the caller remains usable without a fake result.
    pub(super) fn retire_native_compile(&mut self, reason: &str) {
        self.state.native.operation.take();
        self.state.native.outcome_handled = true;
        self.state.native.last_outcome = Some(serde_json::json!({
            "status": "blocked", "reason": reason
        }));
        self.state.native.set_view(self.state.native.caller.view());
        self.release_native_input();
        self.state.vm_state = crate::ide::VmExecutionState::Idle;
        self.native_vm_state = crate::ide::VmExecutionState::Idle;
        self.state.status_message = reason.into();
        self.controller.shared.notify_redraw();
    }

    pub(crate) fn refresh_native_catalog(&mut self) {
        let old = self
            .state
            .native
            .catalog
            .get(self.state.native.selected)
            .map(|e| (e.tab, e.name.clone()));
        let active = self.editor.active_tab().map(|t| t.id);
        let mut catalog = Vec::new();
        for tab in &mut self.editor.tabs {
            tab.sync();
            let revision = tab.revision();
            let hash = ppl_engine::source::hash(tab.content.as_bytes());
            for function in &tab.doc.analysis().symbols.functions {
                if function.exported {
                    catalog.push(CatalogEntry {
                        tab: tab.id,
                        revision,
                        file: tab.title.clone(),
                        name: function.name.clone(),
                        params: function.params.len(),
                        source_sha256: hash.clone(),
                    });
                }
            }
        }
        if catalog != self.state.native.catalog {
            self.state.native.selected = old
                .and_then(|key| catalog.iter().position(|e| (e.tab, e.name.clone()) == key))
                .or_else(|| catalog.iter().position(|e| Some(e.tab) == active))
                .unwrap_or(0);
            self.state.native.catalog = catalog;
            self.state.native.touch_view();
        }
    }

    fn ide_modal_open(&self) -> bool {
        self.dialogs.has_open()
            || self.calc_status_open
            || self.program_manager.open
            || !matches!(self.send.stage, crate::ide::send::SendStage::Idle)
    }

    pub(crate) fn native_input_owner(&self) -> InputOwner {
        if self.ide_modal_open() {
            return InputOwner::IdeModal;
        }
        if self.state.firmware_backend {
            return if self.state.calc_input_focus {
                InputOwner::Firmware
            } else {
                InputOwner::Editor
            };
        }
        let focused = self.state.calc_input_focus
            || self.state.layout == crate::ide::layout::WorkspaceLayout::Calculator;
        if !focused {
            return InputOwner::Editor;
        }
        if self
            .controller
            .shared
            .active_dialog
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
        {
            return InputOwner::CalculatorModal;
        }
        if focused
            && (self.controller.worker_alive()
                || self.native_stopping
                || matches!(
                    self.state.vm_state,
                    crate::ide::VmExecutionState::Running | crate::ide::VmExecutionState::Paused
                )
                || self.native_compile.as_ref().is_some_and(|j| j.run))
        {
            InputOwner::Program
        } else if focused {
            InputOwner::NativeIdle
        } else {
            InputOwner::Editor
        }
    }

    pub(crate) fn native_input_snapshot(&self) -> serde_json::Value {
        use ppl_vm::host::Dialog;
        let active = self
            .controller
            .shared
            .active_dialog
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let active_dialog = active.as_ref().map(|dialog| match dialog {
            Dialog::Message { text } => serde_json::json!({"kind":"message","text":text}),
            Dialog::Choose { title, items, current } => serde_json::json!({"kind":"choose","title":title,"items":items,"current":current}),
            Dialog::Input { title, fields, error, focus } => serde_json::json!({"kind":"input","title":title,"error":error,"focus":focus,
                "fields":fields.iter().map(|f| serde_json::json!({"label":f.label,"current":f.current,"string_only":f.string_only})).collect::<Vec<_>>() }),
        });
        drop(active);
        let session = self.controller.session_snapshot();
        serde_json::json!({
            "view":self.state.native.view, "caller":self.state.native.caller,
            "owner":self.native_input_owner(), "home_input":self.state.native.home_input,
            "history":self.state.native.history, "run_id":self.state.native.run_id,
            "input_epoch":self.controller.shared.input_epoch.load(Ordering::Acquire),
            "view_epoch":self.state.native.input_epoch,"view_generation":self.native_display_generation(),
            "active_dialog":active_dialog,"worker_active":self.controller.worker_alive(),
            "session_alive":self.controller.session_alive(),"session_generation":session.generation,
            "ans":session.globals.get("Ans").map(crate::ide::native::portable_json),
            "last_value":session.last_value.as_ref().map(crate::ide::native::portable_json),
            "last_outcome":self.state.native.last_outcome,
            "queue_len":self.controller.shared.key_queue.lock().unwrap_or_else(|e| e.into_inner()).len(),
            "held_keys":self.controller.shared.held_keys.lock().unwrap_or_else(|e| e.into_inner()).iter().copied().collect::<Vec<_>>(),
            "catalog":self.state.native.catalog.iter().map(|e| serde_json::json!({"tab":format!("{:?}",e.tab),"name":e.name,"file":e.file,"revision":e.revision,"source_sha256":e.source_sha256})).collect::<Vec<_>>(),
            "selected":self.state.native.selected,"trace":self.state.native.trace,
        })
    }

    pub(super) fn trace_native(
        &mut self,
        origin: &str,
        press: Option<KeyPress>,
        owner: InputOwner,
        action: &str,
    ) {
        let active = self
            .controller
            .shared
            .active_dialog
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|d| format!("{d:?}"));
        self.state.native.record(serde_json::json!({"origin":origin,"owner":owner,"consumed":true,
            "hp_code":press.and_then(|p|p.code),"text":press.and_then(|p|p.ch).map(|c|c.to_string()),
            "action":action,"backend":"native","waiting_state":active,
            "input_epoch":self.controller.shared.input_epoch.load(Ordering::Acquire),
            "source":self.running.as_ref().map(|r|r.file.name.clone()),
            "module_namespace":self.running.as_ref().map(|r|r.namespace.clone()),
            "source_sha256":self.running.as_ref().map(|r|ppl_engine::source::hash(r.file.text.as_bytes())),
            "vm_state":self.state.vm_state.label(),
        }));
    }

    pub(super) fn release_native_input(&mut self) {
        self.keypad.release(&mut self.state);
        self.state.clear_held_keys();
        self.state.key_events.clear();
        self.state.key_transitions.clear();
        self.controller.shared.clear_held_keys();
        self.controller.shared.set_touch(None);
        self.controller
            .shared
            .key_queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.controller
            .shared
            .input_epoch
            .fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn focus_native_calculator(&mut self, ctx: &egui::Context) {
        self.state.calc_input_focus = true;
        ctx.memory_mut(|memory| {
            if let Some(id) = memory.focused() {
                memory.surrender_focus(id);
            }
        });
    }

    pub(super) fn set_native_view(&mut self, view: NativeView, ctx: &egui::Context) {
        self.state.native.set_view(view);
        self.release_native_input();
        self.focus_native_calculator(ctx);
        self.state.vm_state = crate::ide::VmExecutionState::Idle;
        self.native_vm_state = crate::ide::VmExecutionState::Idle;
        self.state.status_message = match view {
            NativeView::Home => "Native Home",
            NativeView::Catalog => "Native Programs",
            _ => "Native calculator",
        }
        .into();
        self.update_display_texture(ctx);
        ctx.request_repaint();
        self.trace_native_checkpoint("view_changed");
    }

    pub(crate) fn native_back(&mut self, ctx: &egui::Context) {
        if self.state.firmware_backend
            || self.controller.worker_alive()
            || self.native_compile.as_ref().is_some_and(|j| j.run)
        {
            return;
        }
        let target = match self.state.native.view {
            NativeView::ProgramOutput | NativeView::Error => self.state.native.caller.view(),
            NativeView::Catalog => NativeView::Home,
            NativeView::Home => {
                self.state.native.home_input.clear();
                NativeView::Home
            }
        };
        self.state.native.outcome_handled = true;
        self.set_native_view(target, ctx);
    }

    pub(crate) fn route_native_press(
        &mut self,
        press: KeyPress,
        origin: &str,
        ctx: &egui::Context,
    ) {
        if self.state.firmware_backend {
            return;
        }
        if origin != "keyboard" {
            self.focus_native_calculator(ctx);
        }
        let owner = self.native_input_owner();
        self.state.input_owner = owner;
        if owner == InputOwner::IdeModal {
            if press.code == Some(KEY_ESC) {
                if !self.dialogs.cancel_top() {
                    if self.calc_status_open {
                        self.calc_status_open = false;
                    } else if self.program_manager.open {
                        self.program_manager.open = false;
                    } else {
                        self.cancel_send_dialog_escape();
                    }
                }
                self.trace_native(origin, Some(press), owner, "ide_modal_escape");
            }
            return;
        }
        if matches!(owner, InputOwner::CalculatorModal | InputOwner::Program) {
            self.trace_native(origin, Some(press), owner, "forwarded_to_program");
            self.controller.shared.push_key_press(press);
            ctx.request_repaint();
            return;
        }
        if owner != InputOwner::NativeIdle {
            return;
        }
        self.trace_native(origin, Some(press), owner, "native_view_input");
        // Text is not a command key: '=' in HDigits:=0 must not act as Enter.
        if self.state.native.view == NativeView::Home {
            if let Some(ch) = press.ch {
                self.state.native.home_input.push(ch);
                self.state.native.touch_view();
                ctx.request_repaint();
                return;
            }
        }
        if press.code == Some(KEY_ESC) {
            self.native_back(ctx);
            return;
        }
        if press.code == Some(KEY_HOME) {
            self.set_native_view(NativeView::Home, ctx);
            return;
        }
        if press.code == Some(KEY_MENU) || (self.state.native.shifted && press.code == Some(KEY_1))
        {
            self.refresh_native_catalog();
            self.set_native_view(NativeView::Catalog, ctx);
            return;
        }
        if press.code == Some(KEY_SHIFT) {
            self.state.native.shifted = !self.state.native.shifted;
            return;
        }
        self.state.native.shifted = false;
        match self.state.native.view {
            NativeView::Home => {
                match press.code {
                    Some(KEY_ENTER) => {
                        self.submit_native_home(ctx);
                        return;
                    }
                    Some(KEY_DEL) => {
                        self.state.native.home_input.pop();
                    }
                    Some(KEY_UP) => {
                        if let Some(entry) = self.state.native.history.back() {
                            self.state.native.home_input = entry.expression.clone();
                        }
                    }
                    Some(KEY_ANS) if press.ch.is_none() => {
                        self.state.native.home_input.push_str("Ans")
                    }
                    _ => {
                        if let Some(ch) = press.ch {
                            self.state.native.home_input.push(ch);
                        } else if let Some(code) = press.code {
                            let token = match code {
                                KEY_SIN => Some("SIN("),
                                KEY_COS => Some("COS("),
                                KEY_TAN => Some("TAN("),
                                KEY_LN => Some("LN("),
                                KEY_LOG => Some("LOG("),
                                KEY_SQR => Some("^2"),
                                _ => None,
                            };
                            if let Some(token) = token {
                                self.state.native.home_input.push_str(token);
                            } else if let Some(ch) = crate::dialog::char_of_key(code) {
                                self.state.native.home_input.push(ch);
                            }
                        }
                    }
                }
                self.state.native.touch_view();
            }
            NativeView::Catalog => match press.code {
                Some(KEY_UP) => {
                    self.state.native.selected = self.state.native.selected.saturating_sub(1)
                }
                Some(KEY_DOWN) => {
                    self.state.native.selected = (self.state.native.selected + 1)
                        .min(self.state.native.catalog.len().saturating_sub(1))
                }
                Some(KEY_ENTER) => {
                    self.run_native_selected(ctx);
                    return;
                }
                _ => {}
            },
            NativeView::ProgramOutput | NativeView::Error => {}
        }
        self.state.native.touch_view();
        ctx.request_repaint();
    }

    pub(crate) fn route_native_keyboard(&mut self, ctx: &egui::Context) {
        if self.state.firmware_backend {
            return;
        }
        let owner = self.native_input_owner();
        self.state.input_owner = owner;
        if !ctx.input(|i| i.focused) {
            self.keypad.release(&mut self.state);
            self.state.clear_held_keys();
            self.controller.shared.clear_held_keys();
            self.controller.shared.set_touch(None);
            return;
        }
        let events = ctx.input(|i| i.events.clone());
        let has_text = events.iter().any(|e| matches!(e, egui::Event::Text(_)));
        let mut consumed = Vec::new();
        let start_epoch = self.controller.shared.input_epoch.load(Ordering::Acquire);
        let start_view_epoch = self.state.native.input_epoch;
        let mut transitioned = false;
        for (index, event) in events.iter().enumerate() {
            if transitioned {
                if let egui::Event::Key {
                    key,
                    pressed: false,
                    ..
                } = event
                {
                    if let Some(code) = crate::keymap::egui_key_to_hp(*key) {
                        self.state.set_key_down(code, false);
                        self.controller.shared.set_key_down(code, false);
                    }
                }
                if matches!(event, egui::Event::Key { .. } | egui::Event::Text(_)) {
                    consumed.push(index);
                }
                continue;
            }
            match event {
                egui::Event::Key {
                    key,
                    pressed,
                    repeat,
                    modifiers,
                    ..
                } if !modifiers.command => {
                    if *key == egui::Key::F5 {
                        continue;
                    }
                    let Some(code) = crate::keymap::egui_key_to_hp(*key) else {
                        continue;
                    };
                    if owner == InputOwner::Editor {
                        continue;
                    }
                    if owner == InputOwner::IdeModal && code != KEY_ESC {
                        continue;
                    }
                    if *pressed {
                        if code == KEY_ESC && (*repeat || self.state.keys_down.contains(&code)) {
                            consumed.push(index);
                            continue;
                        }
                        self.state.set_key_down(code, true);
                        if matches!(owner, InputOwner::Program | InputOwner::CalculatorModal) {
                            self.controller.shared.set_key_down(code, true);
                        }
                        let printable = matches!(
                            key,
                            egui::Key::Num0
                                | egui::Key::Num1
                                | egui::Key::Num2
                                | egui::Key::Num3
                                | egui::Key::Num4
                                | egui::Key::Num5
                                | egui::Key::Num6
                                | egui::Key::Num7
                                | egui::Key::Num8
                                | egui::Key::Num9
                                | egui::Key::Plus
                                | egui::Key::Minus
                                | egui::Key::Slash
                                | egui::Key::Period
                                | egui::Key::Comma
                                | egui::Key::Equals
                                | egui::Key::Space
                                | egui::Key::X
                                | egui::Key::S
                                | egui::Key::C
                                | egui::Key::T
                                | egui::Key::L
                                | egui::Key::E
                        );
                        if !printable || !has_text {
                            self.route_native_press(KeyPress::code(code), "keyboard", ctx);
                        }
                        if matches!(code, KEY_ESC | KEY_ENTER) {
                            transitioned = true;
                        }
                    } else {
                        self.state.set_key_down(code, false);
                        self.controller.shared.set_key_down(code, false);
                    }
                    consumed.push(index);
                }
                egui::Event::Text(text)
                    if !matches!(owner, InputOwner::Editor | InputOwner::IdeModal) =>
                {
                    for ch in text.chars() {
                        let mut bytes = [0; 4];
                        let mapped = crate::keymap::map_char(ch.encode_utf8(&mut bytes));
                        self.route_native_press(
                            KeyPress {
                                code: if mapped == Some(KEY_ENTER) {
                                    None
                                } else {
                                    mapped
                                },
                                ch: Some(ch),
                            },
                            "keyboard",
                            ctx,
                        );
                    }
                    consumed.push(index);
                }
                _ => {}
            }
            transitioned |= self.controller.shared.input_epoch.load(Ordering::Acquire)
                != start_epoch
                || self.state.native.input_epoch != start_view_epoch;
        }
        ctx.input_mut(|input| {
            input.events = std::mem::take(&mut input.events)
                .into_iter()
                .enumerate()
                .filter(|(index, _)| !consumed.contains(index))
                .map(|(_, event)| event)
                .collect();
        });
    }

    pub(crate) fn submit_native_home(&mut self, ctx: &egui::Context) {
        if self.controller.worker_alive() || self.native_compile.is_some() {
            return;
        }
        let expression = self.state.native.home_input.trim().to_string();
        if expression.is_empty() {
            return;
        }
        self.refresh_native_catalog();
        let (parsed, diags) = ppl_syntax::parse_expression(&expression);
        if !diags.has_errors() {
            if let ppl_syntax::ast::ExprKind::Call(callee, args) = parsed.kind {
                if let ppl_syntax::ast::ExprKind::Ident(name) = callee.kind {
                    let entries: Vec<_> = self
                        .state
                        .native
                        .catalog
                        .iter()
                        .filter(|e| e.name.eq_ignore_ascii_case(&name))
                        .cloned()
                        .collect();
                    if entries.len() == 1 {
                        let entry = &entries[0];
                        if let Some(index) = self.editor.index_of(entry.tab) {
                            self.editor.active_tab_index = index;
                            self.editor.tabs[index].run_entry = Some(entry.name.clone());
                            self.editor.tabs[index].run_args = args
                                .iter()
                                .map(|a| {
                                    expression[a.span.start as usize..a.span.end as usize]
                                        .to_string()
                                })
                                .collect::<Vec<_>>()
                                .join(",");
                            self.native_home_expression = Some(expression);
                            self.state.native.set_view(NativeView::Home);
                            self.run_source(false, false, ctx);
                            return;
                        }
                    }
                }
            }
        }
        self.release_native_input();
        self.running = None;
        self.state.native.begin(
            NativeOperation::Home {
                expression: expression.clone(),
            },
            NativeCaller::Home,
        );
        if matches!(
            self.controller.evaluate_home(&expression, None),
            crate::ide::execution::RunStart::Started
        ) {
            self.state.vm_state = crate::ide::VmExecutionState::Running;
            self.state.native.home_input.clear();
            self.focus_native_calculator(ctx);
            ctx.request_repaint();
        }
    }

    pub(super) fn complete_native_operation(
        &mut self,
        value: Option<String>,
        error: Option<String>,
    ) {
        let snapshot = self.controller.session_snapshot();
        let typed = if error.is_none() {
            snapshot
                .last_value
                .as_ref()
                .map(crate::ide::native::portable_json)
        } else {
            None
        };
        let operation = self.state.native.operation.take();
        let home_evaluation = matches!(operation, Some(NativeOperation::Home { .. }));
        let home_expression = match &operation {
            Some(NativeOperation::Home { expression }) => Some(expression.clone()),
            Some(NativeOperation::Program {
                home_expression, ..
            }) => home_expression.clone(),
            None => None,
        };
        self.state.native.outcome_handled = true;
        self.state.native.error = error.clone();
        if let Some(expression) = home_expression {
            self.state.native.add_history(
                expression.clone(),
                error.clone().or(value.clone()).unwrap_or_default(),
                error.is_some(),
                if error.is_none() { typed.clone() } else { None },
            );
            self.state.native.caller = NativeCaller::Home;
            if error.is_none() {
                self.state.native.home_input.clear();
                if home_evaluation {
                    self.state.native.set_view(NativeView::Home);
                    self.state.vm_state = crate::ide::VmExecutionState::Idle;
                    self.state.status_message = "Native Home".into();
                } else {
                    self.state.native.set_view(NativeView::ProgramOutput);
                }
            } else {
                self.state.native.home_input = expression;
                self.state.native.set_view(NativeView::Error);
            }
        } else if error.is_some() {
            self.state.native.set_view(NativeView::Error);
        } else {
            self.state.native.set_view(NativeView::ProgramOutput);
        }
        self.release_native_input();
        self.state.native.touch_view();
        self.state.native.last_outcome = Some(
            serde_json::json!({"status":if error.is_some(){"error"}else{"success"},"result":value,"error":error,"typed_result":typed}),
        );
        self.state.native.record(serde_json::json!({"origin":"worker","action":"operation_complete","result":value,"error":error,"typed_result":typed,"operation":format!("{operation:?}"),"input_epoch":self.controller.shared.input_epoch.load(Ordering::Acquire)}));
        self.trace_native_checkpoint("operation_complete");
    }

    pub(super) fn finish_native_stop(&mut self, outcome: Option<WorkerOutcome>) {
        match &outcome {
            Some(WorkerOutcome::Success {
                value_str,
                steps,
                elapsed,
            }) => {
                if self.state.native.operation.is_some() {
                    self.console.print_diagnostic(format!(
                        "--- Program finished: returned {value_str} ({steps} steps) ---"
                    ));
                }
                self.state.vm_state = crate::ide::VmExecutionState::Terminated;
                self.state.step_count = *steps;
                self.state.elapsed_time = *elapsed;
                self.complete_native_operation(Some(value_str.clone()), None);
                self.state.status_message = "Program finished".into();
                return;
            }
            Some(WorkerOutcome::Error(error)) if error.kind != ppl_vm::ErrorKind::Interrupted => {
                self.console.print_error(error.to_string());
                self.state.vm_state = crate::ide::VmExecutionState::Error(error.to_string());
                self.complete_native_operation(None, Some(error.to_string()));
                self.state.status_message = "Runtime error".into();
                return;
            }
            _ => {}
        }
        let operation = self.state.native.operation.take();
        if operation.is_some() {
            self.console
                .print_diagnostic("--- Execution interrupted ---");
            self.state.native.last_outcome = Some(
                serde_json::json!({"status":if outcome.is_some(){"interrupted"}else{"stopped_without_outcome"},"worker_outcome":format!("{outcome:?}"),"operation":format!("{operation:?}"),"session_generation":self.controller.session_snapshot().generation}),
            );
        }
        self.state.native.outcome_handled = true;
        self.state.native.set_view(self.state.native.caller.view());
        self.release_native_input();
        self.state.vm_state = crate::ide::VmExecutionState::Idle;
        self.native_vm_state = crate::ide::VmExecutionState::Idle;
        self.state.status_message = "Stopped".into();
    }

    pub(crate) fn run_native_selected(&mut self, ctx: &egui::Context) {
        if self.controller.worker_alive() || self.native_compile.is_some() {
            return;
        }
        self.refresh_native_catalog();
        let Some(entry) = self
            .state
            .native
            .catalog
            .get(self.state.native.selected)
            .cloned()
        else {
            return;
        };
        let Some(index) = self.editor.index_of(entry.tab) else {
            return;
        };
        self.editor.active_tab_index = index;
        self.editor.tabs[index].run_entry = Some(entry.name.clone());
        self.state.native.set_view(NativeView::Catalog);
        if entry.params > 0 {
            self.open_run_config();
        } else {
            self.editor.tabs[index].run_args.clear();
            self.run_source(false, false, ctx);
        }
    }
}
