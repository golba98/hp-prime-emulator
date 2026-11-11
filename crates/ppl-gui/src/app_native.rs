//! Native session presentation and the single keyboard/keypad input owner.
use super::*;
use crate::ide::native::{CatalogEntry, InputOwner, NativeCaller, NativeOperation, NativeView};
use crate::ide::native::{
    expression::EditCommand,
    keys::{self, KeyAction},
};
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

    fn catalog_editor_index(&mut self, entry: &CatalogEntry) -> Option<usize> {
        if let Some(tab) = entry.tab {
            return self.editor.index_of(tab);
        }
        let source = self
            .controller
            .session_snapshot()
            .programs
            .get(&entry.file)?
            .clone();
        if ppl_engine::source::hash(source.as_bytes()) != entry.source_sha256 {
            return None;
        }
        self.editor
            .new_tab(format!("{}.hpprgm", entry.file), source);
        Some(self.editor.active_tab_index)
    }
    pub(crate) fn refresh_native_catalog(&mut self) {
        let old = self
            .state
            .native
            .catalog
            .get(self.state.native.selected)
            .map(|e| (e.tab, e.file.clone(), e.name.clone()));
        let active = self.editor.active_tab().map(|t| t.id);
        let mut catalog = Vec::new();
        for tab in &mut self.editor.tabs {
            tab.sync();
            let revision = tab.revision();
            let hash = ppl_engine::source::hash(tab.content.as_bytes());
            for function in &tab.doc.analysis().symbols.functions {
                if function.exported {
                    catalog.push(CatalogEntry {
                        tab: Some(tab.id),
                        revision,
                        file: tab.title.clone(),
                        name: function.name.clone(),
                        params: function.params.len(),
                        source_sha256: hash.clone(),
                    });
                }
            }
        }
        let programs = self.controller.session_snapshot().programs;
        if programs != self.installed_catalog_sources {
            self.installed_catalog_entries.clear();
            for (name, source) in &programs {
                let (ast, diagnostics) = ppl_syntax::parse(source);
                if diagnostics.has_errors() {
                    continue;
                }
                for item in ast.items {
                    if let ppl_syntax::ast::Item::Function(function) = item {
                        if function.exported {
                            self.installed_catalog_entries.push(CatalogEntry {
                                tab: None,
                                revision: 0,
                                file: name.clone(),
                                name: function.name.name,
                                params: function.params.len(),
                                source_sha256: ppl_engine::source::hash(source.as_bytes()),
                            });
                        }
                    }
                }
            }
            self.installed_catalog_sources = programs;
        }
        for entry in &self.installed_catalog_entries {
            if !catalog
                .iter()
                .any(|open| open.name == entry.name && open.source_sha256 == entry.source_sha256)
            {
                catalog.push(entry.clone());
            }
        }
        if catalog != self.state.native.catalog {
            self.state.native.selected = old
                .and_then(|key| {
                    catalog
                        .iter()
                        .position(|e| (e.tab, e.file.clone(), e.name.clone()) == key)
                })
                .or_else(|| catalog.iter().position(|e| e.tab == active))
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
            Dialog::Matrix(matrix) => serde_json::json!({"kind":"matrix","title":matrix.title,
                "rows":matrix.rows,"cols":matrix.cols,"cells":matrix.cells,"read_only":matrix.read_only,
                "error":matrix.error,"focus":matrix.focus}),
            Dialog::List { title, items, .. } => serde_json::json!({"kind":"list","title":title,"items":items}),
            Dialog::MessageChoice { text, ok_cancel } => serde_json::json!({"kind":"message_choice","text":text,"ok_cancel":ok_cancel}),
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
            "history":self.state.native.history,"cursor":self.state.native.editor.cursor(),"selection":self.state.native.editor.selection(),"shifted":self.state.native.shifted,"alpha":format!("{:?}",self.state.native.alpha),"sleeping":self.state.native.sleeping,"user_mode":self.state.native.user_keys.mode.indicator(), "run_id":self.state.native.run_id,
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
        if self.dismiss_native_catalog_program(ctx) {
            return;
        }
        if self.finish_math_entry(false, ctx) {
            return;
        }
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
                self.state.native.edit(|e| e.set_text(""));
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
        if origin != "keyboard" && self.native_tools.page.is_none() {
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
        if press.code == Some(KEY_ON)
            && matches!(owner, InputOwner::CalculatorModal | InputOwner::Program)
        {
            self.trace_native(origin, Some(press), owner, "on_interrupt");
            self.handle_action(IdeAction::StopProgram, ctx);
            return;
        }
        if owner == InputOwner::CalculatorModal {
            if let Some(code) = press.code {
                if code == KEY_SHIFT {
                    self.state.native.shifted = !self.state.native.shifted;
                    return;
                }
                let shifted = std::mem::take(&mut self.state.native.shifted);
                if code == KEY_ALPHA {
                    self.state
                        .native
                        .alpha
                        .press(shifted, self.state.native.alpha_default_lower);
                    return;
                }
                if let Some(lower) = self.state.native.alpha.consume(shifted) {
                    if let Some(text) = keys::alpha_character(code, lower) {
                        self.controller
                            .shared
                            .push_dialog_event(crate::dialog::DialogEvent::Text(text));
                        return;
                    }
                }
                let event = match keys::resolve_key(code, shifted) {
                    Some(KeyAction::Insert(text)) => {
                        Some(crate::dialog::DialogEvent::Text(text.into()))
                    }
                    Some(KeyAction::Polar) => Some(crate::dialog::DialogEvent::Text("∠".into())),
                    Some(KeyAction::Sexagesimal) => Some(crate::dialog::DialogEvent::Sexagesimal),
                    Some(KeyAction::Fraction) => Some(crate::dialog::DialogEvent::Template(
                        crate::ide::native::expression::TemplateKind::Fraction,
                    )),
                    Some(KeyAction::AppVariable) => Some(crate::dialog::DialogEvent::Text(
                        self.native_independent_variable().into(),
                    )),
                    Some(KeyAction::Template(template)) => {
                        Some(crate::dialog::DialogEvent::Template(template))
                    }
                    Some(KeyAction::Edit(command)) => {
                        Some(crate::dialog::DialogEvent::Edit(command))
                    }
                    Some(KeyAction::Sign) => Some(crate::dialog::DialogEvent::Sign),
                    Some(KeyAction::Copy) => Some(crate::dialog::DialogEvent::Copy),
                    Some(KeyAction::Paste) => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                        return;
                    }
                    _ => None,
                };
                if let Some(event) = event {
                    self.controller.shared.push_dialog_event(event);
                    ctx.request_repaint();
                    return;
                }
            }
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
        if self.state.native.sleeping {
            if press.code == Some(KEY_ON) {
                self.state.native.sleeping = false;
                self.state.native.touch_view();
                ctx.request_repaint();
            }
            return;
        }
        if let Some(ch) = press.ch {
            let shifted = std::mem::take(&mut self.state.native.shifted);
            let lower = self.state.native.alpha.consume(shifted);
            self.state.native.user_keys.mode.consume();
            let text = if ch.is_ascii_alphabetic() {
                match lower {
                    Some(true) => ch.to_ascii_lowercase().to_string(),
                    Some(false) => ch.to_ascii_uppercase().to_string(),
                    None => ch.to_string(),
                }
            } else {
                ch.to_string()
            };
            if self.state.native.view == NativeView::Home {
                self.state.native.edit(|e| e.type_text(&text));
            }
            ctx.request_repaint();
            return;
        }
        let Some(code) = press.code else {
            return;
        };
        if self.dispatch_native_user_key(code, ctx) {
            return;
        }
        if code == KEY_SHIFT {
            self.state.native.shifted = !self.state.native.shifted;
            self.state.native.touch_view();
            ctx.request_repaint();
            return;
        }
        let shifted = std::mem::take(&mut self.state.native.shifted);
        if code == KEY_ALPHA {
            self.state
                .native
                .alpha
                .press(shifted, self.state.native.alpha_default_lower);
            self.state.native.touch_view();
            ctx.request_repaint();
            return;
        }
        if let Some(lower) = self.state.native.alpha.consume(shifted) {
            if let Some(text) = keys::alpha_character(code, lower) {
                if self.native_tools.page.is_some() {
                    ctx.input_mut(|i| i.events.push(egui::Event::Text(text)));
                } else if self.state.native.view == NativeView::Home {
                    self.state.native.edit(|e| e.insert_math_token(&text));
                }
                ctx.request_repaint();
                return;
            }
        }
        let Some(mut action) = keys::resolve_key(code, shifted) else {
            return;
        };
        if action == KeyAction::AppVariable {
            action = KeyAction::Insert(self.native_independent_variable());
        }
        if self.native_tools.page.is_some() && self.route_native_tool_action(action, ctx) {
            return;
        }
        match action {
            KeyAction::Programs => {
                self.native_tools
                    .open(crate::ide::native_tools::Page::Programs);
                return;
            }
            KeyAction::On => {
                self.native_back(ctx);
                return;
            }
            KeyAction::Off => {
                self.state.native.sleeping = true;
                self.release_native_input();
            }
            KeyAction::Back => {
                self.native_back(ctx);
                return;
            }
            KeyAction::Home => {
                self.switch_native_environment(false, ctx);
                return;
            }
            KeyAction::Tool(tool) => self.state.native.pending_tool = Some(tool),
            KeyAction::Copy => self.native_clipboard(ctx, false),
            KeyAction::Paste => ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste),
            KeyAction::History(delta) if self.state.native.view == NativeView::Catalog => {
                self.state.native.selected = if delta < 0 {
                    self.state.native.selected.saturating_sub(1)
                } else {
                    (self.state.native.selected + 1)
                        .min(self.state.native.catalog.len().saturating_sub(1))
                };
            }
            KeyAction::Evaluate if self.state.native.view == NativeView::Catalog => {
                self.run_native_selected(ctx);
                return;
            }
            _ if self.state.native.view != NativeView::Home => {}
            KeyAction::Insert(text) => self.state.native.edit(|e| e.insert_math_token(text)),
            KeyAction::Polar => self.native_polar_key(),
            KeyAction::Sexagesimal => self.native_sexagesimal_key(),
            KeyAction::Fraction => {
                self.native_fraction_key();
            }
            KeyAction::AppVariable => unreachable!("contextual variable resolved before dispatch"),
            KeyAction::Template(template) => {
                self.state.native.edit(|e| e.insert_template(template))
            }
            KeyAction::Edit(edit) => self.state.native.edit(|e| e.apply(edit)),
            KeyAction::History(delta) => self.state.native.history_move(delta),
            KeyAction::Sign => self.state.native.edit(|e| e.toggle_sign()),
            KeyAction::Evaluate => {
                self.submit_native_home(ctx);
                return;
            }
            KeyAction::Approximate => {
                let expression = self.state.native.home_input.clone();
                self.state
                    .native
                    .edit(|e| e.set_text(&format!("APPROX({expression})")));
                self.submit_native_home(ctx);
                return;
            }
            KeyAction::None | KeyAction::Shift | KeyAction::Alpha => {}
        }
        self.state.native.touch_view();
        ctx.request_repaint();
    }

    fn native_clipboard(&mut self, ctx: &egui::Context, cut: bool) {
        if self.state.native.view != NativeView::Home {
            return;
        }
        self.state
            .native
            .editor
            .sync_external(&self.state.native.home_input);
        let mut selected = self.state.native.editor.selected_text();
        if selected.is_empty() && self.state.native.history_position.is_some() {
            selected = self.state.native.home_input.clone();
        }
        if !selected.is_empty() {
            ctx.copy_text(selected);
            if cut {
                self.state.native.edit(|e| {
                    e.cut();
                });
            }
        }
    }
    /// Returns true only when the focused Native Home document owns the edit.
    fn native_edit_command(&mut self, ctx: &egui::Context, command: EditCommand) -> bool {
        if self.state.native.view != NativeView::Home {
            return false;
        }
        self.state.native.edit(|e| e.apply(command));
        ctx.request_repaint();
        true
    }
    pub(crate) fn route_native_pointer(&mut self, ctx: &egui::Context) {
        if self.state.firmware_backend
            || self.native_input_owner() != InputOwner::NativeIdle
            || self.state.native.sleeping
        {
            return;
        }
        let (pressed, down, pos, shift, double) = ctx.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.interact_pos(),
                i.modifiers.shift,
                i.pointer
                    .button_double_clicked(egui::PointerButton::Primary),
            )
        });
        if !down {
            return;
        }
        let Some(touch) = pos.and_then(|p| self.display.project_pointer_pos(p)) else {
            return;
        };
        match self.state.native.view {
            NativeView::Catalog if pressed && (29..205).contains(&touch.y) => {
                let first = self.state.native.selected.saturating_sub(7);
                let index = first + ((touch.y - 29) / 22) as usize;
                if index < self.state.native.catalog.len() {
                    self.state.native.selected = index;
                    if double {
                        self.run_native_selected(ctx);
                    }
                }
            }
            NativeView::Home if touch.y >= 202 => {
                let byte = self.state.native.editor_byte_at_position(touch.x, touch.y);
                self.state
                    .native
                    .edit(|e| e.set_cursor(byte, shift || !pressed));
                self.state.native.history_position = None;
            }
            NativeView::Home if pressed && (29..185).contains(&touch.y) => {
                let index =
                    self.state.native.history_first_visible() + ((touch.y - 29) / 39) as usize;
                if let Some(entry) = self.state.native.history.get(index) {
                    let text = if (touch.y - 29) % 39 >= 16 {
                        entry.result.clone()
                    } else {
                        entry.expression.clone()
                    };
                    self.state.native.edit(|e| e.set_text(&text));
                    self.state.native.history_position = Some(index);
                }
            }
            _ => {}
        }
        self.state.native.touch_view();
        ctx.request_repaint();
    }

    fn native_polar_key(&mut self) {
        use crate::ide::native::polar_display;
        if !self.state.native.home_input.is_empty() || self.state.native.history.is_empty() {
            self.state.native.edit(|editor| editor.insert("∠"));
            return;
        }
        let settings = self.controller.session_snapshot().settings;
        let index = self
            .state
            .native
            .history_position
            .unwrap_or(self.state.native.history.len() - 1);
        let entry = &mut self.state.native.history[index];
        let value = entry.typed_result.as_ref().and_then(polar_display::value);
        let Some(value) = value else {
            self.state.status_message = "Polar display requires a numeric result".into();
            return;
        };
        if entry.result.contains('∠') {
            entry.result = value.format(settings.format());
        } else {
            match polar_display::format(value, &settings) {
                Ok(result) => entry.result = result,
                Err(error) => self.state.status_message = error,
            }
        }
        self.state.native.touch_view();
    }

    fn native_sexagesimal_key(&mut self) {
        use crate::ide::native::sexagesimal;
        if !self.state.native.home_input.is_empty() || self.state.native.history.is_empty() {
            self.state.native.edit(sexagesimal::insert_unit);
            return;
        }
        let index = self
            .state
            .native
            .history_position
            .unwrap_or(self.state.native.history.len() - 1);
        let entry = &mut self.state.native.history[index];
        let decimal = entry
            .typed_result
            .as_ref()
            .filter(|value| value["type"] == "real")
            .and_then(|value| value["value"].as_str());
        let Some(decimal) = decimal else {
            self.state.status_message = "Hexagesimal display requires a real-valued result".into();
            return;
        };
        if entry.result.contains('°') {
            entry.result = decimal.to_owned();
        } else {
            match sexagesimal::format_decimal(decimal) {
                Ok(result) => entry.result = result,
                Err(error) => self.state.status_message = error,
            }
        }
        self.state.native.touch_view();
    }

    fn native_fraction_key(&mut self) {
        if !self.state.native.home_input.is_empty() || self.state.native.history.is_empty() {
            self.state.native.edit(|e| {
                e.insert_template(crate::ide::native::expression::TemplateKind::Fraction)
            });
            return;
        }
        let index = self
            .state
            .native
            .history_position
            .unwrap_or(self.state.native.history.len() - 1);
        let entry = &mut self.state.native.history[index];
        let decimal = entry
            .typed_result
            .as_ref()
            .filter(|value| value["type"] == "real")
            .and_then(|value| value["value"].as_str());
        let Some(decimal) = decimal else {
            self.state.status_message = "Fraction display requires a real-valued result".into();
            return;
        };
        let Some(display) =
            crate::ide::native::decimal_display::FractionDisplay::from_decimal(decimal)
        else {
            self.state.status_message =
                "This decimal exceeds the exact fraction display range".into();
            return;
        };
        entry.result = display.next(&entry.result, decimal);
        self.state.native.touch_view();
    }

    fn native_independent_variable(&self) -> &'static str {
        use crate::ide::native_apps::GraphKind;
        if self.state.native.cas_mode && self.math_entry.is_none() {
            return "x";
        }
        match self
            .native_tools
            .memory
            .graphs
            .get(&self.native_tools.memory.selected_app)
            .map(|a| a.kind)
        {
            Some(GraphKind::Parametric) => "T",
            Some(GraphKind::Polar) => "θ",
            Some(GraphKind::Sequence) => "N",
            _ => "X",
        }
    }

    /// Calculator view shortcuts remain active while egui owns a catalog or
    /// settings field. Ordinary text, clipboard and navigation stay with that
    /// field, so dispatch never edits the hidden Home expression.
    fn route_native_tool_keyboard(&mut self, ctx: &egui::Context) {
        let events = ctx.input(|i| i.events.clone());
        let mut consumed = Vec::new();
        let start_page = self.native_tools.page;
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
                    }
                }
                if matches!(
                    event,
                    egui::Event::Key { .. } | egui::Event::Text(_) | egui::Event::Paste(_)
                ) {
                    consumed.push(index);
                }
                continue;
            }
            let egui::Event::Key {
                key,
                pressed,
                repeat,
                modifiers,
                ..
            } = event
            else {
                continue;
            };
            use crate::ide::native_tools::Page;
            let catalog_navigation = matches!(
                key,
                egui::Key::ArrowUp | egui::Key::ArrowDown | egui::Key::Enter
            ) && (self.native_tools.page == Some(Page::Apps)
                || (ctx.memory(|memory| memory.focused()).is_none()
                    && matches!(
                        self.native_tools.page,
                        Some(
                            Page::Programs
                                | Page::Notes
                                | Page::Variables
                                | Page::CasVariables
                                | Page::Lists
                                | Page::Matrices
                                | Page::Base
                        )
                    )));
            let plot_navigation = matches!(self.native_tools.page, Some(Page::Plot | Page::Base))
                && ctx.memory(|memory| memory.focused()).is_none()
                && matches!(
                    key,
                    egui::Key::ArrowLeft
                        | egui::Key::ArrowRight
                        | egui::Key::ArrowUp
                        | egui::Key::ArrowDown
                        | egui::Key::Plus
                        | egui::Key::Minus
                );
            if !catalog_navigation
                && !plot_navigation
                && !matches!(
                    key,
                    egui::Key::F1
                        | egui::Key::F2
                        | egui::Key::F3
                        | egui::Key::F4
                        | egui::Key::F6
                        | egui::Key::F7
                        | egui::Key::F8
                        | egui::Key::F9
                        | egui::Key::F10
                        | egui::Key::F11
                        | egui::Key::F12
                        | egui::Key::Escape
                        | egui::Key::PageUp
                        | egui::Key::PageDown
                )
                || modifiers.command
                || modifiers.alt
            {
                continue;
            }
            let Some(code) = crate::keymap::egui_key_to_hp(*key) else {
                continue;
            };
            consumed.push(index);
            self.state.set_key_down(code, *pressed);
            if *pressed && (!repeat || catalog_navigation || plot_navigation) {
                if modifiers.shift {
                    self.state.native.shifted = true;
                }
                self.route_native_press(KeyPress::code(code), "keyboard", ctx);
                transitioned = self.native_tools.page != start_page
                    || self.state.native.pending_tool.is_some()
                    || self.state.native.sleeping;
            }
        }
        ctx.input_mut(|i| {
            i.events = std::mem::take(&mut i.events)
                .into_iter()
                .enumerate()
                .filter(|(index, _)| !consumed.contains(index))
                .map(|(_, event)| event)
                .collect();
        });
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
        if owner == InputOwner::Editor {
            return;
        }
        if owner == InputOwner::NativeIdle && self.native_tools.page.is_some() {
            self.route_native_tool_keyboard(ctx);
            return;
        }
        let events = ctx.input(|i| i.events.clone());
        let has_text = events.iter().any(|e| matches!(e, egui::Event::Text(_)));
        let start_epoch = self.controller.shared.input_epoch.load(Ordering::Acquire);
        let start_view = self.state.native.input_epoch;
        let mut consumed = Vec::new();
        let mut transitioned = false;
        let mut suppress_text_events = 0usize;
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
                if matches!(
                    event,
                    egui::Event::Key { .. } | egui::Event::Text(_) | egui::Event::Paste(_)
                ) {
                    consumed.push(index);
                }
                continue;
            }
            let modal = owner == InputOwner::CalculatorModal;
            match event {
                egui::Event::Copy if owner != InputOwner::IdeModal => {
                    if modal {
                        self.controller
                            .shared
                            .push_dialog_event(crate::dialog::DialogEvent::Copy);
                    } else {
                        self.native_clipboard(ctx, false);
                    }
                    consumed.push(index);
                }
                egui::Event::Cut if owner != InputOwner::IdeModal => {
                    if modal {
                        self.controller
                            .shared
                            .push_dialog_event(crate::dialog::DialogEvent::Cut);
                    } else {
                        self.native_clipboard(ctx, true);
                    }
                    consumed.push(index);
                }
                egui::Event::Paste(text) if owner != InputOwner::IdeModal => {
                    if modal {
                        self.controller
                            .shared
                            .push_dialog_event(crate::dialog::DialogEvent::Text(text.clone()));
                    } else if owner == InputOwner::NativeIdle
                        && self.state.native.view == NativeView::Home
                    {
                        if text.chars().any(char::is_control) {
                            self.state.status_message="Calculator entry accepts one expression; paste PPL source in the IDE".into();
                        } else {
                            self.state.native.edit(|e| e.type_text(text));
                        }
                    }
                    consumed.push(index);
                    ctx.request_repaint();
                }
                egui::Event::Key {
                    key,
                    pressed,
                    repeat,
                    modifiers,
                    ..
                } => {
                    if *key == egui::Key::F5 {
                        continue;
                    }
                    if owner == InputOwner::IdeModal && *key != egui::Key::Escape {
                        continue;
                    }
                    let selection = modifiers.shift || self.state.native.shifted;
                    let edit = match key {
                        egui::Key::ArrowLeft => Some(EditCommand::Left(selection)),
                        egui::Key::ArrowRight => Some(EditCommand::Right(selection)),
                        egui::Key::Home => Some(EditCommand::Home(selection)),
                        egui::Key::End => Some(EditCommand::End(selection)),
                        egui::Key::Delete => Some(EditCommand::Delete),
                        egui::Key::A if modifiers.command => Some(EditCommand::SelectAll),
                        egui::Key::Z if modifiers.command => Some(if modifiers.shift {
                            EditCommand::Redo
                        } else {
                            EditCommand::Undo
                        }),
                        egui::Key::Y if modifiers.command => Some(EditCommand::Redo),
                        _ => None,
                    };
                    if modifiers.command {
                        if *pressed {
                            if let Some(edit) = edit {
                                if modal {
                                    self.controller
                                        .shared
                                        .push_dialog_event(crate::dialog::DialogEvent::Edit(edit));
                                } else {
                                    self.native_edit_command(ctx, edit);
                                }
                            } else {
                                match key {
                                    egui::Key::C => {
                                        if modal {
                                            self.controller.shared.push_dialog_event(
                                                crate::dialog::DialogEvent::Copy,
                                            );
                                        } else {
                                            self.native_clipboard(ctx, false);
                                        }
                                    }
                                    egui::Key::X => {
                                        if modal {
                                            self.controller
                                                .shared
                                                .push_dialog_event(crate::dialog::DialogEvent::Cut);
                                        } else {
                                            self.native_clipboard(ctx, true);
                                        }
                                    }
                                    egui::Key::V => {
                                        ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste)
                                    }
                                    _ => continue,
                                }
                            }
                        }
                        consumed.push(index);
                        continue;
                    }
                    if let Some(edit) = edit {
                        if *pressed {
                            self.state.native.shifted = false;
                        }
                        if modal
                            || (owner == InputOwner::NativeIdle
                                && self.state.native.view == NativeView::Home)
                        {
                            if *pressed {
                                if modal {
                                    self.controller
                                        .shared
                                        .push_dialog_event(crate::dialog::DialogEvent::Edit(edit));
                                } else {
                                    self.native_edit_command(ctx, edit);
                                }
                            }
                            consumed.push(index);
                            continue;
                        }
                    }
                    let Some(mut code) = crate::keymap::egui_key_to_hp(*key) else {
                        continue;
                    };
                    if *key == egui::Key::Tab && modifiers.shift {
                        code = KEY_SHIFT;
                    }
                    if !pressed && *key == egui::Key::Tab {
                        for tab_code in [KEY_SHIFT, KEY_ALPHA] {
                            self.state.set_key_down(tab_code, false);
                            self.controller.shared.set_key_down(tab_code, false);
                        }
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
                        let calculator_modifier = self.state.native.shifted
                            || self.state.native.alpha != keys::AlphaMode::Off;
                        if printable && has_text && calculator_modifier {
                            suppress_text_events += 1;
                        }
                        if !printable || !has_text || calculator_modifier {
                            if modifiers.shift
                                && *key != egui::Key::Tab
                                && !printable
                                && owner == InputOwner::NativeIdle
                            {
                                self.state.native.shifted = true;
                            }
                            self.route_native_press(KeyPress::code(code), "keyboard", ctx);
                        }
                    } else {
                        self.state.set_key_down(code, false);
                        self.controller.shared.set_key_down(code, false);
                    }
                    consumed.push(index);
                }
                egui::Event::Text(text) if owner != InputOwner::IdeModal => {
                    if suppress_text_events > 0 {
                        suppress_text_events -= 1;
                        consumed.push(index);
                        continue;
                    }
                    if modal {
                        self.controller
                            .shared
                            .push_dialog_event(crate::dialog::DialogEvent::Text(text.clone()));
                    } else {
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
                    }
                    consumed.push(index);
                }
                _ => {}
            }
            transitioned = self.controller.shared.input_epoch.load(Ordering::Acquire)
                != start_epoch
                || self.state.native.input_epoch != start_view;
        }
        ctx.input_mut(|i| {
            i.events = std::mem::take(&mut i.events)
                .into_iter()
                .enumerate()
                .filter(|(i, _)| !consumed.contains(i))
                .map(|(_, e)| e)
                .collect()
        });
    }

    pub(crate) fn submit_native_home(&mut self, ctx: &egui::Context) {
        if self.finish_math_entry(true, ctx) {
            return;
        }
        if self.state.native.cas_mode {
            self.handle_native_tool_request(
                crate::ide::native_tools::Request::Cas(self.state.native.home_input.clone()),
                ctx,
            );
            return;
        }
        if self.controller.running() || self.native_compile.is_some() {
            return;
        }
        let expression = self.state.native.home_input.trim().to_string();
        if expression.is_empty() {
            return;
        }
        self.refresh_native_sources();
        let catalog = std::sync::Arc::new(self.native_catalog.sources.clone());
        self.release_native_input();
        self.running = None;
        self.state.native.begin(
            NativeOperation::Home {
                expression: expression.clone(),
            },
            NativeCaller::Home,
        );
        if matches!(
            self.controller
                .evaluate_home_with_catalog(&expression, None, Some(catalog)),
            crate::ide::execution::RunStart::Started
        ) {
            self.state.vm_state = crate::ide::VmExecutionState::Running;
            self.state.native.edit(|e| e.set_text(""));
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
                self.state.native.edit(|e| e.set_text(""));
                if home_evaluation
                    && !self
                        .controller
                        .shared
                        .output_requested
                        .load(Ordering::Acquire)
                {
                    self.state.native.set_view(NativeView::Home);
                    self.state.vm_state = crate::ide::VmExecutionState::Idle;
                    self.state.status_message = "Native Home".into();
                } else {
                    self.state.native.set_view(NativeView::ProgramOutput);
                }
            } else {
                self.state.native.edit(|e| e.set_text(&expression));
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
        self.restore_native_catalog_program_after_stop();
        self.release_native_input();
        self.state.vm_state = crate::ide::VmExecutionState::Idle;
        self.native_vm_state = crate::ide::VmExecutionState::Idle;
        self.state.status_message = "Stopped".into();
    }

    pub(crate) fn run_native_selected(&mut self, ctx: &egui::Context) {
        if self.controller.running() || self.native_compile.is_some() {
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
        let Some(index) = self.catalog_editor_index(&entry) else {
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

#[cfg(test)]
mod remediation_tests {
    use super::*;

    fn home() -> (IdeDesktopApp, egui::Context) {
        let mut app = IdeDesktopApp::default();
        app.state.native.view = NativeView::Home;
        app.state.calc_input_focus = true;
        (app, egui::Context::default())
    }
    fn press(app: &mut IdeDesktopApp, ctx: &egui::Context, code: i64) {
        app.route_native_press(KeyPress::code(code), "virtual", ctx);
    }
    fn wait(app: &mut IdeDesktopApp, predicate: impl Fn(&IdeDesktopApp) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            app.poll_execution();
            if predicate(app) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timeout: {:?}",
                app.state.vm_state
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn shifted_functions_space_ans_alpha_and_lock_use_central_actions() {
        let (mut app, ctx) = home();
        for code in [KEY_SHIFT, KEY_SIN] {
            press(&mut app, &ctx, code);
        }
        assert_eq!(app.state.native.home_input, "ASIN()");
        assert_eq!(app.state.native.editor.cursor(), 5);
        app.state.native.edit(|e| e.set_text(""));
        press(&mut app, &ctx, KEY_ANS);
        assert_eq!(app.state.native.home_input, " ");
        for code in [KEY_SHIFT, KEY_PLUS] {
            press(&mut app, &ctx, code);
        }
        assert_eq!(app.state.native.home_input, " Ans");
        app.state.native.edit(|e| e.set_text(""));
        for code in [
            KEY_ALPHA,
            KEY_VARS,
            KEY_ALPHA,
            KEY_ALPHA,
            KEY_TOOLBOX,
            KEY_TEMPLATE,
            KEY_ALPHA,
        ] {
            press(&mut app, &ctx, code);
        }
        assert_eq!(app.state.native.home_input, "ABC");
        assert_eq!(app.state.native.alpha, keys::AlphaMode::Off);
        for code in [KEY_SHIFT, KEY_ALPHA, KEY_VARS] {
            press(&mut app, &ctx, code);
        }
        assert_eq!(app.state.native.home_input, "ABCa");
    }
    #[test]
    fn structured_fraction_from_keypad_evaluates_and_can_be_edited() {
        let (mut app, ctx) = home();
        for code in [KEY_1, KEY_ABC, KEY_2, KEY_RIGHT, KEY_ENTER] {
            press(&mut app, &ctx, code);
        }
        wait(&mut app, |a| !a.controller.worker_alive());
        assert_eq!(app.state.native.history.back().unwrap().result, "0.5");
        press(&mut app, &ctx, KEY_UP);
        assert_eq!(app.state.native.home_input, "(1)/(2)");
        app.state.native.edit(|e| {
            e.apply(EditCommand::Home(false));
            e.apply(EditCommand::Right(false));
            e.apply(EditCommand::Delete);
            e.insert("3");
        });
        assert_eq!(app.state.native.home_input, "(3)/(2)");
        press(&mut app, &ctx, KEY_ENTER);
        wait(&mut app, |a| !a.controller.worker_alive());
        assert_eq!(app.state.native.history.back().unwrap().result, "1.5");
    }
    #[test]
    fn history_browses_all_entries_and_restores_unsent_draft() {
        let (mut app, ctx) = home();
        for i in 0..10 {
            app.state
                .native
                .add_history(format!("{i}+1"), format!("{}", i + 1), false, None);
        }
        app.state.native.edit(|e| e.set_text("draft"));
        for _ in 0..12 {
            press(&mut app, &ctx, KEY_UP);
        }
        assert_eq!(app.state.native.home_input, "0+1");
        assert_eq!(app.state.native.history_first_visible(), 0);
        for _ in 0..10 {
            press(&mut app, &ctx, KEY_DOWN);
        }
        assert_eq!(app.state.native.home_input, "draft");
        assert_eq!(app.state.native.history_position, None);
        assert_eq!(app.state.native.history.len(), 10);
    }
    #[test]
    fn clipboard_paste_cursor_selection_undo_and_delete_go_to_home() {
        let (mut app, ctx) = home();
        app.state.native.edit(|e| e.set_text("12"));
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let _ = ctx.run(
            egui::RawInput {
                events: vec![
                    key(egui::Key::ArrowLeft, egui::Modifiers::NONE),
                    egui::Event::Paste("3".into()),
                ],
                ..Default::default()
            },
            |ctx| app.route_native_keyboard(ctx),
        );
        assert_eq!(app.state.native.home_input, "132");
        let _ = ctx.run(
            egui::RawInput {
                events: vec![key(egui::Key::Z, egui::Modifiers::COMMAND)],
                ..Default::default()
            },
            |ctx| app.route_native_keyboard(ctx),
        );
        assert_eq!(app.state.native.home_input, "12");
        let _ = ctx.run(
            egui::RawInput {
                events: vec![
                    key(egui::Key::A, egui::Modifiers::COMMAND),
                    egui::Event::Paste("π".into()),
                    key(egui::Key::Home, egui::Modifiers::NONE),
                    key(egui::Key::Delete, egui::Modifiers::NONE),
                ],
                ..Default::default()
            },
            |ctx| app.route_native_keyboard(ctx),
        );
        assert_eq!(app.state.native.home_input, "");
    }
    #[test]
    fn on_interrupts_loop_and_subsequent_run_succeeds() {
        let (mut app, ctx) = home();
        app.editor.active_tab_mut().unwrap().content =
            "EXPORT LOOP() BEGIN REPEAT WAIT(0.01); UNTIL 0; END;".into();
        app.handle_action(IdeAction::RunProgram, &ctx);
        wait(&mut app, |a| a.controller.worker_alive());
        press(&mut app, &ctx, KEY_ON);
        wait(&mut app, |a| {
            !a.controller.worker_alive() && !a.native_stopping
        });
        assert_eq!(app.state.vm_state, crate::ide::VmExecutionState::Idle);
        app.editor.active_tab_mut().unwrap().content = "EXPORT NEXT() BEGIN RETURN 7; END;".into();
        app.handle_action(IdeAction::RunProgram, &ctx);
        wait(&mut app, |a| {
            a.state.vm_state == crate::ide::VmExecutionState::Terminated
        });
        assert!(
            matches!(app.controller.outcome(),Some(WorkerOutcome::Success{value_str,..}) if value_str=="7")
        );
    }
    #[test]
    fn off_on_preserves_expression_and_ignores_asleep_typing() {
        let (mut app, ctx) = home();
        app.state.native.edit(|e| e.set_text("123"));
        press(&mut app, &ctx, KEY_SHIFT);
        press(&mut app, &ctx, KEY_ON);
        assert!(app.state.native.sleeping);
        press(&mut app, &ctx, KEY_4);
        assert_eq!(app.state.native.home_input, "123");
        press(&mut app, &ctx, KEY_ON);
        assert!(!app.state.native.sleeping);
        assert_eq!(app.state.native.home_input, "123");
    }
    #[test]
    fn fraction_key_cycles_typed_answer_without_replacing_value_or_expression() {
        let (mut app, ctx) = home();
        app.state.native.add_history(
            "5.25".into(),
            "5.25".into(),
            false,
            Some(serde_json::json!({"type":"real","value":"5.25"})),
        );
        for expected in ["21/4", "5+1/4", "5.25"] {
            press(&mut app, &ctx, KEY_ABC);
            let entry = app.state.native.history.back().unwrap();
            assert_eq!(entry.result, expected);
            assert_eq!(entry.expression, "5.25");
            assert_eq!(entry.typed_result.as_ref().unwrap()["value"], "5.25");
            assert!(app.state.native.home_input.is_empty());
        }
        app.state.native.edit(|e| e.insert("2"));
        press(&mut app, &ctx, KEY_ABC);
        assert_eq!(app.state.native.home_input, "(2)/()");
    }
    #[test]
    fn active_app_variable_tracks_home_cas_and_math_entry_context() {
        let (mut app, ctx) = home();
        use crate::ide::native_apps::{GraphApp, GraphKind};
        for (name, kind, expected) in [
            ("Function", GraphKind::Function, "X"),
            ("Parametric", GraphKind::Parametric, "T"),
            ("Polar", GraphKind::Polar, "θ"),
            ("Sequence", GraphKind::Sequence, "N"),
        ] {
            app.native_tools.memory.selected_app = name.into();
            app.native_tools
                .memory
                .graphs
                .insert(name.into(), GraphApp::new(kind));
            app.state.native.edit(|e| e.set_text(""));
            press(&mut app, &ctx, KEY_XTN);
            assert_eq!(app.state.native.home_input, expected);
        }
        app.state.native.cas_mode = true;
        app.state.native.edit(|e| e.set_text(""));
        press(&mut app, &ctx, KEY_XTN);
        assert_eq!(app.state.native.home_input, "x");
    }
    #[test]
    fn desktop_tool_shortcuts_navigate_without_consuming_field_text() {
        let (mut app, ctx) = home();
        app.state.native.edit(|e| e.set_text("home draft"));
        app.native_tools
            .open(crate::ide::native_tools::Page::Settings);
        let _ = ctx.run(
            egui::RawInput {
                events: vec![
                    egui::Event::Text("37".into()),
                    egui::Event::Key {
                        key: egui::Key::F2,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::SHIFT,
                    },
                ],
                ..Default::default()
            },
            |ctx| {
                app.route_native_keyboard(ctx);
                assert_eq!(
                    ctx.input(|i| i.events.clone()),
                    vec![egui::Event::Text("37".into())]
                );
            },
        );
        assert_eq!(
            app.state.native.pending_tool,
            Some(keys::Tool::SymbolicSetup)
        );
        assert_eq!(app.state.native.home_input, "home draft");
    }
    #[test]
    fn virtual_tool_selection_and_history_do_not_edit_hidden_home() {
        let (mut app, ctx) = home();
        app.state.native.edit(|e| e.set_text("123"));
        app.native_tools
            .open(crate::ide::native_tools::Page::Settings);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            press(&mut app, ctx, KEY_SHIFT);
            press(&mut app, ctx, KEY_LEFT);
            assert!(ctx.input(|i| i.events.iter().any(|event| matches!(event, egui::Event::Key {key:egui::Key::ArrowLeft, modifiers, ..} if modifiers.shift))));
            press(&mut app, ctx, KEY_UP);
            press(&mut app, ctx, KEY_ENTER);
        });
        assert_eq!(app.state.native.home_input, "123");
        assert!(!app.controller.worker_alive());
        app.native_tools.open(crate::ide::native_tools::Page::Apps);
        app.native_tools.memory.selected_app = "Function".into();
        let before = app.native_tools.memory.selected_app.clone();
        press(&mut app, &ctx, KEY_DOWN);
        assert_ne!(app.native_tools.memory.selected_app, before);
        press(&mut app, &ctx, KEY_ENTER);
        assert_eq!(
            app.native_tools.page,
            Some(crate::ide::native_tools::Page::Plot)
        );
        assert_eq!(app.native_tools.memory.selected_app, "Linear Explorer");
    }
    #[test]
    fn desktop_shift_tab_function_shortcut_consumes_matching_text_once() {
        let (mut app, ctx) = home();
        let event = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let _ = ctx.run(
            egui::RawInput {
                events: vec![event(egui::Key::Tab, egui::Modifiers::SHIFT)],
                ..Default::default()
            },
            |ctx| app.route_native_keyboard(ctx),
        );
        assert!(app.state.native.shifted);
        let _ = ctx.run(
            egui::RawInput {
                events: vec![
                    event(egui::Key::S, egui::Modifiers::NONE),
                    egui::Event::Text("s".into()),
                ],
                ..Default::default()
            },
            |ctx| app.route_native_keyboard(ctx),
        );
        assert_eq!(app.state.native.home_input, "ASIN()");
        assert!(!app.state.native.shifted);
    }
    #[test]
    fn hexagesimal_key_enters_evaluates_and_toggles_documented_value() {
        let (mut app, ctx) = home();
        for code in [
            KEY_1, KEY_SHIFT, KEY_ABC, KEY_2, KEY_2, KEY_SHIFT, KEY_ABC, KEY_3, KEY_0, KEY_SHIFT,
            KEY_ABC,
        ] {
            press(&mut app, &ctx, code);
        }
        assert_eq!(app.state.native.home_input, "1°22′30″");
        press(&mut app, &ctx, KEY_ENTER);
        wait(&mut app, |app| !app.controller.worker_alive());
        assert_eq!(app.state.native.history.back().unwrap().result, "1.375");
        press(&mut app, &ctx, KEY_SHIFT);
        press(&mut app, &ctx, KEY_ABC);
        assert_eq!(app.state.native.history.back().unwrap().result, "1°22′30″");
        press(&mut app, &ctx, KEY_SHIFT);
        press(&mut app, &ctx, KEY_ABC);
        assert_eq!(app.state.native.history.back().unwrap().result, "1.375");
    }

    #[test]
    fn polar_key_enters_complex_value_and_toggles_existing_answer() {
        let (mut app, ctx) = home();
        app.controller
            .mutate_memory(|memory| {
                memory.settings.angle = ppl_vm::AngleMode::Degrees;
                Ok(())
            })
            .unwrap();
        for code in [KEY_2, KEY_SHIFT, KEY_TIMES, KEY_9, KEY_0, KEY_ENTER] {
            press(&mut app, &ctx, code);
        }
        wait(&mut app, |app| !app.controller.worker_alive());
        assert_eq!(app.state.native.history.back().unwrap().result, "2*i");
        press(&mut app, &ctx, KEY_SHIFT);
        press(&mut app, &ctx, KEY_TIMES);
        assert_eq!(app.state.native.history.back().unwrap().result, "2∠90");
        press(&mut app, &ctx, KEY_SHIFT);
        press(&mut app, &ctx, KEY_TIMES);
        assert_eq!(app.state.native.history.back().unwrap().result, "2*i");
    }
    #[test]
    fn desktop_catalog_arrows_repeat_and_enter_match_virtual_dispatch() {
        use crate::ide::native_tools::Page;
        for page in [
            Page::Apps,
            Page::Notes,
            Page::Programs,
            Page::Variables,
            Page::Lists,
            Page::Matrices,
        ] {
            let (mut app, ctx) = home();
            app.native_tools.open(page);
            app.native_tools.memory.name = "A_ITEM".into();
            app.controller
                .mutate_memory(|memory| {
                    memory
                        .store_note("A_ITEM", "first".into())
                        .map_err(|e| e.to_string())?;
                    memory
                        .store_note("B_ITEM", "second".into())
                        .map_err(|e| e.to_string())?;
                    memory
                        .store_program("A_ITEM", "EXPORT AA() BEGIN RETURN 1; END;".into())
                        .map_err(|e| e.to_string())?;
                    memory
                        .store_program("B_ITEM", "EXPORT BB() BEGIN RETURN 2; END;".into())
                        .map_err(|e| e.to_string())?;
                    for name in ["A_ITEM", "B_ITEM"] {
                        let value = match page {
                            Page::Lists => {
                                ppl_vm::PortableValue::List(vec![ppl_vm::PortableValue::Real(
                                    ppl_math::Real::ONE,
                                )])
                            }
                            Page::Matrices => ppl_vm::PortableValue::Matrix {
                                rows: 1,
                                cols: 1,
                                vector: false,
                                data: vec![ppl_vm::PortableValue::Real(ppl_math::Real::ONE)],
                            },
                            _ => ppl_vm::PortableValue::Real(ppl_math::Real::ONE),
                        };
                        memory
                            .set_variable(name, value)
                            .map_err(|e| e.to_string())?;
                    }
                    Ok(())
                })
                .unwrap();
            let event = |key, repeat| egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat,
                modifiers: egui::Modifiers::NONE,
            };
            let _ = ctx.run(
                egui::RawInput {
                    events: vec![event(egui::Key::ArrowDown, true)],
                    ..Default::default()
                },
                |ctx| app.route_native_keyboard(ctx),
            );
            let selected = if page == Page::Apps {
                app.native_tools.memory.selected_app.clone()
            } else {
                app.native_tools.memory.name.clone()
            };
            press(&mut app, &ctx, KEY_UP);
            press(&mut app, &ctx, KEY_DOWN);
            assert_eq!(
                if page == Page::Apps {
                    &app.native_tools.memory.selected_app
                } else {
                    &app.native_tools.memory.name
                },
                &selected,
                "{page:?}"
            );
            if page == Page::Apps {
                assert_ne!(selected, "Function");
            } else if page != Page::Variables {
                assert_eq!(selected, "B_ITEM", "{page:?}");
            }
            let _ = ctx.run(
                egui::RawInput {
                    events: vec![event(egui::Key::Enter, false)],
                    ..Default::default()
                },
                |ctx| app.route_native_keyboard(ctx),
            );
            if page == Page::Apps {
                assert_eq!(selected, "Linear Explorer");
                assert_eq!(app.native_tools.page, Some(Page::Plot));
            } else if matches!(page, Page::Variables | Page::Lists | Page::Matrices) {
                assert!(app.math_entry.is_some(), "{page:?}");
            } else {
                assert_eq!(
                    ctx.memory(|m| m.focused()),
                    Some(egui::Id::new("native_catalog_content"))
                );
            }
        }
    }
    #[test]
    fn scientific_keypad_products_and_nth_root_evaluate_real_inputs() {
        for (codes, expected) in [
            (vec![KEY_3, KEY_SHIFT, KEY_POW, KEY_8, KEY_ENTER], 2.0),
            (vec![KEY_2, KEY_SIN, KEY_3, KEY_0, KEY_ENTER], 1.0),
            (
                vec![KEY_1, KEY_4, KEY_SHIFT, KEY_SQR, KEY_8, KEY_ENTER],
                14.0 * 8.0_f64.sqrt(),
            ),
        ] {
            let (mut app, ctx) = home();
            app.controller
                .mutate_memory(|memory| {
                    memory.settings.angle = ppl_vm::AngleMode::Degrees;
                    Ok(())
                })
                .unwrap();
            for code in codes {
                press(&mut app, &ctx, code);
            }
            wait(&mut app, |app| !app.controller.worker_alive());
            let entry = app.state.native.history.back().unwrap();
            assert!(!entry.error, "{entry:?}");
            let actual = entry.typed_result.as_ref().unwrap()["value"]
                .as_str()
                .unwrap()
                .parse::<f64>()
                .unwrap();
            assert!((actual - expected).abs() < 1e-9, "{entry:?}");
        }
    }
    #[test]
    fn home_nested_open_program_calls_use_one_worker_and_preserve_ide_selection() {
        let (mut app, ctx) = home();
        app.editor
            .new_tab("DOUBLE.hpprgm", "EXPORT DOUBLE(x) BEGIN RETURN 2*x; END;");
        let active = app.editor.active_tab().unwrap().id;
        app.state
            .native
            .edit(|editor| editor.set_text("2+DOUBLE(DOUBLE(1))"));
        app.submit_native_home(&ctx);
        assert!(matches!(
            app.state.native.operation,
            Some(NativeOperation::Home { .. })
        ));
        wait(&mut app, |app| !app.controller.worker_alive());
        assert_eq!(app.state.native.history.back().unwrap().result, "6");
        assert_eq!(app.editor.active_tab().unwrap().id, active);
        assert_eq!(
            app.controller.session_snapshot().globals["Ans"],
            ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(6))
        );
        assert!(app.native_compile.is_none());
    }
    #[test]
    fn home_nested_installed_calls_survive_tab_close_without_reinitializing() {
        let (mut app, ctx) = home();
        let source =
            "LOCAL count:=10; EXPORT COUNTER(x) BEGIN count:=count+1; RETURN count+x; END;";
        app.controller
            .mutate_memory(|memory| {
                memory
                    .store_program("Saved", source.into())
                    .map_err(|error| error.to_string())
            })
            .unwrap();
        app.editor.new_tab("Saved.hpprgm", source);
        let tab = app.editor.tabs.len() - 1;
        for (expression, expected) in [("COUNTER(COUNTER(0))", "23"), ("COUNTER(0)", "13")] {
            app.state.native.edit(|editor| editor.set_text(expression));
            app.submit_native_home(&ctx);
            wait(&mut app, |app| !app.controller.worker_alive());
            let entry = app.state.native.history.back().unwrap();
            assert!(!entry.error, "{entry:?}");
            assert_eq!(entry.result, expected);
            if expected == "23" {
                app.editor.close_tab(tab);
            }
        }
        assert_eq!(app.state.native.history.len(), 2);
        assert_eq!(
            app.editor.tabs.len(),
            1,
            "Home must not reopen a stored program tab"
        );
    }
    #[test]
    fn home_nested_program_input_uses_actual_dialog_and_continues_expression() {
        let (mut app, ctx) = home();
        app.editor.new_tab("Input.hpprgm","EXPORT DOUBLE(x) BEGIN RETURN 2*x; END; EXPORT ASKVALUE() BEGIN LOCAL v:=0; IF INPUT(v) THEN RETURN v; END; RETURN -1; END;");
        app.state
            .native
            .edit(|editor| editor.set_text("2+DOUBLE(ASKVALUE())"));
        app.submit_native_home(&ctx);
        wait(&mut app, |app| {
            app.controller
                .shared
                .active_dialog
                .lock()
                .unwrap()
                .is_some()
        });
        press(&mut app, &ctx, KEY_2);
        press(&mut app, &ctx, KEY_ENTER);
        wait(&mut app, |app| !app.controller.worker_alive());
        assert_eq!(app.state.native.history.back().unwrap().result, "6");
        assert!(
            app.controller
                .shared
                .active_dialog
                .lock()
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn on_interrupts_nested_home_call_and_next_home_expression_succeeds() {
        let (mut app, ctx) = home();
        app.editor.new_tab(
            "Loop.hpprgm",
            "EXPORT SPIN(x) BEGIN WHILE 1 DO x:=x+1; END; RETURN x; END;",
        );
        app.state
            .native
            .edit(|editor| editor.set_text("2+SPIN(SPIN(0))"));
        app.submit_native_home(&ctx);
        wait(&mut app, |app| app.controller.worker_alive());
        press(&mut app, &ctx, KEY_ON);
        wait(&mut app, |app| {
            !app.controller.worker_alive() && !app.native_stopping
        });
        app.state.native.edit(|editor| editor.set_text("2+3"));
        app.submit_native_home(&ctx);
        wait(&mut app, |app| !app.controller.worker_alive());
        assert_eq!(app.state.native.history.back().unwrap().result, "5");
    }
    #[test]
    fn home_retains_requested_output_without_reopening_old_output_on_math() {
        let (mut app, ctx) = home();
        app.editor.new_tab("Output.hpprgm",r#"EXPORT SAY() BEGIN PRINT("Visible"); RETURN 7; END; EXPORT PAINT() BEGIN RECT_P(0,0,319,239,#00FF00h); RETURN 8; END; EXPORT ASK() BEGIN LOCAL x:=0; INPUT(x); RETURN x; END;"#);
        for expression in ["SAY()", "PAINT()"] {
            app.state.native.edit(|editor| editor.set_text(expression));
            app.submit_native_home(&ctx);
            wait(&mut app, |app| !app.controller.worker_alive());
            assert_eq!(
                app.state.native.view,
                NativeView::ProgramOutput,
                "{expression}"
            );
            press(&mut app, &ctx, KEY_ESC);
            assert_eq!(app.state.native.view, NativeView::Home);
            app.state.native.edit(|editor| editor.set_text("2+3"));
            app.submit_native_home(&ctx);
            wait(&mut app, |app| !app.controller.worker_alive());
            assert_eq!(app.state.native.view, NativeView::Home);
            assert_eq!(app.state.native.history.back().unwrap().result, "5");
        }
        app.state.native.edit(|editor| editor.set_text("ASK()"));
        app.submit_native_home(&ctx);
        wait(&mut app, |app| {
            app.controller
                .shared
                .active_dialog
                .lock()
                .unwrap()
                .is_some()
        });
        press(&mut app, &ctx, KEY_2);
        press(&mut app, &ctx, KEY_ENTER);
        wait(&mut app, |app| !app.controller.worker_alive());
        assert_eq!(
            app.state.native.view,
            NativeView::Home,
            "a temporary INPUT dialog is not retained output"
        );
    }
}
