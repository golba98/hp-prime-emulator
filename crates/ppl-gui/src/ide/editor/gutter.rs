//! The line-number gutter: painted over the left edge of the scrolled
//! text so it stays put while the text scrolls horizontally, aligned to
//! the galley's own rows, and limited to the visible rows.

use std::ops::Range;

use egui::emath::GuiRounding;
use egui::{Align2, FontId, Galley, Pos2, Rect, Sense, Stroke, Vec2, pos2};
use ppl_syntax::Severity;

use crate::ide::diagnostics::DiagnosticItem;
use crate::theme::{ACCENT_AMBER, ACCENT_BLUE, ACCENT_RED, editor as colors};

/// Width of the marker column (breakpoints, debugger arrow).
const MARKER_W: f32 = 16.0;
/// Space between the numbers and the text.
const NUMBER_PAD: f32 = 12.0;

/// Gutter width for a buffer with `line_count` lines.
pub fn width(ctx: &egui::Context, font: &FontId, line_count: usize) -> f32 {
    let digits = line_count.max(1).to_string().len().max(3);
    let digit_w = ctx.fonts(|f| f.glyph_width(font, '0'));
    (MARKER_W + digits as f32 * digit_w + NUMBER_PAD).ceil()
}

/// Rows (0-based lines) of `galley` placed at `galley_pos` that intersect
/// `clip`. Code is never wrapped, so row index == line index.
pub fn visible_rows(galley: &Galley, galley_pos: Pos2, clip: Rect) -> Range<usize> {
    let rows = &galley.rows;
    let first = rows.partition_point(|r| galley_pos.y + r.max_y() < clip.min.y);
    let last = rows.partition_point(|r| galley_pos.y + r.min_y() <= clip.max.y);
    first..last.max(first)
}

/// Most severe diagnostic per line among `diags` (sorted by line).
fn severity_on(diags: &[DiagnosticItem], line: usize) -> Option<Severity> {
    let start = diags.partition_point(|d| d.line < line);
    diags[start..]
        .iter()
        .take_while(|d| d.line == line)
        .map(|d| d.severity)
        .min()
}

pub struct GutterInput<'a> {
    pub galley: &'a Galley,
    pub galley_pos: Pos2,
    /// Screen rect of the gutter (visible height).
    pub rect: Rect,
    pub font: FontId,
    pub cursor_line: Option<usize>,
    pub paused_line: usize,
    pub breakpoints: &'a std::collections::BTreeSet<usize>,
    /// Diagnostics sorted by line.
    pub diagnostics: &'a [DiagnosticItem],
}

/// Paints the gutter and returns the 1-based line whose marker area was
/// clicked (to toggle a breakpoint).
pub fn show(ui: &mut egui::Ui, id: egui::Id, g: GutterInput<'_>) -> Option<usize> {
    let painter = ui.painter_at(g.rect);
    let ppp = painter.pixels_per_point();
    let rect = g.rect.round_to_pixels(ppp);
    painter.rect_filled(rect, 0.0, colors::GUTTER_BG);
    painter.line_segment(
        [
            pos2(rect.max.x - 0.5, rect.min.y),
            pos2(rect.max.x - 0.5, rect.max.y),
        ],
        Stroke::new(1.0, crate::theme::BORDER_CRISP.gamma_multiply(0.6)),
    );

    let rows = visible_rows(g.galley, g.galley_pos, g.rect);
    let number_x = rect.max.x - NUMBER_PAD + 2.0;
    for line0 in rows.clone() {
        let row = &g.galley.rows[line0];
        let y0 = g.galley_pos.y + row.min_y();
        let y1 = g.galley_pos.y + row.max_y();
        let mid = ((y0 + y1) / 2.0).round_to_pixels(ppp);
        let line = line0 + 1;
        let is_current = g.cursor_line == Some(line);

        if let Some(sev) = severity_on(g.diagnostics, line) {
            let color = match sev {
                Severity::Error => ACCENT_RED,
                Severity::Warning => ACCENT_AMBER,
                Severity::Note => ACCENT_BLUE,
            };
            let bar = Rect::from_min_max(
                pos2(rect.max.x - 3.0, y0 + 1.0),
                pos2(rect.max.x - 1.0, y1 - 1.0),
            );
            painter.rect_filled(bar.round_to_pixels(ppp), 0.0, color);
        }
        if g.breakpoints.contains(&line) {
            painter.circle_filled(
                pos2(rect.min.x + MARKER_W / 2.0 + 1.0, mid),
                4.5,
                colors::BREAKPOINT,
            );
        }
        if g.paused_line == line {
            let x = rect.min.x + 3.0;
            let arrow = vec![pos2(x, mid - 5.0), pos2(x + 9.0, mid), pos2(x, mid + 5.0)];
            painter.add(egui::Shape::convex_polygon(
                arrow,
                ACCENT_AMBER,
                Stroke::NONE,
            ));
        }
        painter.text(
            pos2(number_x, mid),
            Align2::RIGHT_CENTER,
            line.to_string(),
            g.font.clone(),
            if is_current {
                colors::GUTTER_TEXT_ACTIVE
            } else {
                colors::GUTTER_TEXT
            },
        );
    }

    let resp = ui.interact(g.rect, id, Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if !resp.clicked() {
        return None;
    }
    let pos = resp.interact_pointer_pos()?;
    rows.into_iter().find_map(|line0| {
        let row = &g.galley.rows[line0];
        let r = Rect::from_min_size(
            pos2(g.rect.min.x, g.galley_pos.y + row.min_y()),
            Vec2::new(g.rect.width(), row.height()),
        );
        r.contains(pos).then_some(line0 + 1)
    })
}
