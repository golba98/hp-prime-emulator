//! List editing reuses INPUT's fields and cursor/clipboard behavior.
use super::*;
pub(super) struct ListBox {
    form: InputBox,
}
impl ListBox {
    pub fn new(title: &str, items: Vec<String>) -> Self {
        Self {
            form: InputBox::new(
                title,
                items
                    .into_iter()
                    .enumerate()
                    .map(|(i, text)| (format!("{}", i + 1), "".into(), text))
                    .collect(),
            ),
        }
    }
    pub fn with_error(mut self, focus: usize, error: Option<String>) -> Self {
        self.form = self.form.with_error(focus, error);
        self
    }
    fn append(&mut self) {
        let index = self.form.fields.len();
        let mut extra = InputBox::new("", vec![(format!("{}", index + 1), "".into(), "0".into())]);
        self.form.fields.append(&mut extra.fields);
        self.form.focus = index;
        self.form.fix_scroll();
    }
    fn remove(&mut self) {
        if self.form.focus < self.form.fields.len() {
            self.form.fields.remove(self.form.focus);
        }
        self.form.focus = self
            .form
            .focus
            .min(self.form.fields.len().saturating_sub(1));
        for (i, field) in self.form.fields.iter_mut().enumerate() {
            field.label = (i + 1).to_string();
        }
        self.form.fix_scroll();
    }
}
impl ModalDialog for ListBox {
    fn template(&mut self, template: TemplateKind) {
        self.form.template(template);
    }
    fn sign(&mut self) {
        self.form.sign();
    }

    fn handle(&mut self, key: KeyPress) -> Option<DialogResult> {
        self.form.handle(key)
    }
    fn edit(&mut self, command: EditCommand) {
        self.form.edit(command);
    }
    fn clipboard(&mut self, cut: bool) -> Option<String> {
        self.form.clipboard(cut)
    }
    fn pointer(&mut self, x: i64, y: i64) -> Option<DialogResult> {
        if (195..=214).contains(&y) {
            if x < 160 {
                self.append();
            } else {
                self.remove();
            }
            return None;
        }
        self.form.pointer(x, y)
    }
    fn render(&self, base: &Grob) -> Grob {
        let mut frame = self.form.render(base);
        draw_text(
            &mut frame,
            "Add item                    Delete item",
            16,
            201,
            PX_HINT,
            C_HINT,
            Some(288),
            None,
        );
        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn append_edit_delete_and_cancel_list() {
        let mut list = ListBox::new("L1", vec!["1".into(), "2".into()]);
        list.pointer(20, 202);
        list.event(DialogEvent::Text("9".into()));
        assert_eq!(list.form.values(), vec!["1", "2", "9"]);
        list.pointer(200, 202);
        assert_eq!(list.form.values(), vec!["1", "2"]);
        assert_eq!(list.pointer(140, 227), Some(DialogResult::Cancelled));
        assert_eq!(
            list.pointer(40, 227),
            Some(DialogResult::Entered(vec!["1".into(), "2".into()]))
        );
    }
}
