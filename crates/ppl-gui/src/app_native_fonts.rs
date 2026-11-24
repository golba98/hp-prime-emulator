//! Select calculator fonts independently of the IDE's text and code families.
use super::*;
impl IdeDesktopApp {
    pub(super) fn refresh_native_fonts(&mut self, ctx: &egui::Context) {
        if !self.native_fonts.refresh(&self.state.firmware) {
            return;
        }
        *self
            .controller
            .shared
            .prime_fonts
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = self.native_fonts.selected.clone();
        ctx.set_fonts(self.native_fonts.definitions());
        self.state.native.history_font_px =
            self.native_tools.memory.presentation.font_size.pixels()
                - if self.native_fonts.selected.is_some() {
                    0.
                } else {
                    1.
                };
        self.native_tools
            .set_prime_font(self.native_fonts.selected.is_some());
        self.state.native.touch_view();
        self.display.last_generation = u64::MAX;
        self.display.texture = None;
        self.tool_capture_stamp = None;
        ctx.request_repaint();
    }
    pub(super) fn show_native_font_problem(&mut self, ctx: &egui::Context) {
        if self.state.firmware_backend || self.state.native.sleeping {
            return;
        }
        let Some(error) = &self.native_fonts.error else {
            return;
        };
        let lcd = self.display.last_display_rect;
        if !lcd.is_positive() {
            return;
        }
        egui::Area::new("native_font_problem".into())
            .order(egui::Order::Foreground)
            .fixed_pos(lcd.min)
            .fade_in(false)
            .show(ctx, |ui| {
                ui.set_clip_rect(lcd);
                ui.set_min_size(lcd.size());
                ui.set_max_size(lcd.size());
                ui.painter().rect_filled(lcd, 0., egui::Color32::WHITE);
                ui.vertical_centered(|ui| {
                    ui.add_space(16.);
                    ui.colored_label(egui::Color32::DARK_RED, "Calculator font unavailable");
                    ui.label("Check the firmware resource path in emulator settings.");
                    ui.label(error);
                });
            });
    }
}
