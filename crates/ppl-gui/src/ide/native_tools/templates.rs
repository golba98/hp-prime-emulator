//! The calculator's centered 6×3 mathematical template palette.
use super::{NativeTools, Page, Request};
use crate::ide::native::{
    expression::{EditCommand, palette::CELLS},
    keys::KeyAction,
};
use egui::{Color32, Rect, Stroke, StrokeKind};
impl NativeTools {
    fn template_index(&self) -> usize {
        CELLS
            .iter()
            .position(|cell| cell.name == self.memory.template_selection)
            .unwrap_or(0)
    }
    pub fn template_action(&mut self, action: KeyAction) -> Option<Vec<Request>> {
        if self.page != Some(Page::Templates) {
            return None;
        }
        let index = self.template_index();
        let row = index / 6;
        let column = index % 6;
        let next = match action {
            KeyAction::History(n) if n < 0 => row.saturating_sub(1) * 6 + column,
            KeyAction::History(_) => (row + 1).min(2) * 6 + column,
            KeyAction::Edit(EditCommand::Left(false)) => row * 6 + column.saturating_sub(1),
            KeyAction::Edit(EditCommand::Right(false)) => row * 6 + (column + 1).min(5),
            KeyAction::Evaluate => return Some(vec![Request::InsertTemplate(CELLS[index].kind)]),
            KeyAction::Fraction => {
                return Some(vec![Request::InsertTemplate(
                    crate::ide::native::expression::TemplateKind::Fraction,
                )]);
            }
            KeyAction::Template(kind) => return Some(vec![Request::InsertTemplate(kind)]),
            _ => return None,
        };
        self.memory.template_selection = CELLS[next].name.into();
        self.dirty();
        Some(Vec::new())
    }
    pub(super) fn show_templates(
        &mut self,
        ctx: &egui::Context,
        lcd: Rect,
        scale: f32,
        requests: &mut Vec<Request>,
    ) {
        if self.template_icons.len() != CELLS.len() {
            self.template_icons.clear();
            for (index, cell) in CELLS.iter().enumerate() {
                let image = crate::ide::native::math_layout::palette_icon(cell.kind, 37, 35);
                let pixels = image
                    .pixels()
                    .iter()
                    .map(|&color| {
                        Color32::from_rgba_unmultiplied(0, 0, 0, 255 - (color & 255) as u8)
                    })
                    .collect();
                let color = egui::ColorImage {
                    size: [37, 35],
                    source_size: egui::vec2(37., 35.),
                    pixels,
                };
                self.template_icons.push(ctx.load_texture(
                    format!("native_math_form_{index}"),
                    color,
                    egui::TextureOptions::NEAREST,
                ));
            }
        }
        let selected = self.template_index();
        let grid = Rect::from_min_max(
            lcd.min + egui::vec2(45., 65.) * scale,
            lcd.min + egui::vec2(275., 176.) * scale,
        );
        egui::Area::new("native_math_palette".into())
            .fixed_pos(lcd.min)
            .order(egui::Order::Foreground)
            .fade_in(false)
            .show(ctx, |ui| {
                ui.set_clip_rect(lcd);
                ui.set_min_size(lcd.size());
                ui.painter()
                    .rect_filled(grid, 0., Color32::from_rgb(232, 241, 246));
                for (index, cell) in CELLS.iter().enumerate() {
                    let column = index % 6;
                    let row = index / 6;
                    let rect = Rect::from_min_max(
                        grid.min + egui::vec2(column as f32 * 230. / 6., row as f32 * 37.) * scale,
                        grid.min
                            + egui::vec2((column + 1) as f32 * 230. / 6., (row + 1) as f32 * 37.)
                                * scale,
                    );
                    if index == selected {
                        ui.painter()
                            .rect_filled(rect, 0., Color32::from_rgb(173, 204, 255));
                    }
                    let icon = Rect::from_center_size(rect.center(), egui::vec2(37., 35.) * scale);
                    ui.painter().image(
                        self.template_icons[index].id(),
                        icon,
                        Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                        Color32::WHITE,
                    );
                    let response = ui
                        .interact(rect, ui.id().with(index), egui::Sense::click())
                        .on_hover_text(cell.name);
                    if response.clicked() {
                        self.memory.template_selection = cell.name.into();
                        self.dirty();
                        requests.push(Request::InsertTemplate(cell.kind));
                    }
                }
                // Thin separators and the outside frame match the G1 palette.
                for column in 1..6 {
                    let x = grid.left() + column as f32 * 230. / 6. * scale;
                    ui.painter().line_segment(
                        [egui::pos2(x, grid.top()), egui::pos2(x, grid.bottom())],
                        Stroke::new(scale, Color32::from_gray(220)),
                    );
                }
                for row in 1..3 {
                    let y = grid.top() + row as f32 * 37. * scale;
                    ui.painter().line_segment(
                        [egui::pos2(grid.left(), y), egui::pos2(grid.right(), y)],
                        Stroke::new(scale, Color32::from_gray(220)),
                    );
                }
                ui.painter().rect_stroke(
                    grid,
                    0.,
                    Stroke::new(scale, Color32::BLACK),
                    StrokeKind::Inside,
                );
                if self.template_strip.is_none() {
                    let mut strip = ppl_graphics::Grob::new(320, 240, 0xffffff);
                    crate::ide::native::chrome::softkeys(&mut strip, &["", "", "", "", "", "OK"]);
                    let pixels = strip.pixels()[220 * 320..]
                        .iter()
                        .map(|&c| Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8))
                        .collect();
                    self.template_strip = Some(ctx.load_texture(
                        "native_template_softkeys",
                        egui::ColorImage::new([320, 20], pixels),
                        egui::TextureOptions::NEAREST,
                    ));
                }
                if let Some(strip) = &self.template_strip {
                    ui.painter().image(
                        strip.id(),
                        Rect::from_min_max(lcd.min + egui::vec2(0., 220.) * scale, lcd.max),
                        Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                        Color32::WHITE,
                    );
                }
                let ok = Rect::from_min_max(lcd.min + egui::vec2(265., 220.) * scale, lcd.max);
                if ui
                    .interact(ok, ui.id().with("ok"), egui::Sense::click())
                    .clicked()
                {
                    requests.push(Request::InsertTemplate(CELLS[selected].kind));
                }
                let outside = ctx.input(|i| {
                    i.pointer.primary_pressed()
                        && i.pointer.interact_pos().is_some_and(|p| {
                            lcd.contains(p) && !grid.contains(p) && !ok.contains(p)
                        })
                });
                if outside {
                    requests.push(Request::Dismiss);
                }
            });
    }
}
