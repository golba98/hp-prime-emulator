//! Home/CAS history selection is independent of the entry document.
//! HP Prime User Guide 813269-002 pp. 27–29, 48: Copy inserts at the cursor;
//! Show inspects large results without replacing either the draft or Ans.
use super::{HomeEntry, NativeUiState, expression::ExpressionEditor, math_layout};
use ppl_graphics::{Grob, SCREEN_W};
use ppl_vm::host::Touch;

const SHOW_TOP: i64 = 24;
const SHOW_BOTTOM: i64 = 220;
const SHOW_HEIGHT: i64 = SHOW_BOTTOM - SHOW_TOP;
const SHOW_SIDE_MARGIN: i64 = 10;
const SHOW_VERTICAL_MARGIN: i64 = 6;
const SHOW_CONTENT_WIDTH: i64 = SCREEN_W as i64 - 2 * SHOW_SIDE_MARGIN;
const SHOW_CONTENT_HEIGHT: i64 = SHOW_HEIGHT - 2 * SHOW_VERTICAL_MARGIN;

#[derive(Debug, Clone, Default)]
pub struct HistoryViewer {
    pub x: i64,
    pub y: i64,
    drag_point: Option<Touch>,
}
pub struct HistoryRow {
    pub index: usize,
    pub top: i64,
    pub height: i64,
    expression: math_layout::MathLayout,
    result: math_layout::MathLayout,
    result_x: i64,
    result_y: i64,
    result_below_expression: bool,
    content_y: i64,
}
impl NativeUiState {
    pub fn history_rows(&self) -> Vec<HistoryRow> {
        let end = self
            .history_position
            .map_or(self.history.len(), |p| (p + 1).min(self.history.len()));
        let entry_top = self.entry_top();
        let mut bottom = entry_top - 2;
        let mut rows = Vec::new();
        for index in (0..end).rev() {
            let entry = &self.history[index];
            let fraction_grids = if self.cas_mode
                && self.presentation.textbook_display
                && !entry.error
                && !entry.expression_deleted
                && !entry.result_deleted
            {
                math_layout::history_fraction_grids(
                    entry.display_expression(),
                    entry.display_result(),
                    entry.typed_result.as_ref(),
                    self.history_font_px,
                )
            } else {
                None
            };
            let (expression, result) = fraction_grids.unwrap_or_else(|| {
                let layout_expression = |compact_matrices| {
                    if entry.expression_deleted {
                        math_layout::text("", 12.)
                    } else if self.presentation.textbook_display {
                        if entry.error {
                            math_layout::expression(
                                entry.display_expression(),
                                self.history_font_px,
                            )
                        } else {
                            math_layout::history_row_expression(
                                entry.display_expression(),
                                self.history_font_px,
                                self.history_font_px,
                                (!self.cas_mode).then_some(self.history_digit_grouping),
                                compact_matrices,
                            )
                        }
                    } else {
                        math_layout::text(entry.display_expression(), self.history_font_px)
                    }
                };
                let mut expression = layout_expression(true);
                let result = if entry.result_deleted {
                    math_layout::text("", 13.)
                } else if entry.error || !self.presentation.textbook_display {
                    math_layout::text(entry.display_result(), self.history_font_px)
                } else {
                    math_layout::history_result(
                        entry.display_result(),
                        entry.typed_result.as_ref(),
                        self.history_font_px,
                        expression.compact_numeric_grid,
                    )
                };
                if expression.compact_numeric_grid && !result.compact_numeric_grid {
                    expression = layout_expression(false);
                }
                (expression, result)
            });
            let separate = expression.width + result.width + 20 > 304;
            let result_y = if separate {
                expression.height + 3
            } else {
                expression.history_result_offset(&result)
            };
            let height = (expression.height.max(result_y + result.height) + 8).clamp(26, 150);
            if !rows.is_empty() && bottom - height < super::chrome::HEADER_BOTTOM + 2 {
                break;
            }
            let top = (bottom - height).max(super::chrome::HEADER_BOTTOM + 2);
            let pair_matrices_at_entry = index + 1 == self.history.len()
                && !separate
                && expression.is_matrix_grid
                && result.is_matrix_grid
                && expression.height == result.height
                && expression.height <= bottom - top;
            // The last matrix rails can reach the entry divider. Keep the
            // allocated top so older rows and their hit regions stay fixed.
            let row_end = if pair_matrices_at_entry {
                entry_top
            } else {
                bottom
            };
            let content_y = if pair_matrices_at_entry {
                row_end - top - expression.height
            } else if self.cas_mode
                && index + 1 == self.history.len()
                && !separate
                && !entry.result_deleted
                && let Some(baseline) = expression.history_power_baseline(&result)
            {
                // Place the shared numeric baseline against the entry
                // boundary while keeping the allocated history rows.
                (bottom - top - baseline).max(0)
            } else if expression.height <= 18 && result_y == 0 && result.height <= 18 {
                // The ordinary history line sits against the entry divider.
                // The renderer includes three pixels of leading in its box.
                let leading = if expression.includes_matrix || result.includes_matrix {
                    0
                } else {
                    3
                };
                (bottom - top - expression.height.max(result.height) + leading).max(0)
            } else {
                2
            };
            rows.push(HistoryRow {
                index,
                top,
                height: row_end - top,
                expression,
                result_x: if result.width > 304 {
                    1
                } else {
                    319 - result.width
                },
                result,
                result_y,
                result_below_expression: separate,
                content_y,
            });
            bottom = top;
            if bottom <= super::chrome::HEADER_BOTTOM + 2 {
                break;
            }
        }
        rows.reverse();
        rows
    }
    pub fn history_hit_test(&self, x: i64, y: i64) -> Option<(usize, bool)> {
        self.history_rows()
            .iter()
            .find(|r| y >= r.top && y < r.top + r.height)
            .map(|r| {
                let result = if r.result_below_expression {
                    y >= r.top + r.content_y + r.result_y
                } else {
                    x >= r.result_x - 8
                };
                let entry = &self.history[r.index];
                (
                    r.index,
                    if entry.expression_deleted {
                        true
                    } else if entry.result_deleted {
                        false
                    } else {
                        result
                    },
                )
            })
    }
    pub fn paint_history(&self, screen: &mut Grob) {
        for row in self.history_rows() {
            let mut area = Grob::new(320, row.height as usize, 0xffffff);
            let entry = &self.history[row.index];
            if self.history_position == Some(row.index) {
                let (x, y, w, h) = if self.history_result {
                    (
                        row.result_x,
                        row.content_y + row.result_y,
                        row.result.width,
                        row.result.height,
                    )
                } else {
                    (
                        1,
                        row.content_y,
                        row.expression.width,
                        row.expression.height,
                    )
                };
                area.fill_rect(x - 2, y, x + w + 2, y + h, 0xb7d4f6);
            }
            row.expression.paint_static(&mut area, 1, row.content_y, 0);
            row.result.paint_static(
                &mut area,
                row.result_x,
                row.content_y + row.result_y,
                if entry.error { 0xaa2222 } else { 0 },
            );
            screen.blit(
                (0, row.top, 320, row.top + row.height),
                &area,
                (0, 0, 320, row.height),
                None,
                255,
            );
        }
    }
    fn history_cells(&self) -> Vec<(usize, bool)> {
        self.history
            .iter()
            .enumerate()
            .flat_map(|(i, e)| {
                [(i, false), (i, true)].into_iter().filter(|(_, result)| {
                    if *result {
                        !e.result_deleted
                    } else {
                        !e.expression_deleted
                    }
                })
            })
            .collect()
    }
    pub fn delete_history_item(&mut self) {
        let Some(index) = self.history_position else {
            return;
        };
        let ordinal = self
            .history_cells()
            .iter()
            .position(|&(i, r)| i == index && r == self.history_result)
            .unwrap_or(0);
        let entry = &mut self.history[index];
        if self.history_result {
            entry.result_deleted = true;
        } else {
            entry.expression_deleted = true;
        }
        if entry.result_deleted && entry.expression_deleted {
            self.history.remove(index);
        }
        let cells = self.history_cells();
        if let Some(&(i, r)) =
            cells.get(ordinal.saturating_sub(1).min(cells.len().saturating_sub(1)))
        {
            self.history_position = Some(i);
            self.history_result = r;
        } else {
            self.history_position = None;
        }
        self.touch_view();
    }
    pub fn history_move(&mut self, delta: i8) {
        let cells = self.history_cells();
        if cells.is_empty() {
            return;
        }
        let current = self.history_position.and_then(|i| {
            cells
                .iter()
                .position(|&(p, r)| p == i && r == self.history_result)
        });
        let next = if delta < 0 {
            current.map_or(cells.len() - 1, |p| p.saturating_sub(1))
        } else {
            let Some(p) = current else {
                return;
            };
            if p + 1 == cells.len() {
                self.history_position = None;
                self.touch_view();
                return;
            }
            p + 1
        };
        let (i, r) = cells[next];
        self.history_position = Some(i);
        self.history_result = r;
        self.touch_view();
    }
    pub fn history_jump(&mut self, first: bool) {
        let cells = self.history_cells();
        if let Some(&(i, r)) = if first { cells.first() } else { cells.last() } {
            self.history_position = Some(i);
            self.history_result = r;
            self.touch_view();
        }
    }
    pub fn history_text(&self) -> Option<&str> {
        let entry = self.history.get(self.history_position?)?;
        Some(if self.history_result {
            entry.recall_result()
        } else {
            &entry.expression
        })
    }
    pub fn cas_simplify_operand(&self) -> Option<String> {
        let (entry, result) = if let Some(index) = self.history_position {
            (self.history.get(index)?, self.history_result)
        } else {
            let latest = self
                .history
                .iter()
                .rev()
                .find(|entry| !entry.error && !entry.result_deleted)?;
            (latest, true)
        };
        if entry.error {
            return None;
        }
        if result && !entry.result_deleted {
            let value = entry
                .typed_result
                .as_ref()
                .and_then(|typed| serde_json::from_value::<ppl_cas::CasValue>(typed.clone()).ok())
                .and_then(|value| value.try_expression().ok());
            Some(value.unwrap_or_else(|| entry.result.clone()))
        } else if !result && !entry.expression_deleted {
            ExpressionEditor::from_text(&entry.expression)
                .evaluation_text(true)
                .ok()
        } else {
            None
        }
    }

    pub fn copy_history_to_entry(&mut self) {
        if let Some(text) = self.history_text().map(str::to_owned) {
            self.edit(|e| e.insert_source(&text));
        }
    }
    pub fn show_history(&mut self) {
        if self.history_position.is_none() {
            self.history_jump(false);
        }
        if self.history_text().is_some() {
            self.history_viewer = Some(HistoryViewer::default());
            self.touch_view();
        }
    }
    pub fn history_layout(&self) -> math_layout::MathLayout {
        let entry = self.history_position.and_then(|p| self.history.get(p));
        let layout = match entry {
            Some(e) if self.history_result && !e.error => math_layout::result_with_matrix_font(
                e.display_result(),
                e.typed_result.as_ref(),
                17.,
                self.history_font_px,
            ),
            Some(entry) if !self.history_result => {
                math_layout::history_expression(&entry.expression, 17., self.history_font_px, None)
            }
            _ => math_layout::text(self.history_text().unwrap_or(""), 17.),
        };
        // Keep scrolling for grids that overflow under the ordinary metrics.
        if !layout.is_matrix_grid
            || layout.width > SHOW_CONTENT_WIDTH
            || layout.height > SHOW_CONTENT_HEIGHT
        {
            return layout;
        }
        entry
            .and_then(|entry| self.compact_show_matrix(entry))
            .filter(|compact| {
                compact.height < layout.height
                    && (compact.width == layout.width
                        || (compact.compact_numeric_grid && compact.width <= SHOW_CONTENT_WIDTH))
            })
            .unwrap_or(layout)
    }
    fn compact_show_matrix(&self, entry: &HomeEntry) -> Option<math_layout::MathLayout> {
        if self.history_font_px != 14.
            || !self.presentation.textbook_display
            || entry.error
            || entry.expression_deleted
            || entry.result_deleted
        {
            return None;
        }
        let typed = entry.typed_result.as_ref()?;
        let cols = typed["cols"]
            .as_u64()
            .or_else(|| typed["value"]["cols"].as_u64());
        if typed["type"] != "matrix" || cols != Some(2) {
            return None;
        }
        let px = self.history_font_px;
        if self.cas_mode
            && let Some((source, result)) = math_layout::history_fraction_grids(
                &entry.expression,
                entry.display_result(),
                Some(typed),
                px,
            )
        {
            return Some(if self.history_result { result } else { source });
        }
        let source = math_layout::show_matrix_expression(&entry.expression, px);
        if !source.compact_numeric_grid {
            return None;
        }
        let result = math_layout::show_matrix_result(entry.display_result(), typed, px);
        result
            .compact_numeric_grid
            .then_some(if self.history_result { result } else { source })
    }
    pub fn drag_history_viewer(&mut self, touch: Option<Touch>, pressed: bool) {
        let Some(viewer) = &mut self.history_viewer else {
            return;
        };
        if pressed {
            viewer.drag_point = touch.filter(|p| (SHOW_TOP..SHOW_BOTTOM).contains(&p.y));
        } else if let Some((previous, current)) = viewer.drag_point.take().zip(touch) {
            viewer.drag_point = Some(current);
            self.scroll_history_viewer(previous.x - current.x, previous.y - current.y);
        }
    }
    pub fn scroll_history_viewer(&mut self, dx: i64, dy: i64) {
        if self.history_viewer.is_none() || (dx == 0 && dy == 0) {
            return;
        }
        let layout = self.history_layout();
        if let Some(v) = &mut self.history_viewer {
            let x =
                v.x.saturating_add(dx)
                    .clamp(0, (layout.width - SHOW_CONTENT_WIDTH).max(0));
            let y =
                v.y.saturating_add(dy)
                    .clamp(0, (layout.height - SHOW_CONTENT_HEIGHT).max(0));
            if (v.x, v.y) != (x, y) {
                (v.x, v.y) = (x, y);
                self.touch_view();
            }
        }
    }
    pub fn paint_history_viewer(&self, screen: &mut Grob, viewer: &HistoryViewer) {
        let layout = self.history_layout();
        let fits_width = layout.width <= SHOW_CONTENT_WIDTH;
        let fits_height = layout.height <= SHOW_CONTENT_HEIGHT;
        let centered =
            fits_width && fits_height && (layout.includes_matrix || layout.centers_in_show);
        // Fitting textbook objects open on the original titleless Show LCD.
        let top = if centered { 0 } else { SHOW_TOP };
        let height = SHOW_BOTTOM - top;
        let x = if centered {
            (SCREEN_W as i64 - 1 - layout.width) / 2
        } else if layout.includes_matrix && fits_width {
            (SCREEN_W as i64 - layout.width) / 2
        } else {
            SHOW_SIDE_MARGIN - viewer.x
        };
        let y = if centered {
            (height - 1 - layout.height) / 2
        } else if layout.includes_matrix && fits_height {
            (SHOW_HEIGHT - layout.height) / 2
        } else {
            SHOW_VERTICAL_MARGIN - viewer.y
        };
        let mut viewport = Grob::new(SCREEN_W, height as usize, 0xffffff);
        layout.paint_static(&mut viewport, x, y, 0x111111);
        screen.blit(
            (0, top, SCREEN_W as i64, SHOW_BOTTOM),
            &viewport,
            (0, 0, SCREEN_W as i64, height),
            None,
            255,
        );
        paint_softkeys(screen, &["", "", "", "", "", "OK"]);
    }
}
pub fn paint_softkeys(screen: &mut Grob, labels: &[&str; 6]) {
    super::chrome::softkeys(screen, labels);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fitted_two_column_show_reuses_compact_cells_and_retains_overflow() {
        use serde_json::json;
        let advance = math_layout::text("2", 14.).width;
        for (rows, cols, expected_bounds) in [
            (2, 2, (2 * advance + 19, 36)),
            (3, 1, (advance + 10, 59)),
            (9, 2, (2 * advance + 15, 185)),
        ] {
            let values = vec![json!({"type": "integer", "value": "2"}); rows * cols];
            let row = if cols == 2 { "[2,2]" } else { "[2]" };
            let source = format!("[{}]", vec![row; rows].join(","));
            let mut state = NativeUiState {
                history_font_px: 14.,
                cas_mode: true,
                ..NativeUiState::default()
            };
            state.add_history(
                source.clone(),
                source,
                false,
                Some(json!({"type":"matrix", "value":{"rows":rows,"cols":cols,"values":values}})),
            );
            state.history_move(-1);
            let result = state.history_layout();
            assert_eq!((result.width, result.height), expected_bounds);
            state.history_move(-1);
            let source = state.history_layout();
            assert_eq!((source.width, source.height), expected_bounds);
        }
    }

    #[test]
    fn fitting_structured_show_uses_the_titleless_body() {
        for (source, centered) in [
            ("[[1,2],[3,4]]", true),
            ("({1,2})", true),
            ("sin((x+1)/(x+2))", true),
            ("sin(x)", false),
            ("42", false),
            ("\"{1,2}\"", false),
        ] {
            let mut state = NativeUiState::default();
            state.add_history(source.into(), "1".into(), false, None);
            state.history_move(-1);
            state.history_move(-1);
            state.show_history();
            assert_eq!(state.history_text(), Some(source));
            let mut lcd = Grob::new(SCREEN_W, 240, 0x112233);
            state.paint_history_viewer(&mut lcd, state.history_viewer.as_ref().unwrap());
            let expected = if centered { 0xffffff } else { 0x112233 };
            assert_eq!(lcd.get(0, 0), Some(expected), "{source}");
            assert_eq!(lcd.get(0, SHOW_TOP), Some(0xffffff), "{source}");
        }
    }

    #[test]
    fn cas_simplify_reads_exact_result_by_default_and_honors_selected_source() {
        let mut state = NativeUiState::default();
        assert!(state.cas_simplify_operand().is_none());
        state.add_history(
            "1/3".into(),
            "0.333".into(),
            false,
            Some(serde_json::json!({
                "type": "rational", "value": {"numerator": "1", "denominator": "3"}
            })),
        );
        state.add_history(
            "2+2".into(),
            "4".into(),
            false,
            Some(serde_json::json!({
                "type": "integer", "value": "4"
            })),
        );
        assert_eq!(state.cas_simplify_operand().as_deref(), Some("4"));
        state.history_position = Some(0);
        state.history_result = true;
        assert_eq!(state.cas_simplify_operand().as_deref(), Some("(1)/(3)"));
        state.history_result = false;
        assert_eq!(state.cas_simplify_operand().as_deref(), Some("1/3"));
        state.history[0].expression_deleted = true;
        assert!(state.cas_simplify_operand().is_none());
    }

    #[test]
    fn deleting_a_result_keeps_its_expression_draft_and_other_calculations() {
        let mut s = NativeUiState::default();
        s.add_history("1+2".into(), "3".into(), false, None);
        s.edit(|e| e.set_text("4*"));
        s.history_move(-1);
        s.delete_history_item();
        assert_eq!(s.history.len(), 1);
        assert!(s.history[0].result_deleted);
        assert_eq!(s.history_text(), Some("1+2"));
        assert_eq!(s.home_input, "4*");
        s.history_move(1);
        assert!(s.history_position.is_none());
        s.history_move(-1);
        assert_eq!(s.history_text(), Some("1+2"));
        s.delete_history_item();
        assert!(s.history.is_empty());
        assert_eq!(s.home_input, "4*");
    }
    #[test]
    fn selection_and_show_preserve_entry_and_copy_inserts_at_cursor() {
        let mut s = NativeUiState::default();
        s.add_history("1+2".into(), "3".into(), false, None);
        s.edit(|e| {
            e.set_text("4*");
        });
        s.history_move(-1);
        assert_eq!(s.home_input, "4*");
        assert_eq!(s.history_text(), Some("3"));
        s.show_history();
        s.scroll_history_viewer(300, 300);
        assert_eq!(s.home_input, "4*");
        s.history_viewer = None;
        s.copy_history_to_entry();
        assert_eq!(s.home_input, "4*3");
        assert!(s.history_position.is_none());
        s.history_move(-1);
        s.history_move(-1);
        assert_eq!(s.history_text(), Some("1+2"));
        s.history_move(1);
        s.history_move(1);
        assert!(s.history_position.is_none());
        assert_eq!(s.home_input, "4*3");
    }
    #[test]
    fn recalled_calculus_preserves_outline_and_rebuilds_optional_fields() {
        use crate::ide::native::expression::{TemplateKind, palette::Form};
        let mut s = NativeUiState::default();
        s.add_history("CAS.integrate(x,x,,)".into(), "x^2/2".into(), false, None);
        s.history_move(-1);
        s.history_move(-1);
        s.copy_history_to_entry();
        assert_eq!(s.home_input, "CAS.integrate(x,x,,)");
        let t = s
            .editor
            .templates()
            .iter()
            .find(|t| t.kind == TemplateKind::Palette(Form::Integral))
            .unwrap();
        assert_eq!(t.fields.len(), 4);
        assert_eq!(
            s.editor.evaluation_text(false).unwrap(),
            "CAS.integrate(x,x)"
        );
    }
}
