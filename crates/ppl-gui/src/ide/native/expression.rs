//! Calculator entry document. Byte positions are always UTF-8 boundaries.
//! Templates retain their editable field ranges; conversion to PPL text does
//! not discard field navigation or the user's cursor/selection state.
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditCommand {
    Left(bool),
    Right(bool),
    Home(bool),
    End(bool),
    Backspace,
    Delete,
    SelectAll,
    Undo,
    Redo,
    Clear,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateKind {
    Parentheses,
    Fraction,
    Power,
    Root,
    NthRoot,
    List,
    Matrix,
    Function(&'static str),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    pub kind: TemplateKind,
    pub span: Range<usize>,
    pub fields: Vec<Range<usize>>,
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Document {
    text: String,
    cursor: usize,
    anchor: Option<usize>,
    templates: Vec<Template>,
}
#[derive(Debug, Clone, Default)]
pub struct ExpressionEditor {
    doc: Document,
    undo: Vec<Document>,
    redo: Vec<Document>,
    batching: bool,
}
impl ExpressionEditor {
    pub fn from_text(text: &str) -> Self {
        Self {
            doc: Document {
                text: text.into(),
                cursor: text.len(),
                templates: recover_templates(text),
                ..Default::default()
            },
            ..Default::default()
        }
    }
    pub fn text(&self) -> &str {
        &self.doc.text
    }
    pub fn cursor(&self) -> usize {
        self.doc.cursor
    }
    pub fn templates(&self) -> &[Template] {
        &self.doc.templates
    }
    pub fn selection(&self) -> Range<usize> {
        let a = self.doc.anchor.unwrap_or(self.doc.cursor);
        a.min(self.doc.cursor)..a.max(self.doc.cursor)
    }
    pub fn selected_text(&self) -> String {
        self.doc.text[self.selection()].to_string()
    }
    /// Restore externally replaced input (old automation/persistence clients).
    pub fn sync_external(&mut self, text: &str) {
        if self.doc.text != text {
            *self = Self::from_text(text);
        }
    }
    pub fn set_text(&mut self, text: &str) {
        if self.doc.text == text {
            return;
        }
        self.checkpoint();
        self.doc = Self::from_text(text).doc;
    }
    fn checkpoint(&mut self) {
        if self.batching {
            return;
        }
        self.undo.push(self.doc.clone());
        if self.undo.len() > 256 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
    pub fn set_cursor(&mut self, position: usize, select: bool) {
        let mut position = position.min(self.doc.text.len());
        while !self.doc.text.is_char_boundary(position) {
            position -= 1;
        }
        if select {
            self.doc.anchor.get_or_insert(self.doc.cursor);
        } else {
            self.doc.anchor = None;
        }
        self.doc.cursor = position;
    }
    fn replace(&mut self, range: Range<usize>, text: &str) {
        let removed = range.len();
        let inserted = text.len();
        // Keep ancestors and fields containing the edit. Remove a template if
        // an edit crosses its delimiters, rather than retaining invalid spans.
        self.doc.templates.retain_mut(|t| {
            if range.start < t.span.end
                && range.end > t.span.start
                && !t
                    .fields
                    .iter()
                    .any(|f| range.start >= f.start && range.end <= f.end)
            {
                return false;
            }
            let adjust = |v: usize, inclusive: bool| {
                if v < range.start || (v == range.start && !inclusive) {
                    v
                } else if v >= range.end {
                    v - removed + inserted
                } else {
                    range.start + inserted
                }
            };
            t.span = adjust(t.span.start, false)..adjust(t.span.end, true);
            for f in &mut t.fields {
                let contains = range.start >= f.start && range.end <= f.end;
                f.start = adjust(f.start, !contains);
                f.end = adjust(f.end, true);
            }
            true
        });
        self.doc.text.replace_range(range.clone(), text);
        self.doc.cursor = range.start + inserted;
        self.doc.anchor = None;
    }
    pub fn insert(&mut self, text: &str) {
        if text.is_empty() && self.selection().is_empty() {
            return;
        }
        self.checkpoint();
        self.replace(self.selection(), text);
    }
    fn inside_string(&self) -> bool {
        let mut quoted = false;
        let mut escaped = false;
        for ch in self.doc.text[..self.selection().start].chars() {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                quoted = !quoted;
            }
        }
        quoted
    }
    /// Calculator math keys denote complete tokens, unlike letters inside a
    /// user identifier. Make implied products explicit in executable source.
    pub fn insert_math_token(&mut self, text: &str) {
        self.insert_math(text, true);
    }
    pub fn insert_math_text(&mut self, text: &str) {
        self.insert_math(text, false);
    }
    fn insert_math(&mut self, text: &str, complete: bool) {
        let prefix = self.doc.text[..self.selection().start].trim_end();
        let previous = prefix.chars().last();
        let first = text.chars().next();
        let suffix = prefix
            .rsplit(|ch: char| !ch.is_alphanumeric() && !matches!(ch, '_' | '.' | '#'))
            .next()
            .unwrap_or_default();
        let number = suffix
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_digit() || ch == '.')
            && !suffix.starts_with('#');
        let operand =
            previous.is_some_and(|ch| ch.is_alphanumeric() || matches!(ch, ')' | ']' | '}'));
        let constant = matches!(text, "π" | "ⅈ") || (complete && matches!(text, "i" | "Ans"));
        let product = !self.inside_string()
            && operand
            && ((constant)
                || (first.is_some_and(char::is_alphabetic)
                    && (number || previous.is_some_and(|ch| matches!(ch, ')' | ']' | '}')))
                    && !matches!(text, "E" | "e")));
        if product {
            self.insert(&format!("*{text}"));
        } else {
            self.insert(text);
        }
    }
    pub fn type_text(&mut self, text: &str) {
        let before = self.doc.clone();
        let nested = self.batching;
        self.batching = true;
        for ch in text.chars() {
            if self.inside_string() {
                self.insert(&ch.to_string());
            } else if ch == '(' {
                self.insert_template(TemplateKind::Parentheses);
            } else if matches!(ch, ')' | ']' | '}')
                && self.doc.text[self.doc.cursor..].starts_with(ch)
                && self.selection().is_empty()
            {
                self.set_cursor(self.doc.cursor + ch.len_utf8(), false);
            } else {
                self.insert_math_text(&ch.to_string());
            }
        }
        self.batching = nested;
        if !nested && (before.text != self.doc.text || before.templates != self.doc.templates) {
            self.undo.push(before);
            if self.undo.len() > 256 {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }
    pub fn cut(&mut self) -> String {
        let text = self.selected_text();
        if !text.is_empty() {
            self.insert("");
        }
        text
    }
    pub fn insert_template(&mut self, kind: TemplateKind) {
        if kind == TemplateKind::Parentheses
            && self.doc.text[self.doc.cursor..].starts_with(')')
            && self.selection().is_empty()
        {
            self.set_cursor(self.doc.cursor + 1, false);
            return;
        }
        let before = self.doc.clone();
        if self.selection().is_empty()
            && matches!(
                kind,
                TemplateKind::Fraction | TemplateKind::Power | TemplateKind::NthRoot
            )
        {
            let operand = self.operand_range();
            if !operand.is_empty() {
                self.doc.anchor = Some(operand.start);
                self.doc.cursor = operand.end;
            }
        }
        let selected = self.selected_text();
        let start = self.selection().start;
        let (text, fields): (String, Vec<Range<usize>>) = match kind {
            TemplateKind::Parentheses => (
                format!("({selected})"),
                std::iter::once(1..1 + selected.len()).collect(),
            ),
            TemplateKind::Fraction => (
                format!("({selected})/()"),
                vec![
                    1..1 + selected.len(),
                    4 + selected.len()..4 + selected.len(),
                ],
            ),
            TemplateKind::Power => (
                format!("({selected})^()"),
                vec![
                    1..1 + selected.len(),
                    4 + selected.len()..4 + selected.len(),
                ],
            ),
            TemplateKind::Root => (
                format!("SQRT({selected})"),
                std::iter::once(5..5 + selected.len()).collect(),
            ),
            TemplateKind::NthRoot => (
                format!("NTHROOT({selected},)"),
                vec![
                    8..8 + selected.len(),
                    9 + selected.len()..9 + selected.len(),
                ],
            ),
            TemplateKind::List => (
                format!("{{{selected}}}"),
                std::iter::once(1..1 + selected.len()).collect(),
            ),
            TemplateKind::Matrix => (
                format!("[{selected}]"),
                std::iter::once(1..1 + selected.len()).collect(),
            ),
            TemplateKind::Function(name) => (
                format!("{name}({selected})"),
                std::iter::once(name.len() + 1..name.len() + 1 + selected.len()).collect(),
            ),
        };
        let previous = self.doc.text[..start].chars().last();
        let suffix = self.doc.text[..start]
            .rsplit(|ch: char| !ch.is_alphanumeric() && !matches!(ch, '_' | '.' | '#'))
            .next()
            .unwrap_or_default();
        let numeric_suffix = suffix
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_digit() || ch == '.');
        let multiplication = match kind {
            TemplateKind::Function(_) | TemplateKind::Root => previous.is_some_and(|ch| {
                ch.is_alphanumeric() || matches!(ch, ')' | ']' | '}' | 'π' | 'ⅈ')
            }),
            TemplateKind::Parentheses | TemplateKind::List | TemplateKind::Matrix => previous
                .is_some_and(|ch| {
                    (ch.is_ascii_digit() && numeric_suffix) || matches!(ch, ')' | ']' | '}')
                }),
            _ => false,
        };
        let offset = usize::from(multiplication);
        let text = if multiplication {
            format!("*{text}")
        } else {
            text
        };
        self.insert(&text);
        if !self.batching
            && let Some(previous) = self.undo.last_mut()
        {
            *previous = before;
        }
        let fields: Vec<_> = fields
            .into_iter()
            .map(|r| r.start + start + offset..r.end + start + offset)
            .collect();
        let focus = if !selected.is_empty() && fields.len() > 1 {
            &fields[1]
        } else {
            &fields[0]
        };
        self.doc.cursor = focus.start;
        self.doc.anchor = (!focus.is_empty()).then_some(focus.end);
        self.doc.templates.push(Template {
            kind,
            span: start + offset..start + text.len(),
            fields,
        });
    }
    pub fn toggle_sign(&mut self) {
        let range = self.selection();
        if !range.is_empty() {
            let selected = self.selected_text();
            self.insert(&format!("-({selected})"));
            return;
        }
        let operand = self.operand_range();
        let (start, end) = (operand.start, operand.end);
        self.checkpoint();
        if start > 0
            && self.doc.text[..start].ends_with('-')
            && (start == 1
                || self.doc.text[..start - 1].ends_with(['(', ',', '+', '-', '*', '/', '^']))
        {
            self.replace(start - 1..start, "");
        } else {
            self.replace(start..start, "-");
            self.doc.cursor = end + 1;
        }
    }
    fn operand_range(&self) -> Range<usize> {
        let end = self.doc.cursor;
        let mut start = end;
        let mut depth = 0i32;
        for (index, ch) in self.doc.text[..end].char_indices().rev() {
            if ch == ')' {
                depth += 1;
            } else if ch == '(' {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            } else if depth == 0 && !(ch.is_alphanumeric() || ch == '.' || ch == '_') {
                break;
            }
            start = index;
        }
        start..end
    }
    pub fn apply(&mut self, command: EditCommand) {
        use EditCommand::*;
        match command {
            Undo => {
                if let Some(doc) = self.undo.pop() {
                    self.redo.push(std::mem::replace(&mut self.doc, doc));
                }
            }
            Redo => {
                if let Some(doc) = self.redo.pop() {
                    self.undo.push(std::mem::replace(&mut self.doc, doc));
                }
            }
            SelectAll => {
                self.doc.anchor = Some(0);
                self.doc.cursor = self.doc.text.len();
            }
            Clear => self.set_text(""),
            Home(select) => self.set_cursor(0, select),
            End(select) => self.set_cursor(self.doc.text.len(), select),
            Left(select) | Right(select) => {
                let right = matches!(command, Right(_));
                let selection = self.selection();
                let position = if !select && !selection.is_empty() {
                    if right {
                        selection.end
                    } else {
                        selection.start
                    }
                } else if !select {
                    let jump = self.doc.templates.iter().rev().find_map(|t| {
                        t.fields.iter().enumerate().find_map(|(i, f)| {
                            if right && self.doc.cursor == f.end {
                                Some(t.fields.get(i + 1).map_or(t.span.end, |n| n.start))
                            } else if !right && self.doc.cursor == f.start {
                                Some(if i > 0 {
                                    t.fields[i - 1].end
                                } else {
                                    t.span.start
                                })
                            } else {
                                None
                            }
                        })
                    });
                    jump.unwrap_or_else(|| self.adjacent(right))
                } else {
                    self.adjacent(right)
                };
                self.set_cursor(position, select);
            }
            Backspace | Delete => {
                let mut range = self.selection();
                if range.is_empty() {
                    if command == Backspace {
                        range.start = self.adjacent(false)
                    } else {
                        range.end = self.adjacent(true)
                    }
                }
                if !range.is_empty() {
                    self.checkpoint();
                    self.replace(range, "");
                }
            }
        }
    }
    fn adjacent(&self, right: bool) -> usize {
        if right {
            self.doc.text[self.doc.cursor..]
                .chars()
                .next()
                .map_or(self.doc.cursor, |c| self.doc.cursor + c.len_utf8())
        } else {
            self.doc.text[..self.doc.cursor]
                .char_indices()
                .next_back()
                .map_or(0, |(i, _)| i)
        }
    }
}

/// Rebuild field structure when a history entry or durable document is opened.
/// The stored source remains authoritative; inferred nodes never rewrite it.
fn recover_templates(text: &str) -> Vec<Template> {
    let mut stack = Vec::new();
    let mut pairs = Vec::new();
    let mut quoted = false;
    for (i, ch) in text.char_indices() {
        if ch == '"' {
            quoted = !quoted;
            continue;
        }
        if quoted {
            continue;
        }
        if matches!(ch, '(' | '[' | '{') {
            stack.push((i, ch));
        } else if matches!(ch, ')' | ']' | '}') {
            if let Some((open, kind)) = stack.pop() {
                if matches!((kind, ch), ('(', ')') | ('[', ']') | ('{', '}')) {
                    pairs.push((open, i, kind));
                }
            }
        }
    }
    let mut templates = Vec::new();
    for &(open, close, ch) in &pairs {
        let prefix_start = text[..open]
            .char_indices()
            .rev()
            .take_while(|(_, c)| c.is_ascii_alphabetic())
            .last()
            .map_or(open, |(i, _)| i);
        let prefix = &text[prefix_start..open];
        let function = match prefix {
            "SIN" => Some("SIN"),
            "ASIN" => Some("ASIN"),
            "COS" => Some("COS"),
            "ACOS" => Some("ACOS"),
            "TAN" => Some("TAN"),
            "ATAN" => Some("ATAN"),
            "LN" => Some("LN"),
            "EXP" => Some("EXP"),
            "LOG" => Some("LOG"),
            "ALOG" => Some("ALOG"),
            "ABS" => Some("ABS"),
            _ => None,
        };
        let kind = match ch {
            '[' => TemplateKind::Matrix,
            '{' => TemplateKind::List,
            _ if prefix == "SQRT" => TemplateKind::Root,
            _ if prefix == "NTHROOT" => TemplateKind::NthRoot,
            _ if function.is_some() => TemplateKind::Function(function.unwrap()),
            _ => TemplateKind::Parentheses,
        };
        let start = if matches!(
            kind,
            TemplateKind::Function(_) | TemplateKind::Root | TemplateKind::NthRoot
        ) {
            prefix_start
        } else {
            open
        };
        let mut fields = std::iter::once(open + 1..close).collect();
        if kind == TemplateKind::NthRoot {
            let mut depth = 0;
            for (offset, c) in text[open + 1..close].char_indices() {
                match c {
                    '(' | '[' | '{' => depth += 1,
                    ')' | ']' | '}' => depth -= 1,
                    ',' if depth == 0 => {
                        let comma = open + 1 + offset;
                        fields = vec![open + 1..comma, comma + 1..close];
                        break;
                    }
                    _ => {}
                }
            }
        }
        templates.push(Template {
            kind,
            span: start..close + 1,
            fields,
        });
    }
    for &(open, close, ch) in &pairs {
        if ch != '(' || open < 2 {
            continue;
        }
        let operator = text.as_bytes()[open - 1];
        if !matches!(operator, b'/' | b'^') {
            continue;
        }
        if let Some(&(numerator, end, _)) = pairs
            .iter()
            .find(|(_, end, kind)| *end == open - 2 && *kind == '(')
        {
            templates.retain(|t| t.span != (numerator..end + 1) && t.span != (open..close + 1));
            templates.push(Template {
                kind: if operator == b'/' {
                    TemplateKind::Fraction
                } else {
                    TemplateKind::Power
                },
                span: numerator..close + 1,
                fields: vec![numerator + 1..end, open + 1..close],
            });
        }
    }
    templates.sort_by_key(|t| (t.span.start, std::cmp::Reverse(t.span.end)));
    templates
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn insertion_selection_unicode_and_undo() {
        let mut e = ExpressionEditor::from_text("12π");
        e.apply(EditCommand::Left(false));
        e.insert("3");
        assert_eq!(e.text(), "123π");
        e.apply(EditCommand::Left(true));
        assert_eq!(e.selected_text(), "3");
        e.insert("4");
        assert_eq!(e.text(), "124π");
        e.apply(EditCommand::Undo);
        assert_eq!(e.text(), "123π");
        e.apply(EditCommand::Redo);
        assert_eq!(e.text(), "124π");
        e.apply(EditCommand::End(false));
        e.apply(EditCommand::Backspace);
        assert_eq!(e.text(), "124");
    }
    #[test]
    fn fractions_keep_fields_and_nested_navigation() {
        let mut e = ExpressionEditor::default();
        e.insert_template(TemplateKind::Fraction);
        e.insert("1");
        e.apply(EditCommand::Right(false));
        e.insert("2");
        assert_eq!(e.text(), "(1)/(2)");
        e.apply(EditCommand::Right(false));
        assert_eq!(e.cursor(), 7);
        e.apply(EditCommand::Undo);
        assert_eq!(e.text(), "(1)/()");
        e.insert_template(TemplateKind::Root);
        e.insert("9");
        assert_eq!(e.text(), "(1)/(SQRT(9))");
        e.apply(EditCommand::Right(false));
        e.apply(EditCommand::Right(false));
        assert_eq!(e.cursor(), e.text().len());
    }
    #[test]
    fn paired_parentheses_and_sign() {
        let mut e = ExpressionEditor::default();
        e.type_text("(2)");
        assert_eq!(e.text(), "(2)");
        assert_eq!(e.cursor(), 3);
        e.toggle_sign();
        assert_eq!(e.text(), "-(2)");
        e.toggle_sign();
        assert_eq!(e.text(), "(2)");
    }
    #[test]
    fn postfix_fraction_and_power_wrap_previous_operand() {
        let mut e = ExpressionEditor::from_text("3+12");
        e.insert_template(TemplateKind::Fraction);
        e.insert("4");
        assert_eq!(e.text(), "3+(12)/(4)");
        e.apply(EditCommand::Undo);
        e.apply(EditCommand::Undo);
        assert_eq!(e.text(), "3+12");
        assert!(e.selection().is_empty());
        e.insert_template(TemplateKind::Power);
        e.insert("2");
        assert_eq!(e.text(), "3+(12)^(2)");
    }
    #[test]
    fn history_restore_rebuilds_fraction_fields_without_rewriting_source() {
        let mut e = ExpressionEditor::from_text("(12)/(SQRT(9))");
        assert!(
            e.templates()
                .iter()
                .any(|t| t.kind == TemplateKind::Fraction)
        );
        e.set_cursor(3, false);
        e.apply(EditCommand::Right(false));
        assert_eq!(e.cursor(), 6);
        e.insert("2+");
        assert_eq!(e.text(), "(12)/(2+SQRT(9))");
        let string = ExpressionEditor::from_text("\"(text)\"");
        assert!(string.templates().is_empty());
    }
    #[test]
    fn cut_delete_and_boundary_movement() {
        let mut e = ExpressionEditor::from_text("abc");
        e.apply(EditCommand::Home(false));
        e.apply(EditCommand::Delete);
        assert_eq!(e.text(), "bc");
        e.apply(EditCommand::SelectAll);
        assert_eq!(e.cut(), "bc");
        assert_eq!(e.text(), "");
        e.apply(EditCommand::Undo);
        assert_eq!(e.text(), "bc");
        e.apply(EditCommand::End(false));
        e.apply(EditCommand::Right(false));
        assert_eq!(e.cursor(), 2);
    }
    #[test]
    fn calculator_shortcuts_capture_root_degree_and_insert_implied_products() {
        let mut editor = ExpressionEditor::from_text("3");
        editor.insert_template(TemplateKind::NthRoot);
        editor.insert("8");
        assert_eq!(editor.text(), "NTHROOT(3,8)");
        assert_eq!(editor.templates()[0].fields.len(), 2);
        let mut editor = ExpressionEditor::from_text("14");
        editor.insert_template(TemplateKind::Root);
        editor.insert("8");
        assert_eq!(editor.text(), "14*SQRT(8)");
        let mut editor = ExpressionEditor::from_text("2");
        editor.insert_template(TemplateKind::Function("SIN"));
        editor.insert("30");
        assert_eq!(editor.text(), "2*SIN(30)");
        let mut editor = ExpressionEditor::from_text("SIN");
        editor.type_text("(");
        editor.insert("30");
        editor.type_text(")");
        assert_eq!(editor.text(), "SIN(30)");
    }
}

#[cfg(test)]
mod math_token_tests {
    use super::*;
    #[test]
    fn calculator_tokens_and_desktop_math_preserve_identifiers_literals_and_undo() {
        let mut e = ExpressionEditor::default();
        e.type_text("2π+3i+4Ans");
        assert_eq!(e.text(), "2*π+3*i+4*Ans");
        e.apply(EditCommand::Undo);
        assert_eq!(e.text(), "");
        e.apply(EditCommand::Redo);
        assert_eq!(e.text(), "2*π+3*i+4*Ans");
        e.set_text("");
        e.type_text("SAVED123(2)+SIN(30)+\"7b.8(\"");
        assert_eq!(e.text(), "SAVED123(2)+SIN(30)+\"7b.8(\"");
        e.set_text("2");
        e.insert_math_token("Ans");
        assert_eq!(e.text(), "2*Ans");
        e.set_text("2E");
        e.type_text("-3");
        assert_eq!(e.text(), "2E-3");
    }
}
