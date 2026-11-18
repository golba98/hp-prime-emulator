//! Home/CAS history selection is independent of the entry document.
//! HP Prime User Guide 813269-002 pp. 27–29, 48: Copy inserts at the cursor;
//! Show inspects large results without replacing either the draft or Ans.
use super::{NativeUiState, expression::ExpressionEditor, math_layout};
use ppl_graphics::Grob;
use ppl_vm::host::Touch;

const SHOW_TOP: i64 = 24;
const SHOW_BOTTOM: i64 = 220;

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
    content_y: i64,
}
impl NativeUiState {
    pub fn history_rows(&self) -> Vec<HistoryRow> {
        let end = self
            .history_position
            .map_or(self.history.len(), |p| (p + 1).min(self.history.len()));
        let mut bottom = self.entry_top() - 2;
        let mut rows = Vec::new();
        for index in (0..end).rev() {
            let entry = &self.history[index];
            let expression = if entry.expression_deleted {
                math_layout::text("", 12.)
            } else if self.presentation.textbook_display {
                math_layout::expression(entry.display_expression(), self.history_font_px)
            } else {
                math_layout::text(entry.display_expression(), self.history_font_px)
            };
            let result = if entry.result_deleted {
                math_layout::text("", 13.)
            } else if entry.error || !self.presentation.textbook_display {
                math_layout::text(entry.display_result(), self.history_font_px)
            } else {
                math_layout::result(
                    entry.display_result(),
                    entry.typed_result.as_ref(),
                    self.history_font_px,
                )
            };
            let separate = expression.width + result.width + 20 > 304;
            let result_y = if separate { expression.height + 3 } else { 0 };
            let height = (expression.height.max(result_y + result.height) + 8).clamp(26, 150);
            if !rows.is_empty() && bottom - height < super::chrome::HEADER_BOTTOM + 2 {
                break;
            }
            let top = (bottom - height).max(super::chrome::HEADER_BOTTOM + 2);
            // G1's ordinary history line sits against the entry divider. The
            // renderer includes three pixels of font leading in its box.
            let content_y = if expression.height <= 18 && result_y == 0 && result.height <= 18 {
                (bottom - top - expression.height.max(result.height) + 3).max(0)
            } else {
                2
            };
            rows.push(HistoryRow {
                index,
                top,
                height: bottom - top,
                expression,
                result_x: if result.width > 304 {
                    1
                } else {
                    319 - result.width
                },
                result,
                result_y,
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
                let result = if r.result_y > 0 {
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
        match entry {
            Some(e) if self.history_result && !e.error => {
                math_layout::result(e.display_result(), e.typed_result.as_ref(), 17.)
            }
            Some(entry) if !self.history_result => math_layout::expression(&entry.expression, 17.),
            _ => math_layout::text(self.history_text().unwrap_or(""), 17.),
        }
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
            let x = v.x.saturating_add(dx).clamp(0, (layout.width - 300).max(0));
            let y =
                v.y.saturating_add(dy)
                    .clamp(0, (layout.height - 184).max(0));
            if (v.x, v.y) != (x, y) {
                (v.x, v.y) = (x, y);
                self.touch_view();
            }
        }
    }
    pub fn paint_history_viewer(&self, screen: &mut Grob, viewer: &HistoryViewer) {
        let layout = self.history_layout();
        let height = SHOW_BOTTOM - SHOW_TOP;
        let mut viewport = Grob::new(320, height as usize, 0xffffff);
        layout.paint_static(&mut viewport, 10 - viewer.x, 6 - viewer.y, 0x111111);
        screen.blit(
            (0, SHOW_TOP, 320, SHOW_BOTTOM),
            &viewport,
            (0, 0, 320, height),
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
