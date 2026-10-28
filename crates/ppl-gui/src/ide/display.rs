//! 320×240 GROB framebuffer display container with nearest-neighbor scaling.

use super::{IdeAppState, IdePanel};
use crate::theme::{ACCENT_BLUE, BORDER_CRISP};
use egui::{Color32, ColorImage, Rect, Stroke, StrokeKind, TextureOptions, Vec2};
use ppl_graphics::Grob;

/// Container hosting the 320×240 calculator screen.
pub struct CalculatorDisplay {
    pub texture: Option<egui::TextureHandle>,
    pub last_generation: u64,
    pub zoom_mode: ZoomMode,
    pub smooth: bool,
    /// Screen rect in window coordinates, for touch mapping.
    pub last_display_rect: Rect,
}

impl Default for CalculatorDisplay {
    fn default() -> Self {
        Self {
            texture: None,
            last_generation: 0,
            zoom_mode: ZoomMode::default(),
            smooth: false,
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

        let options = self.texture_options();
        if let Some(tex) = &mut self.texture {
            tex.set(image, options);
        } else {
            self.texture = Some(ctx.load_texture("calc_screen", image, self.texture_options()));
        }

        self.last_generation = generation;
    }
}

impl CalculatorDisplay {
    fn texture_options(&self) -> TextureOptions {
        if self.smooth {
            TextureOptions::LINEAR
        } else {
            TextureOptions::NEAREST
        }
    }
    pub fn set_smooth(&mut self, smooth: bool) {
        if self.smooth != smooth {
            self.smooth = smooth;
            self.last_generation = u64::MAX;
        }
    }
    /// Firmware pixels are 0x00RRGGBB, not the packed physical USB format.
    pub fn update_xrgb(&mut self, ctx: &egui::Context, pixels: &[u32], generation: u64) {
        if pixels.len() != 320 * 240
            || (self.last_generation != u64::MAX
                && self.last_generation >= generation
                && self.texture.is_some())
        {
            return;
        }
        let image = ColorImage::new(
            [320, 240],
            pixels
                .iter()
                .map(|p| Color32::from_rgb((p >> 16) as u8, (p >> 8) as u8, *p as u8))
                .collect(),
        );
        let options = self.texture_options();
        if let Some(tex) = &mut self.texture {
            tex.set(image, options);
        } else {
            self.texture = Some(ctx.load_texture("calc_screen", image, options));
        }
        self.last_generation = generation;
    }
    pub fn paint_at(&mut self, ui: &mut egui::Ui, state: &mut IdeAppState, rect: Rect) {
        self.last_display_rect = rect;
        state.display_scale = rect.width() / 320.0;
        let response = ui.interact(
            rect,
            ui.id().with("lcd_touch"),
            egui::Sense::click_and_drag(),
        );
        if response.is_pointer_button_down_on() {
            state.calc_input_focus = true;
            ui.ctx().memory_mut(|m| {
                if let Some(id) = m.focused() {
                    m.surrender_focus(id);
                }
            });
        }
        ui.painter().rect_filled(rect, 0.0, Color32::BLACK);
        if let Some(texture) = &self.texture {
            ui.painter().image(
                texture.id(),
                rect,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        ui.painter().rect_stroke(
            rect,
            0.0,
            Stroke::new(
                1.0,
                if state.calc_input_focus {
                    ACCENT_BLUE.gamma_multiply(0.6)
                } else {
                    BORDER_CRISP
                },
            ),
            StrokeKind::Outside,
        );
    }
}
impl IdePanel for CalculatorDisplay {
    fn render(&mut self, ui: &mut egui::Ui, state: &mut IdeAppState) {
        let available = ui.available_size();
        let width = available.x.min(available.y * 4.0 / 3.0).max(1.0);
        let size = Vec2::new(width, width * 0.75);
        let rect = Rect::from_center_size(ui.available_rect_before_wrap().center(), size);
        self.paint_at(ui, state, rect);
        ui.allocate_space(available);
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

    #[test]
    fn test_update_xrgb_monotonic_generation() {
        let ctx = egui::Context::default();
        let mut display = CalculatorDisplay::new();
        let pixels1 = vec![0x00FF0000; 320 * 240]; // Red
        display.update_xrgb(&ctx, &pixels1, 10);
        assert_eq!(display.last_generation, 10);
        assert!(display.texture.is_some());

        // Stale frame (generation 5 < 10) must be dropped
        let pixels_old = vec![0x0000FF00; 320 * 240];
        display.update_xrgb(&ctx, &pixels_old, 5);
        assert_eq!(display.last_generation, 10);

        // Same generation (10 == 10) must be skipped (no redundant texture updates)
        display.update_xrgb(&ctx, &pixels_old, 10);
        assert_eq!(display.last_generation, 10);

        // Newer generation (15 > 10) is accepted
        let pixels_new = vec![0x000000FF; 320 * 240];
        display.update_xrgb(&ctx, &pixels_new, 15);
        assert_eq!(display.last_generation, 15);

        // Reset sentinel u64::MAX (used on backend switch) allows any subsequent generation
        display.last_generation = u64::MAX;
        display.update_xrgb(&ctx, &pixels1, 2);
        assert_eq!(display.last_generation, 2);
    }
}
