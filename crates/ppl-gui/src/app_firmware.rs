//! Backend selection, calculator input routing and firmware worker events.
use super::*;
use crate::ide::FirmwareSession;
use ppl_engine::session::{Command, Event, Lifecycle};
use std::sync::atomic::Ordering;
impl IdeDesktopApp {
    /// The current session's helper is running.
    pub(crate) fn firmware_alive(&self) -> bool {
        self.firmware
            .as_ref()
            .is_some_and(|f| f.alive.load(Ordering::Acquire))
    }
    /// A Run, Check or Import sent to the session is unanswered.
    pub(crate) fn firmware_requests_pending(&self) -> bool {
        !self.firmware_pending.is_empty()
            || !self.firmware_runs.is_empty()
            || !self.firmware_imports.is_empty()
    }
    pub(super) fn apply_firmware_environment(&mut self) {
        let config = self.state.firmware.environment.clone();
        if let Err(message) = config.validate() {
            self.firmware_status = message;
            return;
        }
        let Some(firmware) = self.firmware.as_ref() else {
            self.firmware_status = "Start the calculator to apply its environment".into();
            return;
        };
        let Some(request) = self.firmware_environment.next.checked_add(1) else {
            self.firmware_status =
                "Environment request identifiers exhausted; restart the IDE".into();
            return;
        };
        self.firmware_environment.next = request;
        self.firmware_environment.pending = Some(request);
        firmware.send(Command::ConfigureEnvironment { request, config });
        self.firmware_status = "Updating virtual calculator environment…".into();
    }
    /// Starts a session unless one is running.
    pub(super) fn ensure_firmware(&mut self, ctx: &egui::Context) {
        if !self.firmware_alive() {
            self.begin_firmware_session(ctx);
        }
    }
    /// The firmware's own OFF (Shift+ON), as on the keypad: it saves its
    /// state and sleeps; the session and its RAM stay.
    pub(super) fn sleep_firmware(&mut self) {
        for code in [crate::keymap::KEY_SHIFT, crate::keymap::KEY_ON] {
            self.send_calc_transition(code, true);
            self.send_calc_transition(code, false);
        }
    }
    /// Start, or wake a sleeping calculator with ON.
    pub(super) fn power_on_firmware(&mut self, ctx: &egui::Context) {
        if self.state.firmware_session == FirmwareSession::Sleeping && self.firmware_alive() {
            self.send_calc_transition(crate::keymap::KEY_ON, true);
            self.send_calc_transition(crate::keymap::KEY_ON, false);
        } else {
            self.ensure_firmware(ctx);
        }
    }
    /// Replaces any session with a freshly booted one on the same profile.
    pub(super) fn restart_firmware(&mut self, ctx: &egui::Context) {
        self.begin_firmware_session(ctx);
    }
    pub(super) fn save_firmware_live(&mut self) {
        if !self.firmware_alive() || self.firmware_live_save.is_some() {
            return;
        }
        let Some(request) = self.firmware_environment.next.checked_add(1) else {
            self.firmware_status = "Request identifiers exhausted; restart the IDE".into();
            return;
        };
        self.firmware_environment.next = request;
        self.firmware_live_save = Some(request);
        self.firmware_live_save_result = None;
        self.firmware
            .as_ref()
            .unwrap()
            .send(Command::SaveLive { request });
        self.firmware_status = "Saving live calculator state…".into();
    }
    pub(super) fn resume_firmware_live(&mut self, ctx: &egui::Context) {
        if self.firmware_reset.is_some() || self.firmware_live_save.is_some() {
            self.firmware_status =
                "Wait for the current save or reset to finish before resuming".into();
            return;
        }
        self.begin_firmware_session_mode(ctx, true);
    }
    fn begin_firmware_session(&mut self, ctx: &egui::Context) {
        self.begin_firmware_session_mode(ctx, false);
    }
    fn begin_firmware_session_mode(&mut self, ctx: &egui::Context, resume_live: bool) {
        self.firmware_environment.initialized = false;
        self.firmware_environment.pending = None;
        self.state.firmware_clipboard_available = false;
        self.state.firmware_managed_reason = Some("Checking firmware capabilities…".into());
        // The previous helper holds the profile lock; the new session's
        // worker ends it first, so the same calculator profile boots again.
        let previous = self
            .firmware
            .take()
            .or_else(|| self.firmware_retired.take());
        self.reset_firmware_session("The calculator session was replaced");
        self.state.firmware_session = FirmwareSession::Booting;
        self.firmware_status = if resume_live {
            "Resuming live calculator state…"
        } else {
            "Starting local G1 firmware…"
        }
        .into();
        let mut settings = self.state.firmware.clone();
        if resume_live && let Some(profile) = previous.as_ref().and_then(|p| p.profile()) {
            settings.profile = profile.display().to_string();
        }
        self.firmware = Some(if resume_live {
            crate::ide::firmware::FirmwareClient::start_resuming(settings, ctx.clone(), previous)
        } else {
            crate::ide::firmware::FirmwareClient::start(settings, ctx.clone(), previous)
        });
    }
    /// Shuts the calculator down: the firmware's OFF saves its state, then
    /// the helper exits in the background; a later Start waits for it
    /// before booting the same profile.
    pub(super) fn shut_down_firmware(&mut self) {
        self.reset_firmware_session("The calculator was shut down");
        if let Some(f) = self.firmware.take() {
            f.request_shutdown();
            self.firmware_retired = Some(f);
        }
        self.state.firmware_session = FirmwareSession::Off;
        self.firmware_status = "Calculator shut down".into();
    }
    /// Resets the calculator's memory: shut down (the firmware's OFF), then,
    /// once the helper released the profile, move it to a backup. Runs off
    /// the UI thread; `poll_firmware` reports the outcome.
    pub(super) fn reset_firmware_memory(&mut self, ctx: &egui::Context) {
        if self.firmware_reset.is_some() {
            return;
        }
        self.shut_down_firmware();
        let previous = self.firmware_retired.take();
        let settings = self.state.firmware.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            if let Some(previous) = previous {
                previous.shutdown(crate::ide::firmware::SHUTDOWN_LIMIT);
            }
            let _ = tx.send(crate::ide::firmware::reset_profile(&settings));
            ctx.request_repaint();
        });
        self.firmware_reset = Some(rx);
        self.firmware_status = "Resetting calculator memory…".into();
    }
    /// Forgets everything owned by the current session: unanswered
    /// requests (no verdict), held input, the LCD and its counters.
    fn reset_firmware_session(&mut self, message: &str) {
        let pending: Vec<u64> = self.firmware_pending.keys().copied().collect();
        for request in pending {
            self.apply_session_check(request, None, message);
        }
        self.firmware_runs.clear();
        self.firmware_imports.clear();
        self.firmware_live_save = None;
        self.firmware_live_save_result = None;
        self.firmware_resumed = false;
        self.firmware_program_running = false;
        self.firmware_stop_requested = false;
        self.firmware_error = None;
        self.firmware_state = crate::ide::VmExecutionState::Idle;
        if self.state.firmware_backend {
            self.state.vm_state = self.firmware_state.clone();
        }
        self.state.firmware_unsupported.clear();
        self.state.firmware_input_enabled = true;
        self.keypad.release(&mut self.state);
        self.state.clear_held_keys();
        self.last_touch = None;
        self.deferred_touch_end = None;
        self.touch_start_time = None;
        self.touch_sample_time = None;
        self.prev_shift_held = false;
        self.prev_alpha_held = false;
        self.display.texture = None;
        self.display.last_generation = u64::MAX;
    }
    /// Stores the active tab as a new program on the firmware calculator,
    /// named after the tab. The session refuses an existing name.
    pub(super) fn import_to_calculator(&mut self, ctx: &egui::Context) {
        if let Some(reason) = &self.state.firmware_managed_reason {
            self.console.print_error(reason);
            self.state.status_message = reason.clone();
            return;
        }

        let Some(tab) = self.editor.active_tab() else {
            return;
        };
        let name = program_name(&tab.title);
        let source = tab.content.clone();
        self.ensure_firmware(ctx);
        self.firmware_request += 1;
        self.firmware_imports.insert(self.firmware_request);
        self.console
            .print_diagnostic(format!("Importing {name} into the G1 firmware calculator…"));
        if let Some(f) = &self.firmware {
            f.send(Command::Import {
                request: self.firmware_request,
                name,
                source,
            });
        }
    }
    pub(super) fn request_firmware(&mut self, run: bool, ctx: &egui::Context) {
        if let Some(reason) = &self.state.firmware_managed_reason {
            self.console.print_error(reason);
            self.state.status_message = reason.clone();
            return;
        }

        self.ensure_firmware(ctx);
        let Some(tab) = self.editor.active_tab_mut() else {
            return;
        };
        self.firmware_request += 1;
        self.firmware_pending.insert(
            self.firmware_request,
            (tab.id, tab.revision(), tab.title.clone()),
        );
        let command = if run {
            Command::Run {
                request: self.firmware_request,
                source: tab.content.clone(),
                entry: tab.run_entry.clone(),
                arguments: tab.run_args.clone(),
            }
        } else {
            Command::Compile {
                request: self.firmware_request,
                source: tab.content.clone(),
            }
        };
        if let Some(f) = &self.firmware {
            f.send(command);
        }
        // A Check is not a program run: like the native Check, it leaves
        // the VM state alone; its verdict goes to Problems.
        if run {
            self.firmware_runs.insert(self.firmware_request);
            self.firmware_state = crate::ide::VmExecutionState::Running;
            self.state.vm_state = self.firmware_state.clone();
        }
    }
    pub(super) fn poll_firmware(&mut self) {
        if let Some(outcome) = self
            .firmware_reset
            .as_ref()
            .and_then(|rx| rx.try_recv().ok())
        {
            self.firmware_reset = None;
            match outcome {
                Ok(backup) => {
                    let message = format!(
                        "Calculator memory reset; the previous memory is kept at {}",
                        backup.display()
                    );
                    self.console.print_diagnostic(&message);
                    self.firmware_status = message;
                }
                Err(e) => {
                    let message = format!("Calculator memory was not reset: {e}");
                    self.console.print_error(&message);
                    self.firmware_status = message;
                }
            }
        }
        let events = self.firmware.as_ref().map(|f| f.poll()).unwrap_or_default();
        for event in events {
            match event {
                Event::Hello { .. } => {
                    if let Some(profile) = self.firmware.as_ref().and_then(|f| f.profile()) {
                        self.state.firmware.profile = profile.display().to_string();
                    }
                }
                Event::LiveResumed { attachment } => {
                    self.firmware_resumed = true;
                    // The restored calculator owns its environment. A later Ready
                    // event must not replace it with the host's saved defaults.
                    self.firmware_environment.initialized = true;
                    self.firmware_environment.pending = None;
                    self.state.firmware.environment = attachment.environment;
                    let last_request = attachment.next_request.saturating_sub(1);
                    self.firmware_request = self.firmware_request.max(last_request);
                    self.firmware_environment.next =
                        self.firmware_environment.next.max(last_request);
                    self.firmware_runs.extend(
                        attachment
                            .pending
                            .iter()
                            .filter(|p| p.run)
                            .map(|p| p.request),
                    );
                    self.firmware_program_running = attachment
                        .pending
                        .iter()
                        .any(|p| p.run && p.execution_started);
                    self.state.firmware_session = match attachment.lifecycle {
                        Lifecycle::Booting => FirmwareSession::Booting,
                        Lifecycle::Stopping => FirmwareSession::Stopping,
                        Lifecycle::Sleeping => FirmwareSession::Sleeping,
                        Lifecycle::Ready if self.firmware_program_running => {
                            FirmwareSession::Running
                        }
                        Lifecycle::Ready => FirmwareSession::Ready,
                    };
                    self.firmware_status = if attachment.previous_generation {
                        "Calculator resumed from the previous live checkpoint"
                    } else {
                        "Calculator resumed from its live checkpoint"
                    }
                    .into();
                }
                Event::Peripherals { state } => self.state.firmware_peripherals = state,
                Event::Usb {
                    success, message, ..
                } => {
                    if success {
                        self.console.print_diagnostic(&message);
                    } else {
                        self.console.print_error(&message);
                    }
                }
                Event::Environment {
                    request,
                    success,
                    message,
                    config,
                } => {
                    if self.firmware_environment.pending != Some(request) {
                        continue;
                    }
                    self.firmware_environment.pending = None;
                    self.state.firmware.environment = config;
                    if success {
                        self.console.print_diagnostic(&message);
                    } else {
                        self.console.print_error(&message);
                    }
                    self.firmware_status = message;
                }
                Event::Checkpoint { result, .. } => {
                    use ppl_engine::checkpoint::CheckpointResult;
                    let message = match result {
                        CheckpointResult::Saved { .. } => {
                            "Calculator recovery checkpoint saved".into()
                        }
                        CheckpointResult::Deferred { reason } => {
                            format!("Checkpoint deferred: {reason}")
                        }
                        CheckpointResult::Failed { message } => {
                            format!("Checkpoint failed: {message}")
                        }
                    };
                    self.console.print_diagnostic(&message);
                    self.firmware_status = message;
                }
                Event::Lifecycle { state } => {
                    if state == Lifecycle::Ready && !self.firmware_environment.initialized {
                        self.firmware_environment.initialized = true;
                        self.apply_firmware_environment();
                    }
                    self.state.firmware_session = match state {
                        Lifecycle::Booting => FirmwareSession::Booting,
                        Lifecycle::Stopping => FirmwareSession::Stopping,
                        Lifecycle::Sleeping => FirmwareSession::Sleeping,
                        Lifecycle::Ready if self.firmware_program_running => {
                            FirmwareSession::Running
                        }
                        Lifecycle::Ready => FirmwareSession::Ready,
                    };
                    self.firmware_status = match state {
                        Lifecycle::Booting => "Booting local G1 firmware…",
                        Lifecycle::Stopping => "Stopping the firmware program…",
                        Lifecycle::Sleeping => "Calculator off (sleeping); ON wakes it",
                        Lifecycle::Ready if self.firmware_program_running => {
                            "Running in the local G1 firmware"
                        }
                        Lifecycle::Ready => "Calculator ready",
                    }
                    .into();
                }
                Event::Unsupported {
                    id,
                    name,
                    caller_pc,
                } => {
                    self.console.print_error(format!(
                        "Firmware called unsupported OS service {id:#07X} ({}) from \
                         {caller_pc:#010X}; the emulator returned 0",
                        name.as_deref().unwrap_or("unnamed")
                    ));
                    self.state.firmware_unsupported.push((id, name, caller_pc));
                }
                Event::Clipboard { text, .. } => {
                    self.controller.shared.publish_clipboard(text);
                    self.firmware_status = "Copied firmware selection to desktop clipboard".into();
                }
                Event::Capabilities {
                    managed_programs,
                    clipboard_text,
                    reason,
                } => {
                    self.state.firmware_clipboard_available = clipboard_text;
                    self.state.firmware_managed_reason = if managed_programs {
                        None
                    } else {
                        Some(
                            reason
                                .unwrap_or_else(|| "Managed firmware execution unavailable".into()),
                        )
                    };
                }
                Event::Phase { phase } => {
                    self.state.firmware_input_enabled = phase != "compiling";
                    self.firmware_program_running = phase == "running";
                    match (&self.state.firmware_session, phase.as_str()) {
                        (FirmwareSession::Ready | FirmwareSession::Booting, "running") => {
                            self.state.firmware_session = FirmwareSession::Running
                        }
                        (FirmwareSession::Running, "ready") => {
                            self.state.firmware_session = FirmwareSession::Ready
                        }
                        _ => {}
                    }
                    self.firmware_status = match phase.as_str() {
                        "running" => "Running in the local G1 firmware",
                        "compiling" => "Compiling in the live G1 session…",
                        _ => "Calculator ready",
                    }
                    .into();
                }
                Event::Status { message } => {
                    self.firmware_status = message;
                }
                Event::Result {
                    request,
                    success,
                    message,
                    check,
                } => {
                    if self.firmware_live_save == Some(request) {
                        self.firmware_live_save = None;
                        self.firmware_live_save_result = Some(if success {
                            Ok(())
                        } else {
                            Err(message.clone())
                        });
                        if success {
                            self.console.print_diagnostic(&message);
                        } else {
                            self.console.print_error(&message);
                        }
                        self.firmware_status = message;
                        continue;
                    }
                    self.apply_session_check(request, check, &message);
                    self.firmware_imports.remove(&request);
                    self.firmware_program_running = false;
                    if self.state.firmware_session == FirmwareSession::Running {
                        self.state.firmware_session = FirmwareSession::Ready;
                    }
                    let was_run = self.firmware_runs.remove(&request);
                    let stopped =
                        was_run && !success && std::mem::take(&mut self.firmware_stop_requested);
                    if was_run && self.firmware_runs.is_empty() {
                        self.firmware_state = if success {
                            crate::ide::VmExecutionState::Terminated
                        } else if stopped {
                            crate::ide::VmExecutionState::Idle
                        } else {
                            crate::ide::VmExecutionState::Error(message.clone())
                        };
                    }
                    if stopped {
                        self.console
                            .print_diagnostic("--- Firmware program stopped by user ---");
                    }
                    if success || stopped {
                        self.console.print_diagnostic(&message);
                    } else {
                        self.console.print_error(&message);
                    }
                    self.firmware_status = message;
                }
                Event::Error { message } => {
                    self.firmware_error = Some(message.clone());
                    self.firmware_state = crate::ide::VmExecutionState::Error(message.clone());
                    self.firmware_status = message.clone();
                    self.console.print_error(message);
                }
                Event::Closed { message } => {
                    if self.firmware_live_save.take().is_some() {
                        self.firmware_live_save_result = Some(Err(message.clone()));
                    }
                    // Requests the closed session never answered reach no verdict.
                    let pending: Vec<u64> = self.firmware_pending.keys().copied().collect();
                    for request in pending {
                        self.apply_session_check(request, None, &message);
                    }
                    self.firmware_runs.clear();
                    self.firmware_imports.clear();
                    self.firmware_program_running = false;
                    let unavailable = self
                        .firmware
                        .as_ref()
                        .and_then(|f| f.unavailable.lock().unwrap().clone());
                    self.state.firmware_session = match unavailable {
                        Some(reason) => FirmwareSession::Unavailable(reason),
                        None => FirmwareSession::Failed(
                            self.firmware_error
                                .clone()
                                .unwrap_or_else(|| message.clone()),
                        ),
                    };
                    if !matches!(self.firmware_state, crate::ide::VmExecutionState::Error(_)) {
                        self.firmware_status = message;
                        self.firmware_state = crate::ide::VmExecutionState::Idle;
                    }
                }
                Event::Inspection { .. }
                | Event::Execution { .. }
                | Event::ExecutionStarted { .. } => {}
            }
        }
        if self.state.firmware_backend {
            self.state.vm_state = self.firmware_state.clone();
        }
    }
    /// Applies the firmware compiler's verdict from a session result to the
    /// tab revision that request sent. A result without a verdict never
    /// replaces a verdict already held for that revision.
    pub(super) fn apply_session_check(
        &mut self,
        request: u64,
        check: Option<ppl_engine::session::CheckVerdict>,
        message: &str,
    ) {
        let Some((id, revision, title)) = self.firmware_pending.remove(&request) else {
            return;
        };
        match check {
            Some(verdict) => {
                let sent = self
                    .editor
                    .index_of(id)
                    .map(|i| ppl_engine::source::hash(self.editor.tabs[i].content.as_bytes()));
                if sent.as_deref() != Some(verdict.source_sha256.as_str()) {
                    return;
                }
                self.record_firmware_analysis(id, revision, &title, &verdict.diagnostics, None);
            }
            None => {
                let held = self.editor.index_of(id).is_some_and(|i| {
                    self.editor.tabs[i]
                        .firmware
                        .as_ref()
                        .is_some_and(|f| f.revision == revision && f.failure.is_none())
                });
                if !held {
                    self.record_firmware_analysis(id, revision, &title, &[], Some(message.into()));
                }
            }
        }
    }
    /// The live firmware has entered the program body and not returned.
    pub(crate) fn firmware_running(&self) -> bool {
        self.state.firmware_backend && self.firmware_program_running
    }
    pub(crate) fn send_calc_transition(&mut self, code: i64, down: bool) {
        // ON breaks a running program on the calculator itself: the IDE run
        // then ends as stopped by the user, not as an error.
        if self.state.firmware_backend
            && code == crate::keymap::KEY_ON
            && down
            && self.firmware_program_running
        {
            self.firmware_stop_requested = true;
        }
        if self.state.firmware_backend {
            if let (Some(f), Some(mapping)) = (
                &self.firmware,
                u8::try_from(code)
                    .ok()
                    .and_then(crate::keymap::lookup_by_physical),
            ) {
                f.send(Command::Key {
                    code: mapping.device_code,
                    down,
                });
            }
        } else {
            self.controller.shared.set_key_down(code, down);
        }
    }
    pub(super) fn forward_calc_keys(&mut self, ctx: &egui::Context) {
        if !self.state.firmware_backend {
            self.route_native_keyboard(ctx);
            return;
        }
        if self.state.input_owner == crate::ide::native::InputOwner::IdeModal {
            return;
        }
        if !self.state.calc_input_focus {
            return;
        }
        if !ctx.input(|i| i.focused) {
            self.keypad.release(&mut self.state);
            self.controller.shared.clear_held_keys();
            self.controller.shared.set_touch(None);
            self.state.clear_held_keys();
            if let Some(f) = &self.firmware {
                f.send(Command::ReleaseAll);
                if let Some(t) = self.last_touch.take() {
                    f.send(Command::Touch {
                        status: 8,
                        x: t.x as u16,
                        y: t.y as u16,
                    });
                }
            }
            self.last_touch = None;
            self.deferred_touch_end = None;
            self.touch_start_time = None;
            self.touch_sample_time = None;
            self.prev_shift_held = false;
            self.prev_alpha_held = false;
            return;
        }

        // Desktop modifiers belong to desktop text. Sending host Shift/Ctrl
        // as firmware Shift/Alpha as well would apply them a second time.
        // The virtual keypad and Tab (Alpha) retain physical-key semantics.
        let mut events: Vec<egui::Event> = Vec::new();
        for event in ctx.input(|i| i.events.clone()) {
            match (events.last_mut(), event) {
                (Some(egui::Event::Text(previous)), egui::Event::Text(text)) => {
                    previous.push_str(&text)
                }
                (_, event) => events.push(event),
            }
        }
        let has_text = events
            .iter()
            .any(|e| matches!(e, egui::Event::Text(_) | egui::Event::Paste(_)));
        for event in events {
            let paste = matches!(&event, egui::Event::Paste(_));
            match event {
                egui::Event::Text(text) | egui::Event::Paste(text) => {
                    if let Err(error) = ppl_engine::session::validate_desktop_text(&text) {
                        self.firmware_status = error.into();
                        self.console.print_diagnostic(&self.firmware_status);
                        continue;
                    }
                    if let Some(f) = &self.firmware {
                        // One bounded transport packet, prevalidated atomically
                        // by the session. Never flood the channel with key pairs.
                        f.send(Command::Text { text, paste });
                    }
                }
                egui::Event::Key {
                    key,
                    pressed,
                    repeat,
                    modifiers,
                    ..
                } if !repeat && (!pressed || !modifiers.command) => {
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
                    // Printable key-up events are not calculator actions either.
                    if printable && (has_text || !pressed) {
                        continue;
                    }
                    if let Some(code) = egui_key_to_hp(key) {
                        if modifiers.shift && pressed && !printable && key != egui::Key::Tab {
                            // Intentional calculator shortcuts (Shift+Home,
                            // Shift+arrows, etc.) still use physical Shift.
                            self.send_calc_transition(crate::keymap::KEY_SHIFT, true);
                            self.send_calc_transition(crate::keymap::KEY_SHIFT, false);
                        }
                        if key == egui::Key::Tab {
                            // Release the key chosen on key-down even if the
                            // desktop Shift key was released before Tab.
                            let code = if (pressed && modifiers.shift)
                                || (!pressed && self.prev_shift_held)
                            {
                                crate::keymap::KEY_SHIFT
                            } else {
                                crate::keymap::KEY_ALPHA
                            };
                            self.prev_shift_held = pressed && code == crate::keymap::KEY_SHIFT;
                            self.state.set_key_down(code, pressed);
                            self.send_calc_transition(code, pressed);
                        } else {
                            self.state.set_key_down(code, pressed);
                            self.send_calc_transition(code, pressed);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    pub(super) fn forward_touch(&mut self, ctx: &egui::Context) {
        const MIN_TAP_DURATION: std::time::Duration = std::time::Duration::from_millis(300);
        let now = std::time::Instant::now();
        const SAMPLE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(16);

        // A stationary finger still produces MOVE samples on the real touch
        // controller. Firmware menu buttons debounce those samples; a Down/Up
        // pair alone may highlight a button without activating it. Keep the
        // stream alive during both a held pointer and our minimum tap interval.
        if self.state.firmware_backend {
            if let Some(t) = self.last_touch {
                let before_release = self.deferred_touch_end.is_none_or(|(_, end)| now < end);
                if before_release
                    && self
                        .touch_sample_time
                        .is_none_or(|last| now.duration_since(last) >= SAMPLE_INTERVAL)
                {
                    if let Some(f) = &self.firmware {
                        f.send(Command::Touch {
                            status: 2,
                            x: t.x as u16,
                            y: t.y as u16,
                        });
                    }
                    self.touch_sample_time = Some(now);
                }
                ctx.request_repaint_after(SAMPLE_INTERVAL);
            }
        }

        // Flush expired deferred touch-end
        if let Some((t, deadline)) = self.deferred_touch_end {
            if now >= deadline {
                if self.state.firmware_backend {
                    if let Some(f) = &self.firmware {
                        f.send(Command::Touch {
                            status: 8,
                            x: t.x as u16,
                            y: t.y as u16,
                        });
                    }
                }
                self.deferred_touch_end = None;
                self.last_touch = None;
                self.touch_start_time = None;
                self.touch_sample_time = None;
            } else {
                ctx.request_repaint_after(deadline - now);
            }
        }

        let touch = if ctx.input(|i| i.pointer.primary_down()) {
            ctx.input(|i| i.pointer.interact_pos())
                .and_then(|pos| self.display.project_pointer_pos(pos))
        } else {
            None
        };

        if self.state.firmware_backend {
            if let Some(t) = touch {
                // If there was a pending deferred release from a previous tap, flush it before starting new touch
                if let Some((prev_t, _)) = self.deferred_touch_end.take() {
                    if let Some(f) = &self.firmware {
                        f.send(Command::Touch {
                            status: 8,
                            x: prev_t.x as u16,
                            y: prev_t.y as u16,
                        });
                    }
                    self.last_touch = None;
                }

                if self.last_touch.is_none() {
                    // New touch down
                    self.touch_start_time = Some(now);
                    self.touch_sample_time = Some(now);
                    ctx.request_repaint_after(SAMPLE_INTERVAL);
                    if let Some(f) = &self.firmware {
                        f.send(Command::Touch {
                            status: 1,
                            x: t.x as u16,
                            y: t.y as u16,
                        });
                    }
                    self.last_touch = Some(t);
                } else if self.last_touch != Some(t) {
                    // Touch moved / dragged
                    self.touch_sample_time = Some(now);
                    if let Some(f) = &self.firmware {
                        f.send(Command::Touch {
                            status: 2,
                            x: t.x as u16,
                            y: t.y as u16,
                        });
                    }
                    self.last_touch = Some(t);
                }
            } else if let Some(t) = self.last_touch {
                if self.deferred_touch_end.is_none() {
                    let elapsed = self
                        .touch_start_time
                        .map_or(MIN_TAP_DURATION, |st| now.duration_since(st));
                    if elapsed < MIN_TAP_DURATION {
                        let deadline = self.touch_start_time.unwrap_or(now) + MIN_TAP_DURATION;
                        self.deferred_touch_end = Some((t, deadline));
                        ctx.request_repaint_after(deadline - now);
                    } else {
                        if let Some(f) = &self.firmware {
                            f.send(Command::Touch {
                                status: 8,
                                x: t.x as u16,
                                y: t.y as u16,
                            });
                        }
                        self.last_touch = None;
                        self.touch_start_time = None;
                        self.touch_sample_time = None;
                    }
                }
            }
        } else {
            self.controller.shared.set_touch(touch);
            self.last_touch = touch;
        }
    }
}
/// The calculator program name for a tab title: its file stem.
pub(crate) fn program_name(title: &str) -> String {
    let title = title.trim_end_matches('*').trim();
    title.split('.').next().unwrap_or(title).to_string()
}
#[cfg(test)]
mod tests {
    #[test]
    fn program_names_are_tab_file_stems() {
        assert_eq!(super::program_name("M12HELLO.hpprgm"), "M12HELLO");
        assert_eq!(super::program_name("DEMO.ppl"), "DEMO");
        assert_eq!(super::program_name("SQUARE"), "SQUARE");
    }
}
#[cfg(test)]
mod session_tests {
    use super::*;
    use crate::ide::firmware::{FirmwareClient, Frame};
    fn frame(generation: u64, pixel: u32) -> Frame {
        Frame {
            generation,
            micros: 0,
            pixels: vec![pixel; 320 * 240].into(),
        }
    }
    fn firmware_app() -> IdeDesktopApp {
        let mut app = IdeDesktopApp::default();
        // Never start the developer's configured firmware from a unit test.
        app.state.firmware.firmware = "/nonexistent/ppl-test/armfir.elf".into();
        app.state.firmware_backend = true;
        app
    }

    #[test]
    fn desktop_shift_and_clipboard_are_not_firmware_modifiers() {
        let ctx = egui::Context::default();
        let mut app = firmware_app();
        let (client, _, commands) = FirmwareClient::for_test(None);
        app.firmware = Some(client);
        app.state.calc_input_focus = true;
        for (modifiers, event, wanted) in [
            (egui::Modifiers::SHIFT, egui::Event::Text("A".into()), "A"),
            (
                egui::Modifiers::CTRL,
                egui::Event::Paste("(a+2)".into()),
                "(a+2)",
            ),
        ] {
            let _ = ctx.run(
                egui::RawInput {
                    focused: true,
                    modifiers,
                    events: vec![event],
                    ..Default::default()
                },
                |ctx| app.forward_calc_keys(ctx),
            );
            let sent: Vec<_> = commands.try_iter().collect();
            assert!(
                matches!(sent.as_slice(), [Command::Text { text, .. }] if text == wanted),
                "{sent:?}"
            );
        }
    }
    #[test]
    fn shift_tab_release_follows_its_press_after_host_shift_is_released() {
        let ctx = egui::Context::default();
        let mut app = firmware_app();
        let (client, _, commands) = FirmwareClient::for_test(None);
        app.firmware = Some(client);
        app.state.calc_input_focus = true;
        for (pressed, modifiers) in [
            (true, egui::Modifiers::SHIFT),
            (false, egui::Modifiers::NONE),
        ] {
            let _ = ctx.run(
                egui::RawInput {
                    focused: true,
                    events: vec![egui::Event::Key {
                        key: egui::Key::Tab,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers,
                    }],
                    ..Default::default()
                },
                |ctx| app.forward_calc_keys(ctx),
            );
        }
        let keys: Vec<_> = commands
            .try_iter()
            .filter_map(|c| match c {
                Command::Key { code, down } => Some((code, down)),
                _ => None,
            })
            .collect();
        assert_eq!(keys, vec![(0x8b, true), (0x8b, false)]);
        assert!(!app.state.keys_down.contains(&crate::keymap::KEY_SHIFT));
    }

    #[test]
    fn a_large_text_batch_is_one_atomic_command_not_a_channel_flood() {
        let ctx = egui::Context::default();
        let mut app = firmware_app();
        let (client, _, commands) = FirmwareClient::for_test(None);
        app.firmware = Some(client);
        app.state.calc_input_focus = true;
        let _ = ctx.run(
            egui::RawInput {
                focused: true,
                events: (0..500).map(|_| egui::Event::Text("a".into())).collect(),
                ..Default::default()
            },
            |ctx| app.forward_calc_keys(ctx),
        );
        let sent: Vec<_> = commands.try_iter().collect();
        assert!(matches!(sent.as_slice(), [Command::Text { text, .. }] if text.len() == 500));
    }

    #[test]
    fn desktop_text_uses_the_firmware_utf16_budget_and_refuses_whole_invalid_packets() {
        let ctx = egui::Context::default();
        let mut app = firmware_app();
        let (client, _, commands) = FirmwareClient::for_test(None);
        app.firmware = Some(client);
        app.state.calc_input_focus = true;
        let allowed = "λ😀\n".repeat(2000);
        for text in [
            allowed.clone(),
            "before\0after".into(),
            "😀".repeat(ppl_engine::session::MAX_TEXT_UTF16_UNITS / 2 + 1),
        ] {
            let _ = ctx.run(
                egui::RawInput {
                    focused: true,
                    events: vec![egui::Event::Paste(text.clone())],
                    ..Default::default()
                },
                |ctx| app.forward_calc_keys(ctx),
            );
            let sent = commands.try_iter().collect::<Vec<_>>();
            if text == allowed {
                assert!(
                    matches!(sent.as_slice(), [Command::Text { text, paste: true }] if text == &allowed)
                );
            } else {
                assert!(sent.is_empty(), "Invalid text must not be partly forwarded");
                assert!(app.firmware_status.contains("no text was queued"));
            }
        }
    }

    #[test]
    fn session_states_follow_the_live_session() {
        let mut app = firmware_app();
        let (client, events, _commands) = FirmwareClient::for_test(None);
        app.firmware = Some(client);
        app.state.firmware_session = FirmwareSession::Booting;
        let mut expect = |event: Event, state: FirmwareSession| {
            events.send(event).unwrap();
            app.poll_firmware();
            assert_eq!(app.state.firmware_session, state);
            app.state.firmware_unsupported.len()
        };
        expect(
            Event::Lifecycle {
                state: Lifecycle::Ready,
            },
            FirmwareSession::Ready,
        );
        expect(
            Event::Phase {
                phase: "running".into(),
            },
            FirmwareSession::Running,
        );
        expect(
            Event::Lifecycle {
                state: Lifecycle::Stopping,
            },
            FirmwareSession::Stopping,
        );
        // A cancelled result does not end Stopping; the session's Ready does.
        expect(
            Event::Result {
                request: 1,
                success: false,
                message: "Firmware operation cancelled".into(),
                check: None,
            },
            FirmwareSession::Stopping,
        );
        expect(
            Event::Lifecycle {
                state: Lifecycle::Ready,
            },
            FirmwareSession::Ready,
        );
        let unsupported = expect(
            Event::Unsupported {
                id: 0x10042,
                name: None,
                caller_pc: 0x3010_0000,
            },
            FirmwareSession::Ready,
        );
        assert_eq!(unsupported, 1);
        expect(
            Event::Error {
                message: "helper crashed".into(),
            },
            FirmwareSession::Ready,
        );
        expect(
            Event::Closed {
                message: "Local firmware session closed".into(),
            },
            FirmwareSession::Failed("helper crashed".into()),
        );
    }

    #[test]
    fn a_session_that_cannot_start_is_unavailable_not_failed() {
        let mut app = firmware_app();
        let (client, events, _commands) = FirmwareClient::for_test(None);
        *client.unavailable.lock().unwrap() = Some("assets missing".into());
        app.firmware = Some(client);
        events
            .send(Event::Closed {
                message: "closed".into(),
            })
            .unwrap();
        app.poll_firmware();
        assert_eq!(
            app.state.firmware_session,
            FirmwareSession::Unavailable("assets missing".into())
        );
    }

    #[test]
    fn a_replacement_session_shows_its_own_frames_from_generation_one() {
        let ctx = egui::Context::default();
        let mut app = firmware_app();
        let (old, _, _) = FirmwareClient::for_test(Some(frame(500, 0x00ff0000)));
        app.firmware = Some(old);
        app.update_display_texture(&ctx);
        assert_eq!(app.display.last_generation, 500);
        let (new, _, _) = FirmwareClient::for_test(Some(frame(1, 0x0000ff00)));
        app.firmware = Some(new);
        app.update_display_texture(&ctx);
        assert_eq!(app.display.last_generation, 1);
        // Switched off: no stale frame stays on the LCD.
        app.firmware = None;
        app.update_display_texture(&ctx);
        assert!(app.display.texture.is_none());
    }

    #[test]
    fn restart_forgets_the_old_sessions_requests_and_reuses_its_profile_lock() {
        let ctx = egui::Context::default();
        let mut app = firmware_app();
        let (client, _, _) = FirmwareClient::for_test(None);
        let old_id = client.session_id;
        app.firmware = Some(client);
        app.firmware_runs.insert(7);
        app.firmware_imports.insert(8);
        app.firmware_program_running = true;
        app.state.firmware_unsupported.push((1, None, 2));
        app.shut_down_firmware();
        assert_eq!(app.state.firmware_session, FirmwareSession::Off);
        assert!(app.firmware.is_none() && app.firmware_retired.is_some());
        assert!(app.firmware_runs.is_empty() && app.firmware_imports.is_empty());
        assert!(!app.firmware_program_running);
        assert!(app.state.firmware_unsupported.is_empty());
        // Start hands the switched-off session to the new one's worker.
        app.handle_action(IdeAction::FirmwareStart, &ctx);
        assert!(app.firmware_retired.is_none());
        let new = app.firmware.as_ref().unwrap();
        assert_ne!(new.session_id, old_id);
        assert_eq!(app.state.firmware_session, FirmwareSession::Booting);
        // The nonexistent test firmware never starts a helper.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.firmware_alive() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        app.poll_firmware();
        assert!(
            matches!(app.state.firmware_session, FirmwareSession::Unavailable(_)),
            "{:?}",
            app.state.firmware_session
        );
    }

    #[test]
    fn desktop_auto_repeat_never_reaches_the_firmware() {
        let ctx = egui::Context::default();
        let mut app = firmware_app();
        let (client, _, commands) = FirmwareClient::for_test(None);
        app.firmware = Some(client);
        app.state.calc_input_focus = true;
        let key = |repeat, pressed| egui::RawInput {
            focused: true,
            events: vec![egui::Event::Key {
                key: egui::Key::ArrowUp,
                physical_key: None,
                pressed,
                repeat,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        };
        let mut sent = |input| {
            let _ = ctx.run(input, |ctx| app.forward_calc_keys(ctx));
            commands
                .try_iter()
                .filter_map(|c| match c {
                    Command::Key { code, down } => Some((code, down)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let up = crate::keymap::lookup_by_physical(
            crate::keymap::egui_key_to_hp(egui::Key::ArrowUp).unwrap() as u8,
        )
        .unwrap()
        .device_code;
        assert_eq!(sent(key(false, true)), vec![(up, true)]);
        assert_eq!(sent(key(true, true)), vec![]);
        assert_eq!(sent(key(true, true)), vec![]);
        assert_eq!(sent(key(false, false)), vec![(up, false)]);
    }

    #[test]
    fn repeated_navigation_does_not_drop_text_and_equality_is_not_enter() {
        let ctx = egui::Context::default();
        let mut app = firmware_app();
        let (client, _, commands) = FirmwareClient::for_test(None);
        app.firmware = Some(client);
        app.state.calc_input_focus = true;
        // egui determines repeat from its held-key state, not the incoming flag.
        let _ = ctx.run(
            egui::RawInput {
                focused: true,
                events: vec![egui::Event::Key {
                    key: egui::Key::ArrowUp,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ctx| app.forward_calc_keys(ctx),
        );
        commands.try_iter().for_each(drop);
        let _ = ctx.run(
            egui::RawInput {
                focused: true,
                events: vec![
                    egui::Event::Key {
                        key: egui::Key::ArrowUp,
                        physical_key: None,
                        pressed: true,
                        repeat: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                    egui::Event::Text("=".into()),
                ],
                ..Default::default()
            },
            |ctx| app.forward_calc_keys(ctx),
        );
        let sent: Vec<_> = commands.try_iter().collect();
        assert!(
            matches!(sent.as_slice(), [Command::Text { text, .. }] if text == "="),
            "{sent:?}"
        );
    }

    #[test]
    fn calculator_keys_reach_only_the_selected_engine() {
        let ctx = egui::Context::default();
        let mut app = firmware_app();
        let (client, _, commands) = FirmwareClient::for_test(None);
        app.firmware = Some(client);
        let enter = crate::keymap::KEY_ENTER;
        // G1 Firmware: the key goes to the session, not the native VM.
        app.send_calc_transition(enter, true);
        app.send_calc_transition(enter, false);
        assert_eq!(
            commands
                .try_iter()
                .filter(|c| matches!(c, Command::Key { .. }))
                .count(),
            2
        );
        assert!(!app.controller.shared.is_key_down(enter));
        // Native: held firmware keys are released, nothing more is sent.
        app.send_calc_transition(enter, true);
        app.handle_action(IdeAction::SelectBackend(false), &ctx);
        let sent: Vec<_> = commands.try_iter().collect();
        assert!(matches!(sent.last(), Some(Command::ReleaseAll)), "{sent:?}");
        app.send_calc_transition(enter, true);
        assert!(app.controller.shared.is_key_down(enter));
        app.send_calc_transition(enter, false);
        assert_eq!(commands.try_iter().count(), 0);
        // The firmware session itself is kept for the next switch back.
        assert!(app.firmware.is_some());
    }
    #[test]
    fn committed_firmware_text_reaches_the_platform_clipboard_queue_exactly() {
        let mut app = firmware_app();
        let (client, events, _) = FirmwareClient::for_test(None);
        app.firmware = Some(client);
        events
            .send(Event::Capabilities {
                managed_programs: true,
                clipboard_text: true,
                reason: None,
            })
            .unwrap();
        app.poll_firmware();
        assert!(app.state.firmware_clipboard_available);
        assert!(app.controller.shared.take_dialog_clipboard().is_none());
        for (generation, text) in [(1, "π×ⅈ\r\nEXPORT T()"), (2, ""), (3, "same"), (4, "same")]
        {
            events
                .send(Event::Clipboard {
                    generation,
                    text: text.into(),
                })
                .unwrap();
            app.poll_firmware();
            assert_eq!(
                app.controller.shared.take_dialog_clipboard().as_deref(),
                Some(text)
            );
            assert!(app.controller.shared.take_dialog_clipboard().is_none());
        }
        events
            .send(Event::Capabilities {
                managed_programs: true,
                clipboard_text: false,
                reason: None,
            })
            .unwrap();
        events
            .send(Event::Status {
                message: "Unsupported clipboard record".into(),
            })
            .unwrap();
        app.poll_firmware();
        assert!(!app.state.firmware_clipboard_available);
        assert!(app.controller.shared.take_dialog_clipboard().is_none());
    }

    #[test]
    fn stationary_lcd_taps_stream_samples_until_the_minimum_release() {
        use std::time::{Duration, Instant};
        let ctx = egui::Context::default();
        let mut app = firmware_app();
        let (client, _, commands) = FirmwareClient::for_test(None);
        app.firmware = Some(client);
        app.display.last_display_rect =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(320.0, 240.0));
        let pos = egui::pos2(294.5, 230.5);
        let frame = |app: &mut IdeDesktopApp, events| {
            let _ = ctx.run(
                egui::RawInput {
                    focused: true,
                    events,
                    ..Default::default()
                },
                |ctx| app.forward_touch(ctx),
            );
        };
        frame(
            &mut app,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        assert!(matches!(
            commands.try_recv(),
            Ok(Command::Touch {
                status: 1,
                x: 294,
                y: 230
            })
        ));
        app.touch_sample_time = Some(Instant::now() - Duration::from_millis(20));
        frame(&mut app, vec![]);
        assert!(matches!(
            commands.try_recv(),
            Ok(Command::Touch {
                status: 2,
                x: 294,
                y: 230
            })
        ));
        frame(
            &mut app,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(app.deferred_touch_end.is_some());
        assert!(commands.try_recv().is_err());
        app.touch_sample_time = Some(Instant::now() - Duration::from_millis(20));
        frame(&mut app, vec![]);
        assert!(matches!(
            commands.try_recv(),
            Ok(Command::Touch { status: 2, .. })
        ));
        let (touch, _) = app.deferred_touch_end.unwrap();
        app.deferred_touch_end = Some((touch, Instant::now() - Duration::from_millis(1)));
        frame(&mut app, vec![]);
        assert!(matches!(
            commands.try_recv(),
            Ok(Command::Touch {
                status: 8,
                x: 294,
                y: 230
            })
        ));
        frame(&mut app, vec![]);
        assert!(commands.try_recv().is_err());
        assert!(app.last_touch.is_none());
    }
    #[test]
    fn environment_replies_are_correlated_without_terminating_running_programs() {
        let mut app = firmware_app();
        let (client, events, commands) = FirmwareClient::for_test(None);
        app.firmware = Some(client);
        app.firmware_program_running = true;
        app.state.firmware_session = FirmwareSession::Running;
        app.firmware_request = 7;
        app.firmware_runs.insert(7);
        app.state.firmware.environment.battery.adc = 800;
        app.apply_firmware_environment();
        let command = commands.try_recv().unwrap();
        let Command::ConfigureEnvironment { request, config } = command else {
            panic!("expected actual configuration command")
        };
        assert_ne!(request, 7);
        assert_eq!(config.battery.adc, 800);
        let previous = ppl_engine::environment::EnvironmentConfig::default();
        events
            .send(Event::Environment {
                request: request - 1,
                success: true,
                message: "stale".into(),
                config: previous,
            })
            .unwrap();
        app.poll_firmware();
        assert_eq!(app.state.firmware.environment, config);
        events
            .send(Event::Environment {
                request,
                success: true,
                message: "applied".into(),
                config: config.clone(),
            })
            .unwrap();
        app.poll_firmware();
        assert!(app.firmware_program_running);
        assert_eq!(app.firmware_request, 7);
        assert!(app.firmware_runs.contains(&7));
        assert_eq!(app.state.firmware_session, FirmwareSession::Running);
        assert!(app.firmware_environment.pending.is_none());
    }
    #[test]
    fn live_save_result_is_independent_of_a_waiting_run_and_duplicate_clicks() {
        let mut app = firmware_app();
        let (client, events, commands) = FirmwareClient::for_test(None);
        app.firmware = Some(client);
        app.firmware_program_running = true;
        app.state.firmware_session = FirmwareSession::Running;
        app.firmware_request = 7;
        app.firmware_runs.insert(7);
        app.save_firmware_live();
        let Command::SaveLive { request } = commands.try_recv().unwrap() else {
            panic!("expected SaveLive");
        };
        assert_ne!(request, 7);
        assert_eq!(app.firmware_request, 7);
        app.save_firmware_live();
        assert!(commands.try_recv().is_err());
        events
            .send(Event::Result {
                request,
                success: true,
                message: "Live state saved".into(),
                check: None,
            })
            .unwrap();
        app.poll_firmware();
        assert!(app.firmware_program_running);
        assert_eq!(app.state.firmware_session, FirmwareSession::Running);
        assert!(app.firmware_runs.contains(&7));
        assert_eq!(app.firmware_live_save_result, Some(Ok(())));
        assert!(app.firmware_live_save.is_none());
        app.save_firmware_live();
        let Command::SaveLive { request } = commands.try_recv().unwrap() else {
            panic!();
        };
        events
            .send(Event::Result {
                request,
                success: false,
                message: "Unsafe boundary".into(),
                check: None,
            })
            .unwrap();
        app.poll_firmware();
        assert_eq!(
            app.firmware_live_save_result,
            Some(Err("Unsafe boundary".into()))
        );
        assert!(app.firmware_program_running && app.firmware_runs.contains(&7));
        app.save_firmware_live();
        events
            .send(Event::Closed {
                message: "Helper ended".into(),
            })
            .unwrap();
        app.poll_firmware();
        assert_eq!(
            app.firmware_live_save_result,
            Some(Err("Helper ended".into()))
        );
        assert!(app.firmware_live_save.is_none());
    }
    #[test]
    fn restored_environment_and_pending_run_survive_ready_and_complete_without_tab_verdict() {
        use ppl_engine::session::{ResumeAttachment, ResumeRequest};
        let mut app = firmware_app();
        let (client, events, commands) = FirmwareClient::for_test(None);
        app.firmware = Some(client);
        let mut environment = ppl_engine::environment::EnvironmentConfig::default();
        environment.battery.adc = 800;
        events
            .send(Event::LiveResumed {
                attachment: ResumeAttachment {
                    next_request: (1 << 63) + 5,
                    lifecycle: Lifecycle::Ready,
                    environment: environment.clone(),
                    previous_generation: false,
                    pending: vec![
                        ResumeRequest {
                            request: 7,
                            run: true,
                            source: Some("run source".into()),
                            program: None,
                            entry: None,
                            source_sha256: None,
                            execution_started: true,
                        },
                        ResumeRequest {
                            request: 8,
                            run: false,
                            source: Some("check source".into()),
                            program: None,
                            entry: None,
                            source_sha256: None,
                            execution_started: false,
                        },
                    ],
                },
            })
            .unwrap();
        events
            .send(Event::Lifecycle {
                state: Lifecycle::Ready,
            })
            .unwrap();
        app.poll_firmware();
        assert!(commands.try_recv().is_err());
        assert!(app.firmware_resumed && app.firmware_environment.initialized);
        assert_eq!(app.state.firmware.environment, environment);
        assert_eq!(app.firmware_runs, [7].into_iter().collect());
        assert!(app.firmware_pending.is_empty());
        assert_eq!(app.firmware_environment.next, (1 << 63) + 4);
        assert_eq!(app.state.firmware_session, FirmwareSession::Running);
        events
            .send(Event::Result {
                request: 7,
                success: true,
                message: "returned".into(),
                check: None,
            })
            .unwrap();
        app.poll_firmware();
        assert!(!app.firmware_program_running);
        assert!(app.firmware_runs.is_empty());
        assert_eq!(app.firmware_state, crate::ide::VmExecutionState::Terminated);
    }
}
