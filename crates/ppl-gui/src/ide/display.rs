//! 320×240 GROB framebuffer display container with nearest-neighbor scaling.

use super::{IdeAction, IdeAppState, IdePanel};
use crate::theme::{ACCENT_BLUE, BORDER_CRISP, TEXT_MUTED};
use egui::{Color32, ColorImage, Rect, RichText, Stroke, StrokeKind, TextureOptions, Vec2};
use ppl_graphics::Grob;

/// Container hosting the 320×240 calculator screen.
pub struct CalculatorDisplay {
    pub texture: Option<egui::TextureHandle>,
    pub last_generation: u64,
    pub zoom_mode: ZoomMode,
    /// Screen rect in window coordinates, for touch mapping.
    pub last_display_rect: Rect,
}

impl Default for CalculatorDisplay {
    fn default() -> Self {
        Self {
            texture: None,
            last_generation: 0,
            zoom_mode: ZoomMode::default(),
            last_display_rect: Rect::NOTHING,
        }
    }
}

impl std::fmt::Debug for CalculatorDisplay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CalculatorDisplay")
            .field("has_texture", &self.texture.is_some())
            .field("last_generation", &self.last_generation)
            .field("zoom_mode", &self.zoom_mode)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ZoomMode {
    Fit,
    #[default]
    Scale1x,
    Scale2x,
}

impl CalculatorDisplay {
    pub fn new() -> Self {
        Self::default()
    }

    /// Translates an egui window point into emulated 320×240 framebuffer coordinates.
    pub fn project_pointer_pos(&self, pos: egui::Pos2) -> Option<ppl_vm::host::Touch> {
        let rect = self.last_display_rect;
        if !rect.contains(pos) || rect.width() <= 0.0 || rect.height() <= 0.0 {
            return None;
        }
        let fb_x = (((pos.x - rect.min.x) as f64 * crate::scaler::LOGICAL_WIDTH as f64)
            / rect.width() as f64)
            .floor() as i64;
        let fb_y = (((pos.y - rect.min.y) as f64 * crate::scaler::LOGICAL_HEIGHT as f64)
            / rect.height() as f64)
            .floor() as i64;
        Some(ppl_vm::host::Touch {
            x: fb_x.clamp(0, crate::scaler::LOGICAL_WIDTH as i64 - 1),
            y: fb_y.clamp(0, crate::scaler::LOGICAL_HEIGHT as i64 - 1),
        })
    }

    /// Updates the texture from a 320×240 Grob framebuffer.
    pub fn update_from_grob(&mut self, ctx: &egui::Context, grob: &Grob, generation: u64) {
        if self.last_generation == generation && self.texture.is_some() {
            return;
        }

        let width = grob.width();
        let height = grob.height();
        let mut pixels = Vec::with_capacity(width * height);

        for &p in grob.pixels() {
            let (r, g, b) = ppl_graphics::channels(p);
            pixels.push(Color32::from_rgb(r, g, b));
        }

        let image = ColorImage::new([width, height], pixels);

        if let Some(tex) = &mut self.texture {
            tex.set(image, TextureOptions::NEAREST);
        } else {
            self.texture = Some(ctx.load_texture("calc_screen", image, TextureOptions::NEAREST));
        }

        self.last_generation = generation;
    }
}

impl IdePanel for CalculatorDisplay {
    fn render(&mut self, ui: &mut egui::Ui, state: &mut IdeAppState) {
        ui.vertical(|ui| {
            // Header
            ui.horizontal(|ui| {
                ui.label(RichText::new("🖩 HP PRIME G1 SCREEN").strong().size(12.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button("📷")
                        .on_hover_text("Save Screenshot (PNG)")
                        .clicked()
                    {
                        state.queue_action(IdeAction::TakeScreenshot);
                    }

                    // Zoom selector
                    ui.selectable_value(&mut self.zoom_mode, ZoomMode::Fit, "Fit");
                    ui.selectable_value(&mut self.zoom_mode, ZoomMode::Scale2x, "2x");
                    ui.selectable_value(&mut self.zoom_mode, ZoomMode::Scale1x, "1x");
                });
            });

            ui.add(egui::Separator::default().spacing(4.0));

            // Screen Frame Bezel Area
            // Narrow panels scale the screen down instead of clipping it.
            let available_width = ui.available_width().max(168.0);
            let (target_w, target_h) = match self.zoom_mode {
                ZoomMode::Scale1x => (320.0, 240.0),
                ZoomMode::Scale2x => (320.0 * 2.0, 240.0 * 2.0),
                ZoomMode::Fit => {
                    let w = (available_width - 8.0).max(160.0);
                    let h = w * (240.0 / 320.0);
                    (w, h)
                }
            };

            let (rect, response) = ui.allocate_exact_size(
                Vec2::new(available_width, target_h + 12.0),
                egui::Sense::click_and_drag(),
            );

            // Centered display rectangle
            let eff_w = target_w.min(available_width - 8.0);
            let eff_h = eff_w * (240.0 / 320.0);
            let display_rect = Rect::from_center_size(rect.center(), Vec2::new(eff_w, eff_h));

            // Update display scale for status bar accuracy
            state.display_scale = eff_w / 320.0;

            // Outer Bezel
            ui.painter()
                .rect_filled(rect, 0.0, crate::theme::BG_EXTREME);
            ui.painter().rect_stroke(
                rect,
                0.0,
                Stroke::new(1.0_f32, BORDER_CRISP),
                StrokeKind::Inside,
            );

            // Record the display rect for touch mapping and focus.
            self.last_display_rect = display_rect;
            if response.clicked() || response.dragged() {
                state.calc_input_focus = true;
            }

            // Inner Display
            if let Some(texture) = &self.texture {
                ui.painter().image(
                    texture.id(),
                    display_rect,
                    Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                // Placeholder when no program is running yet
                ui.painter()
                    .rect_filled(display_rect, 0.0, Color32::from_rgb(0x10, 0x14, 0x18));
                let center = display_rect.center();
                ui.painter().text(
                    center,
                    egui::Align2::CENTER_CENTER,
                    "320×240 Framebuffer\n(Ready for execution)",
                    egui::FontId::monospace(12.0),
                    TEXT_MUTED,
                );
            }

            let border_stroke = if state.calc_input_focus {
                Stroke::new(1.0_f32, ACCENT_BLUE)
            } else {
                Stroke::new(1.0_f32, Color32::from_rgb(0x35, 0x3B, 0x41))
            };
            ui.painter()
                .rect_stroke(display_rect, 0.0, border_stroke, StrokeKind::Inside);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_project_pointer_pos_bounds_and_flooring() {
        let mut display = CalculatorDisplay::new();
        display.last_display_rect =
            Rect::from_min_max(egui::pos2(100.0, 50.0), egui::pos2(420.0, 290.0)); // 320×240

        // Center
        let touch = display
            .project_pointer_pos(egui::pos2(260.0, 170.0))
            .unwrap();
        assert_eq!(touch.x, 160);
        assert_eq!(touch.y, 120);

        // Top-left
        let touch = display
            .project_pointer_pos(egui::pos2(100.0, 50.0))
            .unwrap();
        assert_eq!(touch.x, 0);
        assert_eq!(touch.y, 0);

        // Subpixel flooring: 100.9 offset -> floor is 0
        let touch = display
            .project_pointer_pos(egui::pos2(100.9, 50.9))
            .unwrap();
        assert_eq!(touch.x, 0);
        assert_eq!(touch.y, 0);

        // Bottom-right inside bounds
        let touch = display
            .project_pointer_pos(egui::pos2(419.9, 289.9))
            .unwrap();
        assert_eq!(touch.x, 319);
        assert_eq!(touch.y, 239);

        // Outside rect -> None
        assert!(
            display
                .project_pointer_pos(egui::pos2(50.0, 50.0))
                .is_none()
        );
        assert!(
            display
                .project_pointer_pos(egui::pos2(500.0, 200.0))
                .is_none()
        );
    }
}
