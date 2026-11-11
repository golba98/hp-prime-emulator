//! Native EDITMAT grid with transactional dimension/cell edits. Values are validated by the VM on OK.
use super::*;
use crate::keymap::{KEY_LEFT, KEY_RIGHT};
use ppl_vm::host::MatrixDialog;

const VISIBLE_ROWS: usize = 6;
const VISIBLE_COLS: usize = 4;

pub(super) struct MatrixBox {
    model: MatrixDialog,
    editing: bool,
    before_edit: String,
    editor: ExpressionEditor,
    dimensions: Option<InputBox>,
    original_size: (usize, usize),
}

impl MatrixBox {
    pub(super) fn new(mut model: MatrixDialog) -> Self {
        model.focus = model.focus.min(model.cells.len().saturating_sub(1));
        let original_size = (model.rows, model.cols);
        Self {
            model,
            editing: false,
            before_edit: String::new(),
            editor: ExpressionEditor::default(),
            dimensions: None,
            original_size,
        }
    }

    fn begin_edit(&mut self) {
        if !self.editing {
            self.before_edit = self.model.cells[self.model.focus].clone();
            self.model.cells[self.model.focus].clear();
            self.editor = ExpressionEditor::default();
            self.editing = true;
        }
        self.model.error = None;
    }

    fn commit_result(&self) -> DialogResult {
        if (self.model.rows, self.model.cols) == self.original_size {
            DialogResult::Entered(self.model.cells.clone())
        } else {
            DialogResult::MatrixEdited {
                rows: self.model.rows,
                cols: self.model.cols,
                cells: self.model.cells.clone(),
            }
        }
    }
    fn resize(&mut self, rows: usize, cols: usize) {
        if self.model.read_only
            || (self.model.vector && rows != 1)
            || rows == 0
            || cols == 0
            || rows.checked_mul(cols).is_none_or(|n| n > 1_000_000)
        {
            return;
        }
        let mut cells = vec!["0".to_owned(); rows * cols];
        for r in 0..rows.min(self.model.rows) {
            for c in 0..cols.min(self.model.cols) {
                cells[r * cols + c] = self.model.cells[r * self.model.cols + c].clone();
            }
        }
        self.model.rows = rows;
        self.model.cols = cols;
        self.model.cells = cells;
        self.model.focus = self.model.focus.min(rows * cols - 1);
        self.editing = false;
    }
    fn insert_row(&mut self) {
        if self.model.vector
            || self.model.read_only
            || self.model.cells.len() + self.model.cols > 1_000_000
        {
            return;
        }
        let row = self.model.focus / self.model.cols;
        self.model.cells.splice(
            row * self.model.cols..row * self.model.cols,
            std::iter::repeat_n("0".to_owned(), self.model.cols),
        );
        self.model.rows += 1;
        self.editing = false;
    }
    fn delete_row(&mut self) {
        if self.model.vector || self.model.read_only || self.model.rows <= 1 {
            return;
        }
        let row = self.model.focus / self.model.cols;
        self.model
            .cells
            .drain(row * self.model.cols..(row + 1) * self.model.cols);
        self.model.rows -= 1;
        self.model.focus = self.model.focus.min(self.model.cells.len() - 1);
        self.editing = false;
    }
    fn insert_col(&mut self) {
        if self.model.read_only || self.model.cells.len() + self.model.rows > 1_000_000 {
            return;
        }
        let col = self.model.focus % self.model.cols;
        for row in (0..self.model.rows).rev() {
            self.model
                .cells
                .insert(row * self.model.cols + col, "0".into());
        }
        self.model.cols += 1;
        self.model.focus = (self.model.focus / (self.model.cols - 1)) * self.model.cols + col;
        self.editing = false;
    }
    fn delete_col(&mut self) {
        if self.model.read_only || self.model.cols <= 1 {
            return;
        }
        let col = self.model.focus % self.model.cols;
        let row = self.model.focus / self.model.cols;
        for row in (0..self.model.rows).rev() {
            self.model.cells.remove(row * self.model.cols + col);
        }
        self.model.cols -= 1;
        self.model.focus = row * self.model.cols + col.min(self.model.cols - 1);
        self.editing = false;
    }
    fn dimensions_result(&mut self, result: Option<DialogResult>) {
        match result {
            Some(DialogResult::Entered(values)) => {
                if let [rows, cols] = values.as_slice() {
                    if let (Ok(rows), Ok(cols)) = (rows.parse::<usize>(), cols.parse::<usize>()) {
                        if rows > 0
                            && cols > 0
                            && (!self.model.vector || rows == 1)
                            && rows.checked_mul(cols).is_some_and(|n| n <= 1_000_000)
                        {
                            self.resize(rows, cols);
                            self.dimensions = None;
                            return;
                        }
                    }
                }
                if let Some(form) = &mut self.dimensions {
                    form.error = Some("Positive dimensions; vectors have one row".into());
                }
            }
            Some(DialogResult::Cancelled) => {
                self.dimensions = None;
            }
            _ => {}
        }
    }
    fn begin_dimensions(&mut self) {
        if !self.model.read_only {
            self.dimensions = Some(InputBox::new(
                "Matrix size",
                vec![
                    ("Rows".into(), "".into(), self.model.rows.to_string()),
                    ("Columns".into(), "".into(), self.model.cols.to_string()),
                ],
            ));
        }
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
    fn template(&mut self, template: TemplateKind) {
        if let Some(form) = &mut self.dimensions {
            form.template(template);
            return;
        }
        if self.model.read_only || self.model.cells.is_empty() {
            return;
        }
        self.begin_edit();
        self.editor.insert_template(template);
        self.model.cells[self.model.focus] = self.editor.text().to_owned();
    }
    fn sexagesimal(&mut self) {
        if let Some(form) = &mut self.dimensions {
            form.sexagesimal();
            return;
        }
        if self.model.read_only || self.model.cells.is_empty() {
            return;
        }
        if !self.editing {
            self.before_edit = self.model.cells[self.model.focus].clone();
            self.editing = true;
            self.editor = ExpressionEditor::from_text(&self.before_edit);
        }
        crate::ide::native::sexagesimal::insert_unit(&mut self.editor);
        self.model.cells[self.model.focus] = self.editor.text().to_owned();
    }
    fn sign(&mut self) {
        if let Some(form) = &mut self.dimensions {
            form.sign();
            return;
        }
        if self.model.read_only || self.model.cells.is_empty() {
            return;
        }
        if !self.editing {
            self.before_edit = self.model.cells[self.model.focus].clone();
            self.editing = true;
            self.editor = ExpressionEditor::from_text(&self.before_edit);
        }
        self.editor.toggle_sign();
        self.model.cells[self.model.focus] = self.editor.text().to_owned();
    }

    fn handle(&mut self, key: KeyPress) -> Option<DialogResult> {
        if let Some(form) = &mut self.dimensions {
            let result = form.handle(key);
            self.dimensions_result(result);
            return None;
        }
        if key.code == Some(KEY_MENU) || (self.model.read_only && key.code == Some(KEY_ENTER)) {
            return Some(self.commit_result());
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
        if self.editing && matches!(key.code, Some(KEY_LEFT | KEY_RIGHT)) {
            self.edit(if key.code == Some(KEY_LEFT) {
                EditCommand::Left(false)
            } else {
                EditCommand::Right(false)
            });
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
            self.editor
                .sync_external(&self.model.cells[self.model.focus]);
            self.editor.apply(EditCommand::Backspace);
            self.model.cells[self.model.focus] = self.editor.text().to_owned();
            self.model.error = None;
            return None;
        }
        if let Some(ch) = key
            .ch
            .filter(|c| !c.is_control())
            .or_else(|| key.code.and_then(char_of_key))
        {
            self.begin_edit();
            self.editor.insert_math_text(&ch.to_string());
            self.model.cells[self.model.focus] = self.editor.text().to_owned();
        }
        None
    }

    fn edit(&mut self, command: EditCommand) {
        if let Some(form) = &mut self.dimensions {
            form.edit(command);
            return;
        }
        if self.model.read_only || self.model.cells.is_empty() {
            return;
        }
        if !self.editing {
            self.before_edit = self.model.cells[self.model.focus].clone();
            self.editing = true;
            self.editor = ExpressionEditor::from_text(&self.before_edit);
        }
        self.editor.apply(command);
        self.model.cells[self.model.focus] = self.editor.text().to_owned();
    }
    fn clipboard(&mut self, cut: bool) -> Option<String> {
        if let Some(form) = &mut self.dimensions {
            return form.clipboard(cut);
        }
        if self.model.cells.is_empty() {
            return None;
        }
        if !self.editing {
            return Some(self.model.cells[self.model.focus].clone());
        }
        let text = if cut && !self.model.read_only {
            self.editor.cut()
        } else {
            self.editor.selected_text()
        };
        self.model.cells[self.model.focus] = self.editor.text().to_owned();
        Some(text)
    }
    fn pointer(&mut self, x: i64, y: i64) -> Option<DialogResult> {
        if let Some(form) = &mut self.dimensions {
            let result = form.pointer(x, y);
            self.dimensions_result(result);
            return None;
        }
        if (215..=236).contains(&y) {
            if x < 106 {
                return Some(self.commit_result());
            }
            if x < 214 {
                return Some(DialogResult::Cancelled);
            }
            self.begin_dimensions();
            return None;
        }
        if (189..=210).contains(&y) && !self.model.read_only {
            match x / 80 {
                0 => self.insert_row(),
                1 => self.delete_row(),
                2 => self.insert_col(),
                3 => self.delete_col(),
                _ => {}
            }
            return None;
        }
        if (48..316).contains(&x) && (52..190).contains(&y) {
            let columns = self.model.cols.max(1);
            let row_start = (self.model.focus / columns) / VISIBLE_ROWS * VISIBLE_ROWS;
            let col_start = (self.model.focus % columns) / VISIBLE_COLS * VISIBLE_COLS;
            let row = row_start + ((y - 52) / 23) as usize;
            let col = col_start + ((x - 48) / 67) as usize;
            if row < self.model.rows && col < self.model.cols {
                self.model.focus = row * columns + col;
                self.editing = false;
            }
        }
        None
    }
    fn render(&self, base: &Grob) -> Grob {
        if let Some(form) = &self.dimensions {
            return form.render(base);
        }
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
        if !self.model.read_only {
            draw_text(
                &mut frame,
                "+Row      -Row      +Col      -Col",
                5,
                194,
                PX_HINT,
                C_HINT,
                Some(309),
                None,
            );
        }
        if let Some(error) = &self.model.error {
            draw_text(&mut frame, error, 5, 205, PX_HINT, C_ERROR, Some(309), None);
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
                205,
                PX_HINT,
                C_HINT,
                Some(309),
                None,
            );
        }
        draw_text(
            &mut frame,
            "OK               Cancel         Size",
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
            vector: false,
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
    #[test]
    fn pointer_cell_selection_and_dimension_changes_preserve_cells() {
        let mut grid = MatrixBox::new(model());
        grid.pointer(120, 78);
        assert_eq!(grid.model.focus, 3);
        grid.pointer(20, 199);
        assert_eq!(grid.model.rows, 3);
        assert_eq!(grid.model.cells, vec!["1", "2", "0", "0", "3", "4"]);
        grid.pointer(20, 225);
        assert_eq!(
            grid.pointer(20, 225),
            Some(DialogResult::MatrixEdited {
                rows: 3,
                cols: 2,
                cells: vec![
                    "1".into(),
                    "2".into(),
                    "0".into(),
                    "0".into(),
                    "3".into(),
                    "4".into()
                ]
            })
        );
    }
    #[test]
    fn vector_and_readonly_dimension_controls_do_not_change_shape() {
        let mut request = model();
        request.vector = true;
        request.rows = 1;
        request.cells.truncate(2);
        let mut grid = MatrixBox::new(request);
        grid.pointer(20, 199);
        assert_eq!(grid.model.rows, 1);
        grid.pointer(170, 199);
        assert_eq!(grid.model.cols, 3);
        let mut request = model();
        request.read_only = true;
        let mut grid = MatrixBox::new(request);
        grid.pointer(20, 199);
        grid.pointer(170, 199);
        assert_eq!((grid.model.rows, grid.model.cols), (2, 2));
        assert_eq!(grid.model.cells, model().cells);
    }
}
