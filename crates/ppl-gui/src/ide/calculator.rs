//! Housing, LCD and keys share a single uniform reference transform.
use super::{
    FirmwareSession, IdeAction, IdeAppState, IdePanel,
    display::CalculatorDisplay,
    keypad::VirtualKeypad,
    layout::{CalculatorMode, DEVICE_MIN_SCALE, WorkspaceLayout},
};
use egui::{Color32, Rect, Stroke, StrokeKind, Vec2};
pub const DEVICE_SIZE: Vec2 = Vec2::new(381.0, 797.0);
/// The G1 Firmware session controls: (enabled, title, action).
pub fn session_actions(state: &IdeAppState) -> [(bool, &'static str, IdeAction); 8] {
    let session = &state.firmware_session;
    let asleep = *session == FirmwareSession::Sleeping;
    [
        (
            !session.live() || asleep,
            if asleep { "Power on" } else { "Start" },
            IdeAction::FirmwareStart,
        ),
        (
            matches!(session, FirmwareSession::Ready | FirmwareSession::Running),
            "Power off",
            IdeAction::FirmwarePowerOff,
        ),
        (session.live(), "Shut down", IdeAction::FirmwareShutDown),
        (
            *session != FirmwareSession::Off,
            "Restart",
            IdeAction::FirmwareRestart,
        ),
        (
            matches!(
                session,
                FirmwareSession::Ready | FirmwareSession::Running | FirmwareSession::Sleeping
            ),
            "Save live state",
            IdeAction::FirmwareSaveLive,
        ),
        (
            !matches!(
                session,
                FirmwareSession::Booting | FirmwareSession::Stopping
            ),
            "Resume live state",
            IdeAction::FirmwareResumeLive,
        ),
        (
            *session == FirmwareSession::Ready && state.firmware_managed_reason.is_none(),
            "Import to calculator",
            IdeAction::ImportToCalculator,
        ),
        (
            !matches!(
                session,
                FirmwareSession::Running | FirmwareSession::Stopping
            ),
            "Reset memory…",
            IdeAction::FirmwareReset,
        ),
    ]
}
pub fn session_color(session: &FirmwareSession) -> Color32 {
    use crate::theme::{ACCENT_AMBER, ACCENT_BLUE, ACCENT_GREEN, ACCENT_RED, TEXT_MUTED};
    match session {
        FirmwareSession::Off | FirmwareSession::Sleeping => TEXT_MUTED,
        FirmwareSession::Booting | FirmwareSession::Stopping => ACCENT_AMBER,
        FirmwareSession::Ready => ACCENT_GREEN,
        FirmwareSession::Running => ACCENT_BLUE,
        FirmwareSession::Failed(_) | FirmwareSession::Unavailable(_) => ACCENT_RED,
    }
}
/// State chip, session controls and the unsupported-service count.
fn session_bar(ui: &mut egui::Ui, state: &mut IdeAppState) {
    ui.horizontal_wrapped(|ui| {
        let session = state.firmware_session.clone();
        let (dot, _) = ui.allocate_exact_size(Vec2::splat(10.0), egui::Sense::hover());
        ui.painter()
            .circle_filled(dot.center(), 4.0, session_color(&session));
        let short = match &session {
            FirmwareSession::Failed(_) => "Failed".to_string(),
            FirmwareSession::Unavailable(_) => "Unavailable".to_string(),
            other => other.label(),
        };
        ui.label(egui::RichText::new(short).size(11.5).strong())
            .on_hover_text(session.label());
        for (enabled, title, action) in session_actions(state) {
            let response = ui.add_enabled(enabled, egui::Button::new(title).small());
            match action {
                IdeAction::FirmwareSaveLive => { state.automation_ui_controls.insert("firmware_save_live".into(), response.rect); }
                IdeAction::FirmwareResumeLive => { state.automation_ui_controls.insert("firmware_resume_live".into(), response.rect); }
                _ => {}
            }
            if response.clicked() {
                state.queue_action(action);
                ui.ctx().request_repaint();
            }
        }
        let environment=egui::containers::menu::MenuButton::new("Environment")
            .config(egui::containers::menu::MenuConfig::default().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside))
            .ui(ui,|ui| {
            use ppl_engine::environment::{BatteryPower,BATTERY_VOLTS_PER_SAMPLE};
            let outputs=state.firmware_peripherals;
            ui.label(if outputs.backlight_on {format!("Screen light: {}%",outputs.backlight_duty.saturating_mul(100).min(35000)/350)}else{"Screen light: off".into()});
            ui.label(if outputs.indicator&7==0 {"Indicator lights: off"}else{"Indicator lights: on"});
            ui.label("Virtual calculator battery");
            let battery=&mut state.firmware.environment.battery;
            let mut voltage=battery.voltage();
            let slider=ui.add(egui::Slider::new(&mut voltage,3.0..=4.65).text("Voltage (V)").fixed_decimals(2));
            state.automation_ui_controls.insert("battery_voltage".into(),Rect::from_min_size(slider.rect.min,Vec2::new(ui.spacing().slider_width,slider.rect.height())));
            if slider.changed() {
                battery.adc=(voltage/BATTERY_VOLTS_PER_SAMPLE).round().clamp(0.0,1023.0) as u16;
            }
            for (name,power,label) in [("battery_power",BatteryPower::Battery,"On battery"),("battery_charging",BatteryPower::Charging,"Charging"),("battery_charged",BatteryPower::Charged,"Fully charged")] {
                let response=ui.radio_value(&mut battery.power,power,label);
                state.automation_ui_controls.insert(name.into(),response.rect);
            }
            let apply=ui.add_enabled(session.live(),egui::Button::new("Apply battery settings"));
            state.automation_ui_controls.insert("battery_apply".into(),apply.rect);
            if apply.clicked() {
                state.queue_action(IdeAction::FirmwareEnvironmentApply);
                ui.close();
            }
        });
        state.automation_ui_controls.insert("firmware_environment".into(),environment.0.rect);
        if ui.small_button("Configure firmware…").clicked() {
            state.queue_action(IdeAction::ShowSettingsDialog);
        }
        if matches!(session, FirmwareSession::Unavailable(_))
            && ui.small_button("Use Native PPL").clicked()
        {
            state.queue_action(IdeAction::SelectBackend(false));
        }
        if let Some(reason) = &state.firmware_managed_reason { ui.label(reason); }
        let unsupported = state.firmware_unsupported.len();
        if unsupported > 0 {
            let text = format!("{unsupported} unsupported OS service call site(s)");
            ui.label(
                egui::RichText::new(format!("⚠ {unsupported} SVC"))
                    .size(11.0)
                    .color(crate::theme::ACCENT_AMBER),
            )
            .on_hover_ui(|ui| {
                ui.label(text);
                for (id, name, pc) in state.firmware_unsupported.iter().take(24) {
                    ui.monospace(format!(
                        "{id:#07X} {} @ {pc:#010X}",
                        name.as_deref().unwrap_or("unnamed")
                    ));
                }
                ui.label("The emulator returned 0 for each; behaviour that depends on them is not authentic.");
            });
        }
    });
}
pub fn device_rect(available: Rect) -> Rect {
    let scale = (available.width() / DEVICE_SIZE.x)
        .min(available.height() / DEVICE_SIZE.y)
        .max(0.0);
    Rect::from_center_size(available.center(), DEVICE_SIZE * scale)
}
pub fn show(
    ui: &mut egui::Ui,
    state: &mut IdeAppState,
    display: &mut CalculatorDisplay,
    keypad: &mut VirtualKeypad,
) {
    state.automation_ui_controls.clear();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("HP PRIME").size(12.0).strong());
        ui.label(
            egui::RichText::new("G1")
                .size(11.0)
                .color(crate::theme::TEXT_MUTED),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let expanded = state.layout == WorkspaceLayout::Calculator;
            if ui
                .small_button(if expanded {
                    "Return to editor"
                } else {
                    "Expand"
                })
                .clicked()
            {
                state.queue_action(IdeAction::SetLayout(if expanded {
                    state.previous_layout
                } else {
                    WorkspaceLayout::Calculator
                }));
            }
            ui.menu_button("···", |ui| {
                for (mode, name) in [
                    (CalculatorMode::Full, "Full Calculator"),
                    (CalculatorMode::ScreenOnly, "Screen Only"),
                ] {
                    if ui
                        .selectable_label(state.calculator_mode == mode, name)
                        .clicked()
                    {
                        state.queue_action(IdeAction::SetCalculatorMode(mode));
                        ui.close();
                    }
                }
                ui.separator();
                ui.checkbox(&mut state.smooth_display, "Smooth LCD scaling");
                if state.firmware_backend {
                    ui.separator();
                    ui.label("Desktop typing follows the firmware Alpha indicator.");
                    ui.label("Tab: Alpha · Shift+Tab: calculator Shift");
                    ui.label("Ctrl+V: paste one expression · Esc: cancel queued typing");
                    ui.label("Right: leave a power/fraction template");
                    if state.firmware_clipboard_available {
                        ui.label("Calculator Copy/Cut also copies selected text to the desktop clipboard.");
                    } else {
                        ui.label("Desktop copy-out has no verified profile for this firmware.");
                    }
                    ui.label("Paste PPL source into the IDE editor, then Import.");
                }
                if ui.button("Save LCD screenshot…").clicked() {
                    state.queue_action(IdeAction::TakeScreenshot);
                    ui.close();
                }
            });
        });
    });
    ui.add_space(12.0);
    if !state.firmware_backend {
        ui.horizontal_wrapped(|ui| {
            let idle = !matches!(
                state.vm_state,
                super::VmExecutionState::Running | super::VmExecutionState::Paused
            );
            for (label, action) in [
                ("Home", IdeAction::NativeHome),
                ("Programs", IdeAction::NativeCatalog),
            ] {
                if ui.add_enabled(idle, egui::Button::new(label)).clicked() {
                    state.queue_action(action);
                    ui.ctx().request_repaint();
                }
            }
            if state.can_dismiss_output()
                && ui
                    .button(match state.native.caller {
                        super::native::NativeCaller::Home => "Back to Home (Esc)",
                        super::native::NativeCaller::Catalog => "Back to Programs (Esc)",
                    })
                    .clicked()
            {
                state.queue_action(IdeAction::DismissOutput);
                ui.ctx().request_repaint();
            }
        });
        ui.add_space(4.0);
    } else {
        session_bar(ui, state);
        ui.add_space(4.0);
    }
    if state.calculator_mode == CalculatorMode::ScreenOnly {
        keypad.release(state);
        display.render(ui, state);
        return;
    }
    let available = ui.available_rect_before_wrap().shrink(8.0);
    if device_rect(available).width() / DEVICE_SIZE.x < DEVICE_MIN_SCALE
        && state.layout != WorkspaceLayout::Calculator
    {
        if ui.small_button("Expand for larger keys").clicked() {
            state.queue_action(IdeAction::SetLayout(WorkspaceLayout::Calculator));
        }
        ui.add_space(4.0);
    }
    let rect = device_rect(ui.available_rect_before_wrap().shrink(8.0));
    let scale = rect.width() / DEVICE_SIZE.x;
    if scale <= 0.0 {
        keypad.release(state);
        return;
    }
    let p = ui.painter();
    // Original vector materials: restrained metal rim, black upper face, silver key bank.
    p.rect_filled(rect, 20.0 * scale, Color32::from_gray(162));
    p.rect_stroke(
        rect.shrink(scale),
        20.0 * scale,
        Stroke::new(scale, Color32::from_gray(204)),
        StrokeKind::Inside,
    );
    p.rect_filled(
        rect.shrink(5.0 * scale),
        16.0 * scale,
        Color32::from_gray(106),
    );
    let at = |x, y| rect.min + Vec2::new(x, y) * scale;
    let upper = Rect::from_min_max(at(10.0, 8.0), at(371.0, 417.0));
    p.rect_filled(upper, 8.0 * scale, Color32::from_gray(7));
    let bank = Rect::from_min_max(at(15.0, 417.0), at(366.0, 776.0));
    p.rect_filled(bank, 7.0 * scale, Color32::from_gray(166));
    p.line_segment(
        [bank.left_top(), bank.right_top()],
        Stroke::new(scale, Color32::from_gray(205)),
    );
    p.text(
        at(21.0, 27.0),
        egui::Align2::LEFT_CENTER,
        "HP Prime Graphing Calculator",
        egui::FontId::proportional(13.0 * scale),
        Color32::WHITE,
    );
    p.circle_filled(at(338.0, 27.0), 12.0 * scale, Color32::from_gray(235));
    p.text(
        at(338.0, 27.0),
        egui::Align2::CENTER_CENTER,
        "hp",
        egui::FontId::proportional(17.0 * scale),
        Color32::from_gray(12),
    );
    let lcd = Rect::from_min_size(at(28.0, 58.0), Vec2::new(320.0, 240.0) * scale);
    display.paint_at(ui, state, lcd);
    keypad.paint_at(ui, state, rect.min, scale);
    ui.allocate_space(ui.available_size());
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uniform_fit_never_stretches_or_crops() {
        for (w, h) in [(400.0, 900.0), (800.0, 600.0), (2560.0, 1440.0)] {
            let available = Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(w, h));
            let r = device_rect(available);
            assert!((r.width() / r.height() - 381.0 / 797.0).abs() < 0.0001);
            assert!(available.expand(0.001).contains_rect(r));
        }
    }

    #[test]
    fn full_calculator_keeps_complete_body_in_narrow_and_high_dpi_views() {
        for pixels_per_point in [0.75, 1.0, 1.5, 2.0, 2.5] {
            for size in [
                Vec2::new(240.0, 400.0),
                Vec2::new(320.0, 500.0),
                Vec2::new(700.0, 900.0),
            ] {
                let ctx = egui::Context::default();
                ctx.set_pixels_per_point(pixels_per_point);
                let mut state = IdeAppState::default();
                let mut display = CalculatorDisplay::new();
                let mut keypad = VirtualKeypad::new();
                let viewport = Rect::from_min_size(egui::Pos2::ZERO, size);
                // Egui applies a requested DPI change at the next frame;
                // settle it before supplying the measured viewport.
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(viewport),
                        ..Default::default()
                    },
                    |_| {},
                );
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(viewport),
                        ..Default::default()
                    },
                    |ctx| {
                        assert_eq!(ctx.screen_rect(), viewport);
                        egui::CentralPanel::default()
                            .show(ctx, |ui| show(ui, &mut state, &mut display, &mut keypad));
                    },
                );
                let lcd = display.last_display_rect;
                let scale = lcd.width() / 320.0;
                let body = Rect::from_min_size(
                    lcd.min - Vec2::new(28.0, 58.0) * scale,
                    DEVICE_SIZE * scale,
                );
                assert!(
                    viewport.expand(0.01).contains_rect(body),
                    "{size:?} at {pixels_per_point}: {body:?}"
                );
                assert!((lcd.height() - 240.0 * scale).abs() < 0.01);
                for key in super::super::keypad::KEYS {
                    assert!(body.expand(0.01).contains_rect(key.rect(body.min, scale)));
                }
                if size.x == 240.0 {
                    assert!(scale < DEVICE_MIN_SCALE);
                }
                assert_eq!(state.calculator_mode, CalculatorMode::Full);
            }
        }
    }
    #[test]
    fn environment_widgets_change_battery_and_queue_the_real_apply_action() {
        let ctx = egui::Context::default();
        let mut state = IdeAppState {
            firmware_backend: true,
            firmware_session: FirmwareSession::Ready,
            layout: WorkspaceLayout::Calculator,
            ..Default::default()
        };
        let mut display = CalculatorDisplay::default();
        let mut keypad = VirtualKeypad::default();
        let mut frame = |state: &mut IdeAppState, events: Vec<egui::Event>| {
            let _ = ctx.run(
                egui::RawInput {
                    focused: true,
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280., 1000.),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .show(ctx, |ui| show(ui, state, &mut display, &mut keypad));
                },
            );
        };
        let mut click = |state: &mut IdeAppState, name: &str| {
            frame(state, vec![]);
            let pos = state
                .automation_ui_controls
                .get(name)
                .unwrap_or_else(|| panic!("missing visible {name}"))
                .center();
            frame(
                state,
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
            frame(
                state,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        };
        click(&mut state, "firmware_environment");
        click(&mut state, "battery_voltage");
        click(&mut state, "battery_charging");
        click(&mut state, "battery_apply");
        // The slider rounds to0.01 V before quantization to a10-bit sample.
        assert!((state.firmware.environment.battery.voltage() - 3.825).abs() <= 0.01);
        assert_eq!(
            state.firmware.environment.battery.power,
            ppl_engine::environment::BatteryPower::Charging
        );
        assert!(
            state
                .take_actions()
                .iter()
                .any(|action| matches!(action, IdeAction::FirmwareEnvironmentApply))
        );
        let saved = serde_json::to_vec(&state.firmware).unwrap();
        let restored: crate::ide::firmware::FirmwareSettings =
            serde_json::from_slice(&saved).unwrap();
        assert_eq!(restored.environment, state.firmware.environment);
    }
}
