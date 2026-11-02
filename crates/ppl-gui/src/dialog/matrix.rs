//! Native fixed-size EDITMAT grid. Values are validated by the VM on OK.
use super::*;
use crate::keymap::{KEY_LEFT, KEY_RIGHT};
use ppl_vm::host::MatrixDialog;

const VISIBLE_ROWS: usize = 6;
const VISIBLE_COLS: usize = 4;

pub(super) struct MatrixBox {
    model: MatrixDialog,
    editing: bool,
    before_edit: String,
}

impl MatrixBox {
    pub(super) fn new(mut model: MatrixDialog) -> Self {
        model.focus = model.focus.min(model.cells.len().saturating_sub(1));
        Self {
            model,
            editing: false,
            before_edit: String::new(),
        }
    }

    fn begin_edit(&mut self) {
        if !self.editing {
            self.before_edit = self.model.cells[self.model.focus].clone();
            self.model.cells[self.model.focus].clear();
            self.editing = true;
        }
        self.model.error = None;
    }

    fn move_focus(&mut self, code: i64) {
        self.editing = false;
        let columns = self.model.cols.max(1);
        let row = self.model.focus / columns;
        let col = self.model.focus % columns;
        let next = match code {
            KEY_LEFT if col > 0 => self.model.focus - 1,
            KEY_RIGHT | KEY_ENTER if col + 1 < columns => self.model.focus + 1,
            KEY_UP if row > 0 => self.model.focus - columns,
            KEY_DOWN if row + 1 < self.model.rows => self.model.focus + columns,
            _ => self.model.focus,
        };
        self.model.focus = next.min(self.model.cells.len().saturating_sub(1));
    }
}

impl ModalDialog for MatrixBox {
    fn handle(&mut self, key: KeyPress) -> Option<DialogResult> {
        if key.code == Some(KEY_MENU) || (self.model.read_only && key.code == Some(KEY_ENTER)) {
            return Some(DialogResult::Entered(self.model.cells.clone()));
        }
        if is_cancel(key.code) {
            if self.editing {
                self.model.cells[self.model.focus] = self.before_edit.clone();
                self.editing = false;
                return None;
            }
            return Some(DialogResult::Cancelled);
        }
        if self.model.cells.is_empty() {
            return None;
        }
        if matches!(
            key.code,
            Some(KEY_LEFT | KEY_RIGHT | KEY_UP | KEY_DOWN | KEY_ENTER)
        ) {
            self.move_focus(key.code.unwrap());
            return None;
        }
        if self.model.read_only {
            return None;
        }
        if key.code == Some(KEY_DEL) {
            if !self.editing {
                self.before_edit = self.model.cells[self.model.focus].clone();
                self.editing = true;
            }
            self.model.cells[self.model.focus].pop();
            self.model.error = None;
            return None;
        }
        if let Some(ch) = key
            .ch
            .filter(|c| !c.is_control())
            .or_else(|| key.code.and_then(char_of_key))
        {
            self.begin_edit();
            self.model.cells[self.model.focus].push(ch);
        }
        None
    }

    fn render(&self, base: &Grob) -> Grob {
        let mut frame = base.clone();
        draw_frame(&mut frame, 0, 12, 319, 236, &self.model.title);
        let columns = self.model.cols.max(1);
        let row_start = (self.model.focus / columns) / VISIBLE_ROWS * VISIBLE_ROWS;
        let col_start = (self.model.focus % columns) / VISIBLE_COLS * VISIBLE_COLS;
        for col in col_start..self.model.cols.min(col_start + VISIBLE_COLS) {
            let label = self
                .model
                .column_headers
                .get(col)
                .cloned()
                .unwrap_or_else(|| (col + 1).to_string());
            draw_text(
                &mut frame,
                &label,
                52 + (col - col_start) as i64 * 67,
                37,
                PX_HINT,
                C_BLACK,
                Some(60),
                None,
            );
        }
        for row in row_start..self.model.rows.min(row_start + VISIBLE_ROWS) {
            let y = 52 + (row - row_start) as i64 * 23;
            let label = self
                .model
                .row_headers
                .get(row)
                .cloned()
                .unwrap_or_else(|| (row + 1).to_string());
            draw_text(
                &mut frame,
                &label,
                4,
                y + 4,
                PX_HINT,
                C_BLACK,
                Some(41),
                None,
            );
            for col in col_start..self.model.cols.min(col_start + VISIBLE_COLS) {
                let index = row * columns + col;
                let Some(text) = self.model.cells.get(index) else {
                    continue;
                };
                let x = 48 + (col - col_start) as i64 * 67;
                let focused = index == self.model.focus;
                frame.rect(
                    x,
                    y,
                    x + 66,
                    y + 21,
                    if focused { C_SELECT } else { 0x0080_8080 },
                    C_WHITE,
                );
                let mut text = text.clone();
                if focused && self.editing {
                    text.push('_');
                }
                while measure(&text, PX) > 58 && !text.is_empty() {
                    text.remove(0);
                }
                draw_text(&mut frame, &text, x + 4, y + 4, PX, C_BLACK, Some(58), None);
            }
        }
        if let Some(error) = &self.model.error {
            draw_text(&mut frame, error, 5, 194, PX_HINT, C_ERROR, Some(309), None);
        } else {
            let status = if self.model.read_only {
                "Read only".to_owned()
            } else {
                format!(
                    "Cell {},{}  Enter: set  Arrows: move",
                    self.model.focus / columns + 1,
                    self.model.focus % columns + 1
                )
            };
            draw_text(
                &mut frame,
                &status,
                5,
                194,
                PX_HINT,
                C_HINT,
                Some(309),
                None,
            );
        }
        draw_text(
            &mut frame,
            "[Menu] OK   [Esc] Cancel",
            5,
            219,
            PX_HINT,
            C_HINT,
            Some(309),
            None,
        );
        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model() -> MatrixDialog {
        MatrixDialog {
            title: "Matrix editor".into(),
            row_headers: vec!["First".into(), "Second".into()],
            column_headers: vec!["X".into(), "Y".into()],
            rows: 2,
            cols: 2,
            cells: vec!["1".into(), "2".into(), "3".into(), "4".into()],
            read_only: false,
            error: None,
            focus: 0,
        }
    }
    #[test]
    fn edits_commit_on_ok_and_cancel_has_no_partial_writeback() {
        let mut grid = MatrixBox::new(model());
        grid.handle(KeyPress::with_char(None, '9'));
        grid.handle(KeyPress::code(KEY_ENTER));
        assert_eq!(grid.model.focus, 1);
        assert_eq!(
            grid.handle(KeyPress::code(KEY_MENU)),
            Some(DialogResult::Entered(vec![
                "9".into(),
                "2".into(),
                "3".into(),
                "4".into()
            ]))
        );
        assert_eq!(
            grid.handle(KeyPress::code(KEY_ESC)),
            Some(DialogResult::Cancelled)
        );
    }
    #[test]
    fn escape_during_cell_edit_restores_cell_before_cancelling_form() {
        let mut grid = MatrixBox::new(model());
        grid.handle(KeyPress::with_char(None, '9'));
        assert_eq!(grid.handle(KeyPress::code(KEY_ESC)), None);
        assert_eq!(grid.model.cells[0], "1");
        assert_eq!(
            grid.handle(KeyPress::code(KEY_ESC)),
            Some(DialogResult::Cancelled)
        );
    }
    #[test]
    fn readonly_ignores_edits_but_allows_navigation() {
        let mut request = model();
        request.read_only = true;
        let mut grid = MatrixBox::new(request);
        grid.handle(KeyPress::with_char(None, '9'));
        grid.handle(KeyPress::code(KEY_DEL));
        grid.handle(KeyPress::code(KEY_DOWN));
        assert_eq!(grid.model.cells, model().cells);
        assert_eq!(grid.model.focus, 2);
    }
    #[test]
    fn render_headers_and_scroll_to_offscreen_cells() {
        let mut request = model();
        request.rows = 8;
        request.cols = 5;
        request.cells = (0..40).map(|i| i.to_string()).collect();
        let mut grid = MatrixBox::new(request);
        let first = grid.render(&Grob::screen());
        if let Ok(path) = std::env::var("PRIME_MATRIX_CAPTURE") {
            first.save_png(std::path::Path::new(&path)).unwrap();
        }
        for _ in 0..7 {
            grid.handle(KeyPress::code(KEY_DOWN));
        }
        for _ in 0..4 {
            grid.handle(KeyPress::code(KEY_RIGHT));
        }
        assert_eq!(grid.model.focus, 39);
        assert_ne!(first.pixels(), grid.render(&Grob::screen()).pixels());
    }
}
