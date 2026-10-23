//! The code editor for one tab: egui `TextEdit` for text input, selection
//! and IME, wrapped with everything a code editor needs around it.
//!
//! Paint order inside the scroll area:
//! 1. background bands (editor fill, current line, paused line, find
//!    matches, bracket pair) into a placeholder reserved *before* the text,
//!    so they sit behind it;
//! 2. the text (`TextEdit`, frameless, laid out by the cached galley);
//! 3. diagnostic squiggles and the gutter, on top.

use std::ops::Range;
use std::sync::Arc;

use egui::emath::GuiRounding;
use egui::text::{CCursor, CCursorRange};
use egui::text_edit::TextEditState;
use egui::{Galley, Id, Pos2, Rect, ScrollArea, Shape, Stroke, TextEdit, pos2, vec2};
use ppl_syntax::Severity;

use super::completion::{self, CompletionItem, CompletionState};
use super::document::LineIndex;
use super::gutter::{self, GutterInput};
use super::input;
use super::tab::{EditorTab, TabId};
use crate::ide::diagnostics::DiagnosticItem;
use crate::ide::syntax::CodeStyle;
use crate::ide::{IdeAction, IdeAppState};
use crate::theme::{ACCENT_AMBER, ACCENT_BLUE, ACCENT_RED, editor as colors};

/// Editor state shared across tabs (one popup / one paused line at a time).
#[derive(Debug, Default)]
pub struct ViewState {
    pub completion: CompletionState,
    candidates: Option<(TabId, u64, Arc<Vec<CompletionItem>>)>,
    last_paused_line: usize,
    last_cursor: Option<(TabId, usize)>,
    /// Geometry of the last frame (tests and automation inspect it).
    pub last_frame: Option<FrameInfo>,
}

/// Where the editor drew its text in the last frame.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameInfo {
    pub tab: TabId,
    /// Screen position of the galley's top-left corner (moves on scroll).
    pub galley_pos: Pos2,
    /// Visible text viewport (screen coordinates).
    pub clip: Rect,
    pub gutter_width: f32,
    /// 0-based lines at least partly visible.
    pub visible_lines: Range<usize>,
    pub row_height: f32,
}

/// The `TextEdit` id of a tab.
pub fn edit_id(tab: TabId) -> Id {
    Id::new(("ppl_editor", tab.0))
}

/// Screen rect of a 0-width caret before char column `col` of 0-based row
/// `line` (rows are lines: code is never wrapped).
fn caret(galley: &Galley, galley_pos: Pos2, line: usize, col: usize) -> Rect {
    let Some(row) = galley.rows.get(line).or(galley.rows.last()) else {
        return Rect::from_min_size(galley_pos, vec2(0.0, 0.0));
    };
    let x = galley_pos.x + row.pos.x + row.x_offset(col);
    Rect::from_min_max(
        pos2(x, galley_pos.y + row.min_y()),
        pos2(x, galley_pos.y + row.max_y()),
    )
}

fn caret_of_char(galley: &Galley, galley_pos: Pos2, lines: &LineIndex, c: usize) -> Rect {
    let (line, col) = lines.line_col(c);
    caret(galley, galley_pos, line - 1, col - 1)
}

fn severity_color(s: Severity) -> egui::Color32 {
    match s {
        Severity::Error => ACCENT_RED,
        Severity::Warning => ACCENT_AMBER,
        Severity::Note => ACCENT_BLUE,
    }
}

fn squiggle(painter: &egui::Painter, x0: f32, x1: f32, y: f32, color: egui::Color32) {
    let (wavelength, amplitude) = (4.0, 1.25);
    let mut points = Vec::new();
    let mut x = x0;
    while x <= x1 {
        let phase = (x - x0) / wavelength * std::f32::consts::TAU;
        points.push(pos2(x, y + amplitude * phase.sin()));
        x += 1.0;
    }
    if points.len() >= 2 {
        painter.add(Shape::line(points, Stroke::new(1.0, color)));
    }
}

/// Find matches to highlight: every match (byte ranges, in order) and
/// the one last navigated to.
pub struct FindHighlight<'a> {
    pub matches: &'a [Range<usize>],
    pub current: Option<Range<usize>>,
}

/// Per-frame inputs from outside the tab.
pub struct ViewInput<'a> {
    pub style: CodeStyle,
    pub find: Option<FindHighlight<'a>>,
}

/// Shows `tab` filling the available space.
pub fn show(
    ui: &mut egui::Ui,
    tab: &mut EditorTab,
    state: &mut IdeAppState,
    view: &mut ViewState,
    input: ViewInput<'_>,
) {
    let ctx = ui.ctx().clone();
    let style = input.style;
    let tab_id = tab.id;
    let id = edit_id(tab_id);
    tab.sync();

    // 1. Editor keys, before TextEdit sees them.
    let focused = ctx.memory(|m| m.has_focus(id));
    let mut explicit_completion = false;
    if focused {
        let lines = tab.doc.lines().clone();
        let analysis = tab.doc.analysis().clone();
        let r = input::intercept(
            ui,
            id,
            &mut tab.content,
            &lines,
            &analysis,
            &mut view.completion,
        );
        explicit_completion = r.complete;
        if r.edited {
            tab.sync();
        }
    } else if view.completion.open {
        view.completion.close();
    }

    // 2. Pending cursor moves (go to line, problems, find).
    let mut scroll_to: Option<usize> = None;
    if let Some(idx) = tab.pending_jump.take() {
        let mut st = TextEditState::load(&ctx, id).unwrap_or_default();
        st.cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(idx))));
        st.store(&ctx, id);
        ctx.memory_mut(|m| m.request_focus(id));
        scroll_to = Some(idx);
    }
    if let Some((a, b)) = tab.pending_select.take() {
        let mut st = TextEditState::load(&ctx, id).unwrap_or_default();
        st.cursor
            .set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
        st.store(&ctx, id);
        scroll_to = Some(b);
    }

    let lines = tab.doc.lines().clone();
    let analysis = tab.doc.analysis().clone();
    let line_count = lines.line_count();
    let number_font = egui::FontId::monospace((style.font_size - 1.0).max(8.0));
    let gutter_w = gutter::width(&ctx, &number_font, line_count);
    let outer = ui.available_rect_before_wrap();
    let paused_line = state.paused_line;
    let paused_changed = paused_line != view.last_paused_line;
    view.last_paused_line = paused_line;

    let mut gutter_click = None;
    let mut cursor_char: Option<usize> = None;
    let mut typed_word = false;
    let mut caret_rect: Option<Rect> = None;

    let EditorTab {
        content,
        doc,
        diagnostics,
        breakpoints,
        ..
    } = tab;

    ScrollArea::both()
        .id_salt(("ppl_editor_scroll", id))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let clip = ui.clip_rect();
            let bg_idx = ui.painter().add(Shape::Noop);

            let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, _wrap: f32| {
                doc.galley(ui.ctx(), buf.as_str(), style)
            };
            let output = ui
                .horizontal_top(|ui| {
                    ui.add_space(gutter_w);
                    TextEdit::multiline(content)
                        .id(id)
                        .font(style.font())
                        .frame(false)
                        .margin(egui::Margin {
                            left: 8,
                            right: 32,
                            top: 6,
                            bottom: 48,
                        })
                        .desired_width(f32::INFINITY)
                        .min_size(vec2(0.0, clip.height()))
                        .lock_focus(true)
                        .layouter(&mut layouter)
                        .show(ui)
                })
                .inner;
            let galley = output.galley.clone();
            let gpos = output.galley_pos;
            let has_focus = output.response.has_focus();
            let rows = gutter::visible_rows(&galley, gpos, clip);
            let text_left = clip.min.x + gutter_w;
            view.last_frame = Some(FrameInfo {
                tab: tab_id,
                galley_pos: gpos,
                clip,
                gutter_width: gutter_w,
                visible_lines: rows.clone(),
                row_height: galley.rows.first().map_or(0.0, |r| r.height()),
            });

            // Keystrokes that changed the text through TextEdit itself.
            if output.response.changed() {
                typed_word = ui.input(|i| {
                    i.events.iter().any(|e| {
                        matches!(e, egui::Event::Text(t)
                            if t.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.'))
                    })
                });
            }

            // --- background (behind the text) ---------------------------
            let mut bg = vec![Shape::rect_filled(clip, 0.0, colors::BG)];
            let band = |line0: usize| -> Option<Rect> {
                let row = galley.rows.get(line0)?;
                Some(
                    Rect::from_min_max(
                        pos2(text_left, gpos.y + row.min_y()),
                        pos2(clip.max.x, gpos.y + row.max_y()),
                    )
                    .round_to_pixels(ui.pixels_per_point()),
                )
            };
            if let Some(range) = output.cursor_range {
                let c = range.primary.index;
                cursor_char = Some(c);
                let (line, _) = lines.line_col(c);
                if has_focus && range.is_empty() {
                    if let Some(r) = band(line - 1) {
                        bg.push(Shape::rect_filled(r, 0.0, colors::CURRENT_LINE));
                    }
                }
                caret_rect = Some(caret_of_char(&galley, gpos, &lines, c));
            }
            if paused_line > 0 {
                if let Some(r) = band(paused_line - 1) {
                    bg.push(Shape::rect_filled(r, 0.0, colors::PAUSED_LINE));
                    bg.push(Shape::rect_filled(
                        Rect::from_min_size(r.min, vec2(3.0, r.height())),
                        0.0,
                        ACCENT_AMBER,
                    ));
                }
            }
            if let Some(FindHighlight { matches, current }) = input.find {
                let first_b = lines.line_bytes(rows.start).start;
                let last_b = lines.line_bytes(rows.end.saturating_sub(1)).end;
                let i0 = matches.partition_point(|m| m.end < first_b);
                for m in matches[i0..].iter().take_while(|m| m.start <= last_b) {
                    let a =
                        caret_of_char(&galley, gpos, &lines, lines.char_of_byte(content, m.start));
                    let b =
                        caret_of_char(&galley, gpos, &lines, lines.char_of_byte(content, m.end));
                    if (a.min.y - b.min.y).abs() > 0.5 {
                        continue;
                    }
                    let r = Rect::from_min_max(a.min, pos2(b.max.x.max(a.min.x + 2.0), a.max.y));
                    let is_current = current.as_ref() == Some(m);
                    bg.push(Shape::rect_filled(r, 2.0, colors::FIND_MATCH));
                    if is_current {
                        bg.push(Shape::rect_stroke(
                            r,
                            2.0,
                            Stroke::new(1.0, ACCENT_AMBER),
                            egui::StrokeKind::Inside,
                        ));
                    }
                }
            }
            if let (Some(c), true) = (cursor_char, has_focus) {
                let byte = lines.byte_of_char(content, c);
                let pair = analysis.bracket_partner(byte).or_else(|| {
                    byte.checked_sub(1)
                        .filter(|&b| content.is_char_boundary(b))
                        .and_then(|b| analysis.bracket_partner(b))
                });
                if let Some((at, partner)) = pair {
                    for b in std::iter::once(at).chain(partner) {
                        let ch = lines.char_of_byte(content, b);
                        let a = caret_of_char(&galley, gpos, &lines, ch);
                        let z = caret_of_char(&galley, gpos, &lines, ch + 1);
                        let r =
                            Rect::from_min_max(a.min, pos2(z.max.x.max(a.min.x + 6.0), a.max.y));
                        let color = if partner.is_some() {
                            colors::BRACKET_MATCH
                        } else {
                            crate::theme::syntax::INVALID_BG
                        };
                        bg.push(Shape::rect_filled(r, 2.0, color));
                    }
                }
            }
            ui.painter().set(bg_idx, Shape::Vec(bg));

            // --- diagnostics (on top) ------------------------------------
            let painter = ui.painter_at(Rect::from_min_max(pos2(text_left, clip.min.y), clip.max));
            let first_line = rows.start + 1;
            let last_line = rows.end;
            let i0 = diagnostics.partition_point(|d| d.line < first_line);
            let hover = output.response.hover_pos();
            let mut hovered: Option<&DiagnosticItem> = None;
            for d in diagnostics[i0..].iter().take_while(|d| d.line <= last_line) {
                let s = lines.char_of_byte(content, d.span.start as usize);
                let e = lines.char_of_byte(content, d.span.end as usize).max(s + 1);
                let a = caret_of_char(&galley, gpos, &lines, s);
                let (el, _) = lines.line_col(e);
                let x1 = if el == lines.line_col(s).0 {
                    caret_of_char(&galley, gpos, &lines, e).max.x
                } else {
                    let row = &galley.rows[(el - 1).min(galley.rows.len() - 1)];
                    gpos.x + row.pos.x + row.size.x
                };
                let x1 = x1.max(a.min.x + 6.0);
                squiggle(
                    &painter,
                    a.min.x,
                    x1,
                    a.max.y - 2.0,
                    severity_color(d.severity),
                );
                if let Some(p) = hover {
                    let r = Rect::from_min_max(pos2(a.min.x, a.min.y), pos2(x1, a.max.y));
                    if r.contains(p) && hovered.is_none_or(|h| d.severity < h.severity) {
                        hovered = Some(d);
                    }
                }
            }
            if let Some(d) = hovered {
                egui::Tooltip::always_open(
                    ui.ctx().clone(),
                    ui.layer_id(),
                    id.with("diagnostic"),
                    egui::PopupAnchor::Pointer,
                )
                .gap(10.0)
                .show(|ui: &mut egui::Ui| diagnostic_tooltip(ui, d));
            }

            // --- scrolling ---------------------------------------------
            let row_pad = vec2(gutter_w + 24.0, style.row_height() * 2.0);
            if let Some(c) = scroll_to {
                let r = caret_of_char(&galley, gpos, &lines, c);
                ui.scroll_to_rect(r.expand2(row_pad), Some(egui::Align::Center));
            } else if paused_changed && paused_line > 0 {
                let r = caret(&galley, gpos, paused_line - 1, 0);
                ui.scroll_to_rect(r.expand2(row_pad), Some(egui::Align::Center));
            } else if let (Some(r), true) = (caret_rect, has_focus) {
                // Keep the caret clear of the gutter after it moves left.
                let moved = view.last_cursor != cursor_char.map(|c| (tab_id, c));
                if moved && r.min.x < text_left + 4.0 && r.max.x >= clip.min.x {
                    ui.scroll_to_rect(r.expand2(vec2(gutter_w + 16.0, 0.0)), None);
                }
            }
            view.last_cursor = cursor_char.map(|c| (tab_id, c));

            // --- gutter (on top, fixed horizontally) ----------------------
            gutter_click = gutter::show(
                ui,
                id.with("gutter"),
                GutterInput {
                    galley: &galley,
                    galley_pos: gpos,
                    rect: Rect::from_min_size(clip.min, vec2(gutter_w, clip.height())),
                    font: number_font.clone(),
                    cursor_line: cursor_char.map(|c| lines.line_col(c).0),
                    paused_line,
                    breakpoints,
                    diagnostics,
                },
            );
        });

    // Focus indicator.
    let focused_now = ctx.memory(|m| m.has_focus(id));
    if focused_now {
        state.calc_input_focus = false;
        ui.painter().rect_stroke(
            outer.round_to_pixels(ui.pixels_per_point()),
            0.0,
            Stroke::new(1.0, colors::FOCUS_BORDER.gamma_multiply(0.7)),
            egui::StrokeKind::Inside,
        );
    }
    if let Some(c) = cursor_char {
        let (l, col) = lines.line_col(c);
        state.cursor_line = l;
        state.cursor_col = col;
    }
    if let Some(line) = gutter_click {
        state.queue_action(IdeAction::ToggleBreakpoint(line));
    }

    // Completion.
    if focused_now && (typed_word || explicit_completion) {
        if let Some(c) = cursor_char {
            let revision = tab.revision();
            let fresh = view
                .candidates
                .as_ref()
                .is_some_and(|(t, r, _)| *t == tab.id && *r == revision);
            if !fresh {
                let items = completion::candidates(&tab.doc.analysis().symbols);
                view.candidates = Some((tab.id, revision, Arc::new(items)));
            }
            let kind = tab.doc.analysis().kind_at(
                tab.doc
                    .lines()
                    .byte_of_char(&tab.content, c)
                    .saturating_sub(1),
            );
            let in_literal = matches!(
                kind,
                Some(
                    ppl_compiler::highlight::TokenKind::String
                        | ppl_compiler::highlight::TokenKind::Comment
                        | ppl_compiler::highlight::TokenKind::InvalidString
                )
            );
            if in_literal && !explicit_completion {
                view.completion.close();
            } else if let Some((_, _, items)) = &view.candidates {
                input::update_completion(
                    &mut view.completion,
                    items,
                    &tab.content,
                    c,
                    explicit_completion,
                );
            }
        }
    } else if view.completion.open
        && view
            .last_cursor
            .is_some_and(|(_, c)| c < view.completion.word_start)
    {
        view.completion.close();
    }
    if view.completion.open {
        if let Some(r) = caret_rect {
            completion_popup(
                ui,
                id,
                &mut tab.content,
                &mut view.completion,
                r.left_bottom(),
            );
        }
    }
}

fn diagnostic_tooltip(ui: &mut egui::Ui, d: &DiagnosticItem) {
    ui.set_max_width(460.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("{} · {}", d.class.label(), d.severity_label()))
                .color(severity_color(d.severity))
                .strong(),
        );
        ui.label(egui::RichText::new(&d.code).monospace().weak());
    });
    ui.label(&d.message);
    if let Some(h) = &d.hint {
        ui.label(egui::RichText::new(format!("hint: {h}")).weak());
    }
    if d.unverified {
        ui.label(
            egui::RichText::new("rule not yet verified on a calculator")
                .weak()
                .italics(),
        );
    }
}

fn completion_popup(
    ui: &mut egui::Ui,
    id: Id,
    text: &mut String,
    completion: &mut CompletionState,
    at: Pos2,
) {
    let ctx = ui.ctx().clone();
    let mut accept = None;
    egui::Area::new(id.with("completion"))
        .order(egui::Order::Foreground)
        .fixed_pos(at + vec2(0.0, 2.0))
        .show(&ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(260.0);
                for (i, item) in completion.items.iter().enumerate() {
                    let text = egui::RichText::new(format!("{:<18} {}", item.label, item.detail))
                        .monospace()
                        .size(12.0);
                    let r = ui.selectable_label(i == completion.selected, text);
                    if r.clicked() {
                        accept = Some(i);
                    }
                }
            });
        });
    if let Some(i) = accept {
        completion.selected = i;
        input::accept_completion(&ctx, id, text, completion);
        ctx.memory_mut(|m| m.request_focus(id));
    }
}
