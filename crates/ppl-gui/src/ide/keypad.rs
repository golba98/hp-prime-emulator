//! Proportional HP Prime G1 keys, with press/release pointer capture.
mod definitions;
use super::{IdeAppState, IdePanel};
pub use definitions::KEYS;
use egui::{Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};

#[derive(Debug, Clone, Copy)]
pub struct KeyDef {
    pub label: &'static str,
    pub code: i64,
    pub device_code: u16,
    pub secondary: &'static str,
    pub alpha: &'static str,
    /// Absolute coordinates in the 381×797 reference device.
    pub bounds: [f32; 4],
}
impl KeyDef {
    pub fn rect(&self, origin: Pos2, scale: f32) -> Rect {
        Rect::from_min_max(
            origin + Vec2::new(self.bounds[0], self.bounds[1]) * scale,
            origin + Vec2::new(self.bounds[2], self.bounds[3]) * scale,
        )
    }
    pub fn directional(&self) -> bool {
        matches!(self.code, 2 | 7 | 8 | 12)
    }
}

#[derive(Debug, Default)]
pub struct VirtualKeypad {
    pub last_clicked_key: Option<i64>,
    captured: Option<i64>,
}
impl VirtualKeypad {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn release(&mut self, state: &mut IdeAppState) {
        if let Some(code) = self.captured.take() {
            state.set_key_down(code, false);
            state.key_transitions.push((code, false));
        }
    }
    pub fn paint_at(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut IdeAppState,
        origin: Pos2,
        scale: f32,
    ) {
        let program_input = if state.firmware_backend {
            state.firmware_input_enabled
        } else {
            matches!(
                state.vm_state,
                super::VmExecutionState::Running | super::VmExecutionState::Paused
            )
        };
        let p = ui.painter();
        let pos = |x, y| origin + Vec2::new(x, y) * scale;
        // The physical navigation mechanism has four directions, no center key.
        let center = pos(187.0, 367.0);
        p.circle_filled(center, 42.0 * scale, crate::theme::CALC_DPAD_RIM);
        p.circle_filled(center, 38.5 * scale, crate::theme::CALC_DPAD_BODY);
        // Center dead-zone pivot cap
        p.circle_filled(center, 9.5 * scale, crate::theme::CALC_DPAD_PIVOT);
        p.circle_stroke(
            center,
            9.5 * scale,
            Stroke::new(scale, crate::theme::CALC_DPAD_PIVOT_STROKE),
        );
        for (code, x, y) in [
            (2, 187.0, 332.0),
            (7, 153.0, 367.0),
            (8, 221.0, 367.0),
            (12, 187.0, 402.0),
        ] {
            let active = state.keys_down.contains(&code);
            p.circle_filled(
                pos(x, y),
                2.5 * scale,
                if active {
                    crate::theme::CALC_DPAD_MARKER_ACTIVE
                } else {
                    crate::theme::CALC_DPAD_MARKER
                },
            );
        }
        let input = ui.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.interact_pos(),
                i.focused,
            )
        });
        if !input.1 || !input.3 {
            self.release(state);
        }
        for key in &KEYS {
            let dismiss_output = key.code == crate::keymap::KEY_ESC && state.can_dismiss_output();
            let enabled =
                program_input || (!state.firmware_backend && state.native.supports_key(key.code));
            let rect = key.rect(origin, scale);
            let response = ui.interact(rect, ui.id().with(("g1-key", key.code)), Sense::hover());
            let over = enabled
                && input.2.is_some_and(|pos| {
                    if key.directional() {
                        navigation_at(pos, center, scale) == Some(key.code)
                    } else {
                        rect.contains(pos)
                    }
                });
            if input.0 && over && self.captured.is_none() {
                state.calc_input_focus = true;
                ui.ctx().memory_mut(|m| {
                    if let Some(id) = m.focused() {
                        m.surrender_focus(id);
                    }
                });
                self.last_clicked_key = Some(key.code);
                self.captured = Some(key.code);
                state.set_key_down(key.code, true);
                state.key_transitions.push((key.code, true));
                state.push_key_event(key.code);
            }
            let pressed = state.keys_down.contains(&key.code);
            if key.directional() {
                if pressed || over {
                    ui.painter().rect_filled(
                        rect.shrink(scale * 2.0),
                        8.0 * scale,
                        Color32::from_white_alpha(if pressed { 40 } else { 15 }),
                    );
                }
                response.on_hover_text(key.label);
                continue;
            }
            let upper = key.code < 14;
            let fill = if upper {
                Color32::from_gray(if pressed {
                    65
                } else if over {
                    58
                } else {
                    39
                })
            } else {
                Color32::from_gray(if pressed {
                    190
                } else if over {
                    244
                } else {
                    226
                })
            };
            let p = ui.painter();
            p.rect_filled(
                rect,
                4.0 * scale,
                Color32::from_gray(if upper { 80 } else { 110 }),
            );
            p.rect_filled(rect.shrink(1.5 * scale), 3.0 * scale, fill);
            p.line_segment(
                [
                    rect.left_top() + Vec2::splat(3.0 * scale),
                    rect.right_top() + Vec2::new(-3.0, 3.0) * scale,
                ],
                Stroke::new(scale, Color32::from_gray(if upper { 100 } else { 255 })),
            );
            let primary = if upper {
                Color32::from_gray(245)
            } else {
                Color32::from_gray(29)
            };
            let blue = if upper {
                crate::theme::CALC_KEY_FN_SEC
            } else {
                crate::theme::CALC_KEY_NUM_SEC
            };
            let orange = crate::theme::CALC_KEY_ALPHA_FG;
            let label_y = if key.secondary.is_empty() { 0.5 } else { 0.32 };
            let label_size = if upper {
                13.0
            } else if (32..=50).contains(&key.code) && !matches!(key.code, 36 | 41 | 46) {
                24.0
            } else if key.code == 17 {
                14.0
            } else {
                16.0
            };
            let label = match key.code {
                15 => "▱",
                16 => "∑√▦",
                5 => "⌂",
                _ => key.label,
            };
            if matches!(key.code, 36 | 41) {
                let band = Rect::from_min_size(
                    rect.min + Vec2::splat(3.0 * scale),
                    Vec2::new(rect.width() - 6.0 * scale, rect.height() * 0.48),
                );
                p.rect_filled(
                    band,
                    2.0 * scale,
                    if key.code == 36 {
                        crate::theme::CALC_KEY_ALPHA
                    } else {
                        crate::theme::CALC_KEY_SHIFT
                    },
                );
                p.text(
                    band.center(),
                    egui::Align2::CENTER_CENTER,
                    label,
                    FontId::proportional(14.0 * scale),
                    Color32::WHITE,
                );
            } else if key.code == 15 {
                let icon = Rect::from_center_size(
                    Pos2::new(rect.center().x, rect.top() + 13.0 * scale),
                    Vec2::new(28.0, 14.0) * scale,
                );
                p.rect_filled(icon, 1.0 * scale, primary);
                p.rect_stroke(
                    Rect::from_center_size(icon.center_top(), Vec2::new(10.0, 5.0) * scale),
                    1.0 * scale,
                    Stroke::new(1.5 * scale, primary),
                    egui::StrokeKind::Inside,
                );
                p.line_segment(
                    [icon.left_center(), icon.right_center()],
                    Stroke::new(scale, Color32::from_gray(226)),
                );
                p.rect_filled(
                    Rect::from_center_size(icon.center(), Vec2::new(4.0, 3.0) * scale),
                    0.0,
                    Color32::WHITE,
                );
            } else if key.code == 5 {
                let c = Pos2::new(rect.center().x, rect.top() + 12.0 * scale);
                let s = Stroke::new(1.5 * scale, primary);
                p.line_segment(
                    [
                        c + Vec2::new(-9.0, 1.0) * scale,
                        c + Vec2::new(0.0, -7.0) * scale,
                    ],
                    s,
                );
                p.line_segment(
                    [
                        c + Vec2::new(0.0, -7.0) * scale,
                        c + Vec2::new(9.0, 1.0) * scale,
                    ],
                    s,
                );
                p.rect_stroke(
                    Rect::from_min_max(
                        c + Vec2::new(-6.0, 0.0) * scale,
                        c + Vec2::new(6.0, 8.0) * scale,
                    ),
                    0.0,
                    s,
                    egui::StrokeKind::Inside,
                );
            } else {
                p.text(
                    Pos2::new(rect.center().x, rect.top() + rect.height() * label_y),
                    egui::Align2::CENTER_CENTER,
                    label,
                    FontId::proportional(label_size * scale),
                    primary,
                );
            }
            if !key.secondary.is_empty() {
                let align = if key.alpha.is_empty() {
                    egui::Align2::CENTER_CENTER
                } else {
                    egui::Align2::LEFT_CENTER
                };
                let x = if key.alpha.is_empty() {
                    rect.center().x
                } else {
                    rect.left() + 4.0 * scale
                };
                p.text(
                    Pos2::new(x, rect.bottom() - 8.0 * scale),
                    align,
                    key.secondary,
                    FontId::proportional(11.5 * scale),
                    blue,
                );
            }
            if !key.alpha.is_empty() {
                p.text(
                    Pos2::new(rect.right() - 3.0 * scale, rect.bottom() - 8.0 * scale),
                    egui::Align2::RIGHT_CENTER,
                    key.alpha,
                    FontId::proportional(11.5 * scale),
                    orange,
                );
            }
            response.on_hover_text(if dismiss_output {"Return to the caller; keep the program result and output history".into()} else if enabled {format!("{} · HP key {}",key.label,key.code)}else if state.firmware_backend {"Calculator input is reserved while the firmware compiles".into()}else {"This key is unavailable in Native Home/PPL. CAS and built-in calculator apps are not implemented.".into()});
            if !enabled {
                ui.painter()
                    .rect_filled(rect, 3.0 * scale, Color32::from_black_alpha(75));
            }
        }
    }
}
impl IdePanel for VirtualKeypad {
    fn render(&mut self, ui: &mut egui::Ui, state: &mut IdeAppState) {
        let scale = (ui.available_width() / 381.0).max(0.1);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(381.0, 435.0) * scale, Sense::hover());
        self.paint_at(ui, state, rect.min - Vec2::new(0.0, 314.0) * scale, scale);
    }
}

/// Four physical navigation sectors; the center has no click action.
pub fn navigation_at(point: Pos2, center: Pos2, scale: f32) -> Option<i64> {
    let d = (point - center) / scale;
    let radius = d.length();
    if !(10.0..=42.0).contains(&radius) {
        return None;
    }
    Some(if d.x.abs() > d.y.abs() {
        if d.x < 0.0 { 7 } else { 8 }
    } else if d.y < 0.0 {
        2
    } else {
        12
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pointer_press(state: &mut IdeAppState, code: i64) {
        let ctx = egui::Context::default();
        let origin = Pos2::new(10.0, 10.0);
        let point = KEYS
            .iter()
            .find(|key| key.code == code)
            .unwrap()
            .rect(origin, 1.0)
            .center();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(420.0, 850.0))),
            events: vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        };
        let mut keypad = VirtualKeypad::new();
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| keypad.paint_at(ui, state, origin, 1.0));
        });
    }

    #[test]
    fn finished_native_esc_is_a_view_action_and_running_esc_remains_program_input() {
        let mut finished = IdeAppState {
            vm_state: super::super::VmExecutionState::Terminated,
            ..Default::default()
        };
        finished.native.view = super::super::native::NativeView::ProgramOutput;
        pointer_press(&mut finished, crate::keymap::KEY_ESC);
        assert!(finished.take_actions().is_empty());
        assert_eq!(finished.key_events, [crate::keymap::KEY_ESC]);
        finished.key_events.clear();
        finished.key_transitions.clear();
        finished.clear_held_keys();
        pointer_press(&mut finished, crate::keymap::KEY_ENTER);
        assert!(finished.take_actions().is_empty() && finished.key_events.is_empty());

        for firmware in [false, true] {
            let mut running = IdeAppState {
                firmware_backend: firmware,
                vm_state: super::super::VmExecutionState::Running,
                ..Default::default()
            };
            pointer_press(&mut running, crate::keymap::KEY_ESC);
            assert!(running.take_actions().is_empty());
            assert_eq!(running.key_events, [crate::keymap::KEY_ESC]);
            assert_eq!(running.key_transitions, [(crate::keymap::KEY_ESC, true)]);
        }
    }

    #[test]
    fn authentic_geometry_and_codes_are_complete() {
        let mut seen = std::collections::HashSet::new();
        for key in KEYS {
            assert!(seen.insert(key.code));
            let r = key.rect(Pos2::ZERO, 1.0);
            assert!(r.is_positive() && r.right() <= 381.0 && r.bottom() <= 797.0);
            for other in KEYS
                .iter()
                .filter(|k| k.code > key.code && !k.directional() && !key.directional())
            {
                assert!(
                    !r.intersects(other.rect(Pos2::ZERO, 1.0)),
                    "{} overlaps {}",
                    key.label,
                    other.label
                );
            }
        }
        assert_eq!(seen.len(), 51);
        assert_eq!(KEYS[5].device_code, 0xE047);
        assert_eq!(KEYS[30].bounds, [242.0, 516.0, 347.0, 555.0]);
        assert!(
            KEYS[30].rect(Pos2::ZERO, 1.0).width() > 2.0 * KEYS[26].rect(Pos2::ZERO, 1.0).width()
        );
        assert!(KEYS[32].rect(Pos2::ZERO, 1.0).width() > KEYS[31].rect(Pos2::ZERO, 1.0).width());
    }
}
