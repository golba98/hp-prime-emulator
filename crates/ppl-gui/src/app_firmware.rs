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
    fn begin_firmware_session(&mut self, ctx: &egui::Context) {
        // The previous helper holds the profile lock; the new session's
        // worker ends it first, so the same calculator profile boots again.
        let previous = self
            .firmware
            .take()
            .or_else(|| self.firmware_retired.take());
        self.reset_firmware_session("The calculator session was replaced");
        self.state.firmware_session = FirmwareSession::Booting;
        self.firmware_status = "Starting local G1 firmware…".into();
        self.firmware = Some(crate::ide::firmware::FirmwareClient::start(
            self.state.firmware.clone(),
            ctx.clone(),
            previous,
        ));
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
        self.prev_shift_held = false;
        self.prev_alpha_held = false;
        self.display.texture = None;
        self.display.last_generation = u64::MAX;
    }
    /// Stores the active tab as a new program on the firmware calculator,
    /// named after the tab. The session refuses an existing name.
    pub(super) fn import_to_calculator(&mut self, ctx: &egui::Context) {
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
                Event::Lifecycle { state } => {
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
                    self.apply_session_check(request, check, &message);
                    self.firmware_imports.remove(&request);
                    self.firmware_program_running = false;
                    if self.state.firmware_session == FirmwareSession::Running {
                        self.state.firmware_session = FirmwareSession::Ready;
                    }
                    let was_run = self.firmware_runs.remove(&request);
                    let stopped =
                        was_run && !success && std::mem::take(&mut self.firmware_stop_requested);
                    if request == self.firmware_request && was_run {
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
                Event::Hello { .. } => {}
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
            self.prev_shift_held = false;
            self.prev_alpha_held = false;
            return;
        }

        // Track host modifier transitions and dispatch Shift / Alpha to calculator.
        let (shift_now, alpha_now) =
            ctx.input(|i| (i.modifiers.shift, i.modifiers.alt || i.modifiers.ctrl));
        if shift_now != self.prev_shift_held {
            self.state.set_key_down(crate::keymap::KEY_SHIFT, shift_now);
            self.send_calc_transition(crate::keymap::KEY_SHIFT, shift_now);
            if shift_now {
                self.state.push_key_event(crate::keymap::KEY_SHIFT);
            }
            self.prev_shift_held = shift_now;
        }
        if alpha_now != self.prev_alpha_held {
            self.state.set_key_down(crate::keymap::KEY_ALPHA, alpha_now);
            self.send_calc_transition(crate::keymap::KEY_ALPHA, alpha_now);
            if alpha_now {
                self.state.push_key_event(crate::keymap::KEY_ALPHA);
            }
            self.prev_alpha_held = alpha_now;
        }

        let events = ctx.input(|i| i.events.clone());
        let has_text = events.iter().any(|e| matches!(e, egui::Event::Text(_)));
        // The desktop's key auto-repeat is not a calculator event: a held key
        // stays down for the firmware, which handles holding itself. Text
        // produced by auto-repeat is dropped with it.
        let repeating = events
            .iter()
            .any(|e| matches!(e, egui::Event::Key { repeat: true, .. }));
        for event in events {
            match event {
                egui::Event::Key {
                    key,
                    pressed,
                    repeat,
                    modifiers,
                    ..
                } if !modifiers.command && !repeat => {
                    let code = egui_key_to_hp(key);
                    if let Some(code) = code {
                        // Text events own printable input, avoiding a duplicate Key + Text press.
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
                        self.state.set_key_down(code, pressed);
                        if !self.state.firmware_backend || !printable || !has_text {
                            self.send_calc_transition(code, pressed);
                        }
                        if pressed && (!printable || !has_text) && !self.state.firmware_backend {
                            self.controller.shared.push_key_press(KeyPress {
                                code: Some(code),
                                ch: None,
                            });
                        }
                    }
                }
                egui::Event::Text(text) if !repeating => {
                    for ch in text.chars() {
                        if self.state.firmware_backend {
                            if let Some(f) = &self.firmware {
                                if ch.is_ascii_alphabetic() {
                                    for code in letter_keys(ch) {
                                        f.send(Command::Key { code, down: true });
                                        f.send(Command::Key { code, down: false });
                                    }
                                } else {
                                    let mut b = [0; 4];
                                    if let Some(code) =
                                        crate::keymap::map_char(ch.encode_utf8(&mut b))
                                    {
                                        if let Some(mapping) = u8::try_from(code)
                                            .ok()
                                            .and_then(crate::keymap::lookup_by_physical)
                                        {
                                            f.send(Command::Key {
                                                code: mapping.device_code,
                                                down: true,
                                            });
                                            f.send(Command::Key {
                                                code: mapping.device_code,
                                                down: false,
                                            });
                                        }
                                    }
                                }
                            }
                        } else {
                            let mut b = [0; 4];
                            let code = crate::keymap::map_char(ch.encode_utf8(&mut b));
                            self.controller
                                .shared
                                .push_key_press(KeyPress { code, ch: Some(ch) });
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
                    }
                }
            }
        } else {
            self.controller.shared.set_touch(touch);
            self.last_touch = touch;
        }
    }
}
/// Device keys that type one letter: Alpha, then (lowercase) Shift, then
/// the key whose Alpha label is the letter. The firmware's device code for
/// that key is the letter's ASCII code (0x41 "A" is Vars); the sequence is
/// the one verified for typed entry names (M7 `invocation_keys`).
pub(crate) fn letter_keys(ch: char) -> Vec<u16> {
    const ALPHA: u16 = 0xB6;
    const SHIFT: u16 = 0x8B;
    let mut keys = vec![ALPHA];
    if ch.is_ascii_lowercase() {
        keys.push(SHIFT);
    }
    keys.push(ch.to_ascii_uppercase() as u16);
    keys
}
/// The calculator program name for a tab title: its file stem.
pub(crate) fn program_name(title: &str) -> String {
    let title = title.trim_end_matches('*').trim();
    title.split('.').next().unwrap_or(title).to_string()
}
#[cfg(test)]
mod tests {
    #[test]
    fn letters_type_with_alpha_and_lowercase_with_shift() {
        assert_eq!(super::letter_keys('A'), vec![0xB6, 0x41]);
        assert_eq!(super::letter_keys('s'), vec![0xB6, 0x8B, 0x53]);
    }
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
}
