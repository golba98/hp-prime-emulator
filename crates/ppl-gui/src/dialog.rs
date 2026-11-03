//! Modal dialog state machines for `MSGBOX`, `CHOOSE` and `INPUT`.
//!
//! Each dialog is a small, window-independent state machine: it consumes
//! [`KeyPress`] events (HP key code plus the optional typed character) and
//! eventually yields a [`DialogResult`]. Rendering produces a fresh 320×240
//! frame drawn over the program's current screen, so the whole flow can be
//! unit-tested without a window.
//!
//! Key bindings (HP key codes from [`crate::keymap`]):
//!
//! | Dialog | Accept | Cancel | Other |
//! |---|---|---|---|
//! | MSGBOX | Enter (30) | Esc (4), On (46) | everything else is ignored |
//! | CHOOSE | Enter (30), digit 1–9 within range | Esc, On | Up (2) / Down (12) move the selection |
//! | INPUT  | Enter (30) | Esc, On | typed characters append, Del/Backspace (19) deletes, Up/Down switch field |

mod matrix;

use ppl_graphics::Grob;
use ppl_graphics::text::{draw_text, measure};
use ppl_vm::host::{Dialog, DialogResult, INPUT_LINES_PER_PAGE, InputPosition};

use crate::keymap::{
    KEY_0, KEY_1, KEY_2, KEY_3, KEY_4, KEY_5, KEY_6, KEY_7, KEY_8, KEY_9, KEY_ANS, KEY_COMMA,
    KEY_DEL, KEY_DIV, KEY_DOT, KEY_DOWN, KEY_EEX, KEY_ENTER, KEY_ESC, KEY_MENU, KEY_MINUS, KEY_NEG,
    KEY_ON, KEY_PAREN, KEY_PLUS, KEY_POW, KEY_TIMES, KEY_UP,
};

/// One key event delivered to the VM worker: the HP key code (if the PC key
/// maps to one) and the printable character the user typed (if any).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyPress {
    pub code: Option<i64>,
    pub ch: Option<char>,
}

impl KeyPress {
    /// A key event with only an HP key code.
    pub fn code(code: i64) -> Self {
        Self {
            code: Some(code),
            ch: None,
        }
    }

    /// A key event carrying both the HP key code and the typed character.
    pub fn with_char(code: Option<i64>, ch: char) -> Self {
        Self { code, ch: Some(ch) }
    }
}

/// A modal dialog driven by key presses.
pub trait ModalDialog: Send {
    /// Feeds one key; `Some` ends the dialog with that result.
    fn handle(&mut self, key: KeyPress) -> Option<DialogResult>;
    /// Draws the dialog over `base` (the program's screen).
    fn render(&self, base: &Grob) -> Grob;
}

/// Builds the state machine for a VM dialog request.
pub fn make_dialog(dialog: &Dialog) -> Box<dyn ModalDialog> {
    match dialog {
        Dialog::Matrix(matrix) => Box::new(matrix::MatrixBox::new(matrix.clone())),
        Dialog::Message { text } => Box::new(MessageBox::new(text)),
        Dialog::Choose {
            title,
            items,
            current,
        } => Box::new(ChooseBox::new(title, items.clone(), *current)),
        Dialog::Input {
            title,
            fields,
            error,
            focus,
        } => Box::new(
            InputBox::new(
                title,
                fields
                    .iter()
                    .map(|f| (f.label.clone(), f.help.clone(), f.current.clone()))
                    .collect(),
            )
            .with_resets(fields.iter().map(|f| f.reset_text.clone()).collect())
            .with_layout(
                fields
                    .iter()
                    .map(|f| (f.position, f.page_title.clone()))
                    .collect(),
            )
            .with_error(*focus, error.clone()),
        ),
    }
}

fn is_cancel(code: Option<i64>) -> bool {
    matches!(code, Some(KEY_ESC) | Some(KEY_ON))
}

/// The digit (0–9) an HP key code stands for.
pub fn digit_of(code: i64) -> Option<usize> {
    Some(match code {
        KEY_0 => 0,
        KEY_1 => 1,
        KEY_2 => 2,
        KEY_3 => 3,
        KEY_4 => 4,
        KEY_5 => 5,
        KEY_6 => 6,
        KEY_7 => 7,
        KEY_8 => 8,
        KEY_9 => 9,
        _ => return None,
    })
}

/// The character an HP key inserts into an edit line when no typed
/// character accompanies it (e.g. keys injected without text).
pub fn char_of_key(code: i64) -> Option<char> {
    if let Some(d) = digit_of(code) {
        return char::from_digit(d as u32, 10);
    }
    Some(match code {
        KEY_DOT => '.',
        KEY_COMMA => ',',
        KEY_PLUS => '+',
        KEY_MINUS | KEY_NEG => '-',
        KEY_TIMES => '*',
        KEY_DIV => '/',
        KEY_POW => '^',
        KEY_PAREN => '(',
        KEY_EEX => 'E',
        KEY_ANS => ' ',
        _ => return None,
    })
}

// ---- drawing helpers -------------------------------------------------------

const C_BLACK: u32 = 0x0000_0000;
const C_WHITE: u32 = 0x00FF_FFFF;
const C_PANEL: u32 = 0x00E0_E0E0;
const C_TITLE: u32 = 0x0000_3366;
const C_HINT: u32 = 0x0055_5555;
const C_SELECT: u32 = 0x0033_66CC;
const C_ERROR: u32 = 0x00CC_0000;
const PX: f32 = 12.0;
const PX_HINT: f32 = 10.0;

fn draw_frame(g: &mut Grob, x1: i64, y1: i64, x2: i64, y2: i64, title: &str) {
    g.rect(x1, y1, x2, y2, C_BLACK, C_PANEL);
    g.fill_rect(x1 + 2, y1 + 2, x2 - 2, y1 + 20, C_TITLE);
    draw_text(
        g,
        title,
        x1 + 6,
        y1 + 5,
        PX,
        C_WHITE,
        Some(x2 - x1 - 12),
        None,
    );
}

/// Greedy word wrap to `width` pixels (long words are hard-split).
pub fn wrap_text(text: &str, width: i64, px: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut cur = String::new();
        for word in para.split(' ') {
            let candidate = if cur.is_empty() {
                word.to_string()
            } else {
                format!("{cur} {word}")
            };
            if measure(&candidate, px) <= width {
                cur = candidate;
                continue;
            }
            if !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
            }
            // Hard-split words that are wider than a line on their own.
            for c in word.chars() {
                let mut next = cur.clone();
                next.push(c);
                if measure(&next, px) > width && !cur.is_empty() {
                    lines.push(std::mem::take(&mut cur));
                    cur.push(c);
                } else {
                    cur = next;
                }
            }
        }
        lines.push(cur);
    }
    lines
}

// ---- MSGBOX ----------------------------------------------------------------

/// `MSGBOX(text)`: Enter accepts, Esc/On cancels, other keys are ignored.
#[derive(Clone, Debug)]
pub struct MessageBox {
    pub text: String,
}

impl MessageBox {
    pub fn new(text: &str) -> Self {
        Self { text: text.into() }
    }
}

impl ModalDialog for MessageBox {
    fn handle(&mut self, key: KeyPress) -> Option<DialogResult> {
        match key.code {
            Some(KEY_ENTER) => Some(DialogResult::Ok),
            c if is_cancel(c) => Some(DialogResult::Cancelled),
            _ => None,
        }
    }

    fn render(&self, base: &Grob) -> Grob {
        let mut g = base.clone();
        draw_frame(&mut g, 20, 50, 299, 189, "Message");
        let lines = wrap_text(&self.text, 260, PX);
        for (i, line) in lines.iter().take(6).enumerate() {
            draw_text(
                &mut g,
                line,
                30,
                78 + i as i64 * 16,
                PX,
                C_BLACK,
                Some(260),
                None,
            );
        }
        draw_text(
            &mut g,
            "[Enter] OK   [Esc] Cancel",
            30,
            172,
            PX_HINT,
            C_HINT,
            None,
            None,
        );
        g
    }
}

// ---- CHOOSE ----------------------------------------------------------------

/// Number of CHOOSE rows visible at once.
pub const CHOOSE_ROWS: usize = 8;
const CHOOSE_ROW_H: i64 = 16;

/// `CHOOSE(var, title, items…)`: Up/Down move, Enter picks the highlighted
/// item, digits 1–9 pick that item directly when it exists, Esc/On cancels.
#[derive(Clone, Debug)]
pub struct ChooseBox {
    pub title: String,
    pub items: Vec<String>,
    /// 0-based highlighted item.
    pub selected: usize,
    /// 0-based first visible item.
    pub scroll: usize,
}

impl ChooseBox {
    /// `current` is the 1-based initial selection (clamped into range).
    pub fn new(title: &str, items: Vec<String>, current: usize) -> Self {
        let selected = current.clamp(1, items.len().max(1)) - 1;
        let mut b = Self {
            title: title.into(),
            items,
            selected,
            scroll: 0,
        };
        b.fix_scroll();
        b
    }

    fn fix_scroll(&mut self) {
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + CHOOSE_ROWS {
            self.scroll = self.selected + 1 - CHOOSE_ROWS;
        }
    }
}

impl ModalDialog for ChooseBox {
    fn handle(&mut self, key: KeyPress) -> Option<DialogResult> {
        if self.items.is_empty() {
            return Some(DialogResult::Cancelled);
        }
        match key.code {
            Some(KEY_ENTER) => Some(DialogResult::Chosen(self.selected + 1)),
            c if is_cancel(c) => Some(DialogResult::Cancelled),
            Some(KEY_UP) => {
                self.selected = self.selected.saturating_sub(1);
                self.fix_scroll();
                None
            }
            Some(KEY_DOWN) => {
                if self.selected + 1 < self.items.len() {
                    self.selected += 1;
                }
                self.fix_scroll();
                None
            }
            Some(code) => match digit_of(code) {
                Some(d) if d >= 1 && d <= self.items.len() => Some(DialogResult::Chosen(d)),
                _ => None,
            },
            None => None,
        }
    }

    fn render(&self, base: &Grob) -> Grob {
        let mut g = base.clone();
        let title = if self.title.is_empty() {
            "Choose"
        } else {
            &self.title
        };
        draw_frame(&mut g, 20, 30, 299, 209, title);
        let top = 56;
        for (row, idx) in (self.scroll..self.items.len())
            .take(CHOOSE_ROWS)
            .enumerate()
        {
            let y = top + row as i64 * CHOOSE_ROW_H;
            let fg = if idx == self.selected {
                g.fill_rect(24, y - 1, 286, y + CHOOSE_ROW_H - 2, C_SELECT);
                C_WHITE
            } else {
                C_BLACK
            };
            let line = format!("{}. {}", idx + 1, self.items[idx]);
            draw_text(&mut g, &line, 28, y, PX, fg, Some(254), None);
        }
        if self.items.len() > CHOOSE_ROWS {
            // Scrollbar: track plus a thumb proportional to the visible window.
            let (t0, t1) = (top - 1, top + CHOOSE_ROWS as i64 * CHOOSE_ROW_H - 2);
            g.fill_rect(290, t0, 295, t1, 0x00B0_B0B0);
            let len = self.items.len() as i64;
            let span = t1 - t0;
            let a = t0 + span * self.scroll as i64 / len;
            let b = t0 + span * (self.scroll + CHOOSE_ROWS) as i64 / len;
            g.fill_rect(290, a, 295, b.max(a + 3), C_TITLE);
        }
        draw_text(
            &mut g,
            "[Up/Down] Move  [Enter] OK  [Esc] Cancel",
            26,
            193,
            PX_HINT,
            C_HINT,
            Some(270),
            None,
        );
        g
    }
}

// ---- INPUT -----------------------------------------------------------------

/// One editable INPUT field.
#[derive(Clone, Debug)]
pub struct EditField {
    pub label: String,
    pub help: String,
    pub text: String,
    /// The field still shows its initial value: the first typed character
    /// replaces it (like the calculator's edit line).
    pub fresh: bool,
    pub reset: Option<String>,
    pub position: Option<InputPosition>,
    pub page_title: Option<String>,
}

/// Number of INPUT fields visible at once.
pub const INPUT_ROWS: usize = INPUT_LINES_PER_PAGE;
const INPUT_ROW_H: i64 = 22;

/// `INPUT(...)`: printable characters append to the focused field, Del /
/// Backspace deletes, Up/Down change field, Enter accepts, Esc/On cancels.
#[derive(Clone, Debug)]
pub struct InputBox {
    pub title: String,
    pub fields: Vec<EditField>,
    pub focus: usize,
    pub scroll: usize,
    /// Why the previous entry was rejected (shown in place of the help).
    pub error: Option<String>,
}

impl InputBox {
    /// `fields` are `(label, help, current value)`.
    pub fn new(title: &str, fields: Vec<(String, String, String)>) -> Self {
        Self {
            title: title.into(),
            fields: fields
                .into_iter()
                .map(|(label, help, text)| EditField {
                    label,
                    help,
                    text,
                    fresh: true,
                    reset: None,
                    position: None,
                    page_title: None,
                })
                .collect(),
            focus: 0,
            scroll: 0,
            error: None,
        }
    }

    pub fn with_resets(mut self, resets: Vec<Option<String>>) -> Self {
        for (field, reset) in self.fields.iter_mut().zip(resets) {
            field.reset = reset;
        }
        self
    }

    pub fn with_layout(mut self, layout: Vec<(Option<InputPosition>, Option<String>)>) -> Self {
        for (field, (position, title)) in self.fields.iter_mut().zip(layout) {
            field.position = position;
            field.page_title = title;
        }
        self.fix_scroll();
        self
    }

    fn page_title(&self) -> &str {
        self.fields
            .get(self.focus)
            .and_then(|f| f.page_title.as_deref())
            .unwrap_or(&self.title)
    }

    /// Visible edit rectangle. Percentages are relative to the full 320px
    /// display, and line numbers select fixed pages of seven rows.
    fn field_rect(&self, index: usize) -> Option<(i64, i64, i64, i64)> {
        let position = self.fields[index].position;
        let line = position.map_or(index, |p| p.line);
        if line / INPUT_ROWS != self.scroll / INPUT_ROWS {
            return None;
        }
        let y = 42 + (line % INPUT_ROWS) as i64 * INPUT_ROW_H;
        let (left, right) = position.map_or((110, 300), |p| {
            let left = (p.start_percent * 3.2).round() as i64;
            let right = ((p.start_percent + p.width_percent) * 3.2).round() as i64;
            (left.clamp(0, 319), right.clamp(0, 319))
        });
        Some((left, y - 2, right, y + 16))
    }

    /// Reopens the form after a rejected entry: focus on the offending
    /// field and show the reason.
    pub fn with_error(mut self, focus: usize, error: Option<String>) -> Self {
        self.focus = focus.min(self.fields.len().saturating_sub(1));
        self.error = error;
        self.fix_scroll();
        self
    }

    pub fn values(&self) -> Vec<String> {
        self.fields.iter().map(|f| f.text.clone()).collect()
    }

    fn fix_scroll(&mut self) {
        let line = self
            .fields
            .get(self.focus)
            .and_then(|f| f.position)
            .map_or(self.focus, |p| p.line);
        self.scroll = line / INPUT_ROWS * INPUT_ROWS;
    }
}

impl ModalDialog for InputBox {
    fn handle(&mut self, key: KeyPress) -> Option<DialogResult> {
        match key.code {
            Some(KEY_ENTER) => return Some(DialogResult::Entered(self.values())),
            c if is_cancel(c) => return Some(DialogResult::Cancelled),
            _ => {}
        }
        if key.code == Some(KEY_MENU) {
            for field in &mut self.fields {
                if let Some(reset) = &field.reset {
                    field.text = reset.clone();
                    field.fresh = true;
                }
            }
            self.error = None;
            return None;
        }
        let field = self.fields.get_mut(self.focus)?;
        match key.code {
            Some(KEY_DEL) => {
                field.text.pop();
                field.fresh = false;
                return None;
            }
            Some(KEY_UP) => {
                self.focus = self.focus.saturating_sub(1);
                self.fix_scroll();
                return None;
            }
            Some(KEY_DOWN) => {
                if self.focus + 1 < self.fields.len() {
                    self.focus += 1;
                }
                self.fix_scroll();
                return None;
            }
            _ => {}
        }
        let ch = key
            .ch
            .filter(|c| !c.is_control())
            .or_else(|| key.code.and_then(char_of_key));
        if let Some(c) = ch {
            if field.fresh {
                field.text.clear();
                field.fresh = false;
            }
            field.text.push(c);
        }
        None
    }

    fn render(&self, base: &Grob) -> Grob {
        let mut g = base.clone();
        let title = if self.page_title().is_empty() {
            "Input"
        } else {
            self.page_title()
        };
        draw_frame(&mut g, 0, 12, 319, 236, title);
        for (idx, f) in self.fields.iter().enumerate() {
            let Some((left, top, right, bottom)) = self.field_rect(idx) else {
                continue;
            };
            let label = if f.label.is_empty() {
                format!("{}:", idx + 1)
            } else {
                format!("{}:", f.label)
            };
            let label_left = if f.position.is_some() {
                let previous_edge = (0..self.fields.len())
                    .filter(|&other| other != idx)
                    .filter_map(|other| self.field_rect(other))
                    .filter(|&(x, y, right, _)| y == top && x < left && right <= left)
                    .map(|(_, _, right, _)| right + 4)
                    .max()
                    .unwrap_or(4);
                (left - measure(&label, PX) - 4).max(previous_edge)
            } else {
                16
            };
            if left - label_left > 4 {
                draw_text(
                    &mut g,
                    &label,
                    label_left,
                    top + 4,
                    PX,
                    C_BLACK,
                    Some(left - label_left - 4),
                    None,
                );
            }
            let focused = idx == self.focus;
            let edge = if focused { C_SELECT } else { 0x0080_8080 };
            g.rect(left, top, right, bottom, edge, C_WHITE);
            let mut shown = f.text.clone();
            if focused {
                shown.push('_');
            }
            let text_width = right - left - 8;
            // Keep the end of long values (where the cursor is) visible.
            while measure(&shown, PX) > text_width && !shown.is_empty() {
                shown.remove(0);
            }
            if text_width > 0 {
                draw_text(
                    &mut g,
                    &shown,
                    left + 4,
                    top + 3,
                    PX,
                    C_BLACK,
                    Some(text_width),
                    None,
                );
            }
        }
        let help = self
            .fields
            .get(self.focus)
            .map(|f| f.help.as_str())
            .unwrap_or("");
        if let Some(err) = &self.error {
            draw_text(&mut g, err, 16, 204, PX_HINT, C_ERROR, Some(288), None);
        } else if !help.is_empty() {
            draw_text(&mut g, help, 16, 204, PX_HINT, C_HINT, Some(288), None);
        }
        draw_text(
            &mut g,
            if self.fields.iter().any(|f| f.reset.is_some()) {
                "[Enter] OK  [Esc] Cancel  [Menu] Reset"
            } else {
                "[Enter] OK  [Esc] Cancel  [Del] Erase"
            },
            16,
            220,
            PX_HINT,
            C_HINT,
            Some(288),
            None,
        );
        g
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(d: &mut dyn ModalDialog, keys: &[KeyPress]) -> Option<DialogResult> {
        for k in keys {
            if let Some(r) = d.handle(*k) {
                return Some(r);
            }
        }
        None
    }

    fn typed(s: &str) -> Vec<KeyPress> {
        s.chars().map(|c| KeyPress::with_char(None, c)).collect()
    }

    #[test]
    fn input_reset_changes_editor_values_and_cancel_still_cancels() {
        let mut form = InputBox::new("T", vec![("A".into(), "".into(), "5".into())])
            .with_resets(vec![Some("2".into())]);
        form.handle(KeyPress::with_char(None, '8'));
        form.handle(KeyPress::code(KEY_MENU));
        assert_eq!(form.values(), vec!["2"]);
        assert_eq!(
            form.handle(KeyPress::code(KEY_ENTER)),
            Some(DialogResult::Entered(vec!["2".into()]))
        );
        assert_eq!(
            form.handle(KeyPress::code(KEY_ESC)),
            Some(DialogResult::Cancelled)
        );
    }

    #[test]
    fn msgbox_accepts_enter_and_cancels_on_esc() {
        let mut m = MessageBox::new("hi");
        assert_eq!(m.handle(KeyPress::code(KEY_1)), None);
        assert_eq!(m.handle(KeyPress::with_char(None, 'a')), None);
        assert_eq!(m.handle(KeyPress::code(KEY_ENTER)), Some(DialogResult::Ok));
        assert_eq!(
            m.handle(KeyPress::code(KEY_ESC)),
            Some(DialogResult::Cancelled)
        );
        assert_eq!(
            m.handle(KeyPress::code(KEY_ON)),
            Some(DialogResult::Cancelled)
        );
    }

    #[test]
    fn choose_navigation_and_digits() {
        let items: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        let mut c = ChooseBox::new("T", items.clone(), 1);
        assert_eq!(c.handle(KeyPress::code(KEY_DOWN)), None);
        assert_eq!(c.handle(KeyPress::code(KEY_DOWN)), None);
        assert_eq!(c.handle(KeyPress::code(KEY_DOWN)), None); // clamped at last
        assert_eq!(c.selected, 2);
        assert_eq!(c.handle(KeyPress::code(KEY_UP)), None);
        assert_eq!(
            c.handle(KeyPress::code(KEY_ENTER)),
            Some(DialogResult::Chosen(2))
        );

        let mut c = ChooseBox::new("T", items.clone(), 2);
        // Out-of-range digits and unknown keys keep waiting.
        assert_eq!(c.handle(KeyPress::code(KEY_9)), None);
        assert_eq!(c.handle(KeyPress::code(KEY_0)), None);
        assert_eq!(c.handle(KeyPress::code(KEY_PLUS)), None);
        assert_eq!(c.handle(KeyPress::with_char(None, 'z')), None);
        assert_eq!(
            c.handle(KeyPress::code(KEY_3)),
            Some(DialogResult::Chosen(3))
        );

        let mut c = ChooseBox::new("T", items, 7); // clamps to last
        assert_eq!(c.selected, 2);
        assert_eq!(
            c.handle(KeyPress::code(KEY_ESC)),
            Some(DialogResult::Cancelled)
        );
    }

    #[test]
    fn choose_scrolls_long_lists() {
        let items: Vec<String> = (1..=20).map(|i| format!("item {i}")).collect();
        let mut c = ChooseBox::new("T", items, 1);
        for _ in 0..12 {
            c.handle(KeyPress::code(KEY_DOWN));
        }
        assert_eq!(c.selected, 12);
        assert_eq!(c.scroll, 12 + 1 - CHOOSE_ROWS);
        for _ in 0..12 {
            c.handle(KeyPress::code(KEY_UP));
        }
        assert_eq!((c.selected, c.scroll), (0, 0));
        let mut c = ChooseBox::new("T", (1..=20).map(|i| format!("item {i}")).collect(), 20);
        assert_eq!(c.scroll, 20 - CHOOSE_ROWS);
        // Rendering a scrolled list differs from the base screen.
        let base = Grob::screen();
        assert_ne!(c.render(&base).pixels(), base.pixels());
        assert_eq!(
            c.handle(KeyPress::code(KEY_ENTER)),
            Some(DialogResult::Chosen(20))
        );
    }

    #[test]
    fn choose_empty_list_cancels() {
        let mut c = ChooseBox::new("T", Vec::new(), 1);
        assert_eq!(
            c.handle(KeyPress::code(KEY_DOWN)),
            Some(DialogResult::Cancelled)
        );
    }

    #[test]
    fn input_seven_line_pages_preserve_edits_and_title() {
        let mut form = InputBox::new(
            "One",
            (0..8)
                .map(|i| (format!("Field {i}"), "Page help".into(), i.to_string()))
                .collect(),
        )
        .with_layout(
            (0..8)
                .map(|i| (None, Some(if i < 7 { "One" } else { "Two" }.into())))
                .collect(),
        );
        assert_eq!(form.field_rect(6), Some((110, 172, 300, 190)));
        assert_eq!(form.field_rect(7), None);
        if let Ok(path) = std::env::var("PRIME_INPUT_LAYOUT_CAPTURE") {
            form.render(&Grob::screen())
                .save_png(std::path::Path::new(&format!("{path}.seven.png")))
                .unwrap();
        }
        for _ in 0..7 {
            form.handle(KeyPress::code(KEY_DOWN));
        }
        assert_eq!(form.scroll, 7);
        assert_eq!(form.page_title(), "Two");
        assert_eq!(form.field_rect(0), None);
        assert_eq!(form.field_rect(7), Some((110, 40, 300, 58)));
        form.handle(KeyPress::with_char(None, '9'));
        form.handle(KeyPress::code(KEY_UP));
        assert_eq!(form.page_title(), "One");
        assert_eq!(form.values()[7], "9");
        assert_eq!(form.focus, 6);
    }

    #[test]
    fn input_positions_render_same_line_fields_and_sparse_pages() {
        let mut form = InputBox::new(
            "Layout",
            vec![
                ("A".into(), "Two fields share line zero".into(), "12".into()),
                ("B".into(), "".into(), "34".into()),
                ("C".into(), "".into(), "56".into()),
            ],
        )
        .with_layout(vec![
            (
                Some(InputPosition {
                    start_percent: 15.0,
                    width_percent: 25.0,
                    line: 0,
                }),
                Some("Layout".into()),
            ),
            (
                Some(InputPosition {
                    start_percent: 65.0,
                    width_percent: 30.0,
                    line: 0,
                }),
                Some("Layout".into()),
            ),
            (
                Some(InputPosition {
                    start_percent: 25.0,
                    width_percent: 50.0,
                    line: 14,
                }),
                Some("Third page".into()),
            ),
        ]);
        assert_eq!(form.field_rect(0), Some((48, 40, 128, 58)));
        assert_eq!(form.field_rect(1), Some((208, 40, 304, 58)));
        assert_eq!(form.field_rect(2), None);
        let frame = form.render(&Grob::screen());
        assert_eq!(frame.pixels()[40 * 320 + 48], C_SELECT);
        assert_eq!(frame.pixels()[40 * 320 + 208], 0x0080_8080);
        if let Ok(path) = std::env::var("PRIME_INPUT_LAYOUT_CAPTURE") {
            frame.save_png(std::path::Path::new(&path)).unwrap();
        }
        form.handle(KeyPress::code(KEY_DOWN));
        assert_eq!(form.scroll, 0);
        form.handle(KeyPress::code(KEY_DOWN));
        assert_eq!(form.scroll, 14);
        assert_eq!(form.page_title(), "Third page");
        assert_eq!(form.field_rect(2), Some((80, 40, 240, 58)));
        assert_eq!(form.field_rect(1), None);
        // Error focus must select the same positioned page on a re-prompt.
        assert_eq!(form.with_error(0, Some("Invalid input".into())).scroll, 0);
    }

    #[test]
    fn input_typing_replaces_initial_value_then_appends() {
        let mut i = InputBox::new("T", vec![("A".into(), "help".into(), "5".into())]);
        let mut keys = typed("12");
        keys.push(KeyPress::code(KEY_ENTER));
        assert_eq!(
            feed(&mut i, &keys),
            Some(DialogResult::Entered(vec!["12".into()]))
        );
    }

    #[test]
    fn input_backspace_edits_existing_value() {
        let mut i = InputBox::new("T", vec![("A".into(), String::new(), "123".into())]);
        let mut keys = vec![KeyPress::code(KEY_DEL)];
        keys.extend(typed("9"));
        keys.push(KeyPress::code(KEY_ENTER));
        assert_eq!(
            feed(&mut i, &keys),
            Some(DialogResult::Entered(vec!["129".into()]))
        );
    }

    #[test]
    fn input_multiple_fields_and_key_code_fallback() {
        let mut i = InputBox::new(
            "T",
            vec![
                ("A".into(), String::new(), "0".into()),
                ("B".into(), String::new(), "0".into()),
            ],
        );
        let keys = [
            KeyPress::code(KEY_4),
            KeyPress::code(KEY_DOT),
            KeyPress::code(KEY_5),
            KeyPress::code(KEY_DOWN),
            KeyPress::with_char(Some(KEY_MINUS), '-'),
            KeyPress::code(KEY_2),
            // Control characters (e.g. Backspace's "\u{8}") are never inserted.
            KeyPress::with_char(None, '\u{8}'),
            KeyPress::code(KEY_ENTER),
        ];
        assert_eq!(
            feed(&mut i, &keys),
            Some(DialogResult::Entered(vec!["4.5".into(), "-2".into()]))
        );
    }

    #[test]
    fn input_untouched_returns_defaults_and_esc_cancels() {
        let mut i = InputBox::new("T", vec![("A".into(), String::new(), "7".into())]);
        assert_eq!(
            i.handle(KeyPress::code(KEY_ENTER)),
            Some(DialogResult::Entered(vec!["7".into()]))
        );
        let mut i = InputBox::new("T", vec![("A".into(), String::new(), "7".into())]);
        assert_eq!(feed(&mut i, &typed("abc")), None);
        assert_eq!(
            i.handle(KeyPress::code(KEY_ESC)),
            Some(DialogResult::Cancelled)
        );
    }

    #[test]
    fn input_reopened_with_error_focuses_field_and_draws_message() {
        let fields = vec![
            ("A".into(), "help".into(), "1".into()),
            ("B".into(), "help".into(), "abc".into()),
        ];
        let plain = InputBox::new("T", fields.clone());
        let i = InputBox::new("T", fields).with_error(1, Some("Syntax error".into()));
        assert_eq!(i.focus, 1);
        let base = Grob::screen();
        assert_ne!(plain.render(&base).pixels(), i.render(&base).pixels());
        // The previous entry is kept and replaced by the first typed key.
        let mut i = i;
        let mut keys = typed("16");
        keys.push(KeyPress::code(KEY_ENTER));
        assert_eq!(
            feed(&mut i, &keys),
            Some(DialogResult::Entered(vec!["1".into(), "16".into()]))
        );
    }

    #[test]
    fn input_string_field_takes_letters_verbatim() {
        let mut i = InputBox::new("T", vec![("N".into(), String::new(), String::new())]);
        let mut keys = typed("7b.8");
        keys.push(KeyPress::code(KEY_ENTER));
        assert_eq!(
            feed(&mut i, &keys),
            Some(DialogResult::Entered(vec!["7b.8".into()]))
        );
    }

    #[test]
    fn input_render_shows_typed_text() {
        let mut i = InputBox::new("T", vec![("A".into(), String::new(), String::new())]);
        let base = Grob::screen();
        let before = i.render(&base);
        feed(&mut i, &typed("88"));
        let after = i.render(&base);
        assert_ne!(before.pixels(), after.pixels());
    }

    #[test]
    fn wrap_text_splits_long_lines() {
        let lines = wrap_text("the quick brown fox jumps over the lazy dog", 60, 12.0);
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|l| measure(l, 12.0) <= 60));
        assert_eq!(wrap_text("a\nb", 100, 12.0), vec!["a", "b"]);
    }
}
