//! Backend selection, calculator input routing and firmware worker events.
use super::*;
use ppl_engine::session::{Command, Event};
impl IdeDesktopApp {
    pub(super) fn ensure_firmware(&mut self, ctx: &egui::Context) {
        if self
            .firmware
            .as_ref()
            .is_none_or(|f| !f.alive.load(std::sync::atomic::Ordering::Acquire))
        {
            self.firmware_status = "Starting local G1 firmware…".into();
            self.firmware = Some(crate::ide::firmware::FirmwareClient::start(
                self.state.firmware.clone(),
                ctx.clone(),
            ));
        }
    }
    pub(super) fn request_firmware(&mut self, run: bool, ctx: &egui::Context) {
        self.ensure_firmware(ctx);
        let Some(tab) = self.editor.active_tab_mut() else {
            return;
        };
        self.firmware_request += 1;
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
        self.firmware_state = crate::ide::VmExecutionState::Running;
        self.state.vm_state = self.firmware_state.clone();
    }
    pub(super) fn poll_firmware(&mut self) {
        let events = self.firmware.as_ref().map(|f| f.poll()).unwrap_or_default();
        for event in events {
            match event {
                Event::Phase { phase } => {
                    self.state.firmware_input_enabled = phase != "compiling";
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
                } => {
                    if request == self.firmware_request {
                        self.firmware_state = if success {
                            crate::ide::VmExecutionState::Terminated
                        } else {
                            crate::ide::VmExecutionState::Error(message.clone())
                        };
                    }
                    if success {
                        self.console.print_diagnostic(&message);
                    } else {
                        self.console.print_error(&message);
                    }
                    self.firmware_status = message;
                }
                Event::Error { message } => {
                    self.firmware_state = crate::ide::VmExecutionState::Error(message.clone());
                    self.firmware_status = message.clone();
                    self.console.print_error(message);
                }
                Event::Closed { message } => {
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
    pub(super) fn send_calc_transition(&mut self, code: i64, down: bool) {
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
        if !self.state.calc_input_focus {
            return;
        }
        if self.state.firmware_backend
            && self
                .firmware
                .as_ref()
                .is_none_or(|f| !f.alive.load(std::sync::atomic::Ordering::Acquire))
        {
            self.ensure_firmware(ctx);
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
        for event in events {
            match event {
                egui::Event::Key {
                    key,
                    pressed,
                    repeat: _,
                    modifiers,
                    ..
                } if !modifiers.command => {
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
                egui::Event::Text(text) => {
                    for ch in text.chars() {
                        if self.state.firmware_backend {
                            if let Some(f) = &self.firmware {
                                if ch.is_ascii_alphabetic() {
                                    f.send(Command::Key {
                                        code: 0xB6,
                                        down: true,
                                    });
                                    f.send(Command::Key {
                                        code: 0xB6,
                                        down: false,
                                    });
                                    let code = ch.to_ascii_uppercase() as u16;
                                    f.send(Command::Key { code, down: true });
                                    f.send(Command::Key { code, down: false });
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
            if touch.is_some()
                && self
                    .firmware
                    .as_ref()
                    .is_none_or(|f| !f.alive.load(std::sync::atomic::Ordering::Acquire))
            {
                self.ensure_firmware(ctx);
            }
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
