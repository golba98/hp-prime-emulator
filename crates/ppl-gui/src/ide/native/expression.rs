//! Calculator entry document. Byte positions are always UTF-8 boundaries.
//! Templates retain their editable field ranges; conversion to PPL text does
//! not discard field navigation or the user's cursor/selection state.
use std::ops::Range;
pub mod palette;

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
    Palette(palette::Form),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    pub kind: TemplateKind,
    pub span: Range<usize>,
    pub fields: Vec<Range<usize>>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SelectionAnchor {
    byte: usize,
    textbook: bool,
}
impl SelectionAnchor {
    fn plain(byte: usize) -> Self {
        Self {
            byte,
            textbook: false,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Document {
    text: String,
    cursor: usize,
    anchor: Option<SelectionAnchor>,
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
    pub fn has_edits(&self) -> bool {
        !self.undo.is_empty() || !self.redo.is_empty()
    }
    pub fn cursor(&self) -> usize {
        self.doc.cursor
    }
    pub fn templates(&self) -> &[Template] {
        &self.doc.templates
    }
    pub(super) fn powered_base<'a>(&self, power: &'a Template) -> Option<&'a Range<usize>> {
        let [base, _] = power.fields.as_slice() else {
            return None;
        };
        if power.kind != TemplateKind::Power
            || self.text().get(power.span.start..base.start)? != "("
            || self.text().as_bytes().get(base.end) != Some(&b')')
        {
            return None;
        }
        self.templates()
            .iter()
            .any(|child| child.kind == TemplateKind::Power && child.span == *base)
            .then_some(base)
    }

    fn powered_base_step(&self, right: bool) -> Option<usize> {
        self.templates().iter().rev().find_map(|power| {
            let close = self.powered_base(power)?.end;
            let exponent = &power.fields[1];
            match self.cursor() {
                pos if right && pos == close => Some(close + 1),
                pos if right && pos == close + 1 => Some(exponent.start),
                pos if !right && pos == exponent.start => Some(close + 1),
                _ => None,
            }
        })
    }
    pub(super) fn fraction_factorial_operand(&self, group: &Template) -> Option<&Template> {
        let [field] = group.fields.as_slice() else {
            return None;
        };
        let source = self.text();
        if group.kind != TemplateKind::Parentheses
            || source.get(group.span.start..field.start)? != "("
            || source.get(field.end..group.span.end)? != ")"
            || !source.get(group.span.end..)?.starts_with('!')
            || parentheses_follow_name(source, group.span.start)
        {
            return None;
        }
        self.templates()
            .iter()
            .find(|template| template.kind == TemplateKind::Fraction && template.span == *field)
    }

    fn fraction_factorial_step(&self, right: bool) -> Option<usize> {
        let cursor = self.cursor();
        self.templates().iter().rev().find_map(|group| {
            let [upper, lower] = self.fraction_factorial_operand(group)?.fields.as_slice() else {
                return None;
            };
            if right && cursor == group.span.start {
                Some(lower.start)
            } else if !right && cursor == group.span.end {
                Some(lower.end)
            } else if right && (cursor == upper.end || cursor == lower.end) {
                Some(group.span.end)
            } else if !right && (cursor == upper.start || cursor == lower.start) {
                Some(group.span.start)
            } else {
                None
            }
        })
    }
    pub fn selection(&self) -> Range<usize> {
        let cursor = self.doc.cursor;
        let Some(anchor) = self.doc.anchor else {
            return cursor..cursor;
        };
        let mut range = anchor.byte.min(cursor)..anchor.byte.max(cursor);
        if anchor.textbook && !range.is_empty() {
            for template in &self.doc.templates {
                let Some(field) = self.stacked_absolute_field(template) else {
                    continue;
                };
                // Touching either visible bar selects the complete ABS node.
                // The underlying caret and anchor still permit reversing direction.
                if (range.start < field.start && range.end > template.span.start)
                    || (range.start < template.span.end && range.end > field.end)
                {
                    range.start = range.start.min(template.span.start);
                    range.end = range.end.max(template.span.end);
                }
            }
        }
        range
    }
    pub(super) fn stacked_absolute_field<'a>(
        &self,
        template: &'a Template,
    ) -> Option<&'a Range<usize>> {
        let [field] = template.fields.as_slice() else {
            return None;
        };
        if template.kind != TemplateKind::Function("ABS")
            || !self
                .doc
                .text
                .get(template.span.start..field.start)?
                .eq_ignore_ascii_case("ABS(")
            || self.doc.text.get(field.end..template.span.end)? != ")"
        {
            return None;
        }
        self.doc
            .templates
            .iter()
            .any(|child| {
                child.kind == TemplateKind::Fraction
                    && child.span.start >= field.start
                    && child.span.end <= field.end
            })
            .then_some(field)
    }
    fn stacked_absolute_opening_step(&self, right: bool) -> Option<usize> {
        let cursor = self.doc.cursor;
        self.doc.templates.iter().rev().find_map(|template| {
            let [field] = template.fields.as_slice() else {
                return None;
            };
            let position = if right && cursor == template.span.start {
                field.start
            } else if !right && cursor == field.start {
                template.span.start
            } else {
                return None;
            };
            self.stacked_absolute_field(template).map(|_| position)
        })
    }
    /// Move between vertically arranged textbook fields before leaving entry
    /// for history. Offsets count characters, never split a UTF-8 glyph.
    pub fn vertical_field(&mut self, down: bool) -> bool {
        let cursor = self.doc.cursor;
        let target = self
            .doc
            .templates
            .iter()
            .filter_map(|t| {
                let (from, to) = if let TemplateKind::Palette(form) = t.kind {
                    let from = t
                        .fields
                        .iter()
                        .position(|r| cursor >= r.start && cursor <= r.end)?;
                    (
                        from,
                        form.neighbor(from, 0, if down { 1 } else { -1 }, t.fields.len())?,
                    )
                } else {
                    let (upper, lower) = match t.kind {
                        TemplateKind::Fraction if t.fields.len() == 2 => (0, 1),
                        TemplateKind::Power if t.fields.len() == 2 => (1, 0),
                        _ => return None,
                    };
                    if down { (upper, lower) } else { (lower, upper) }
                };
                let source = &t.fields[from];
                if cursor < source.start || cursor > source.end {
                    return None;
                }
                let offset = self.doc.text[source.start..cursor].chars().count();
                let dest = &t.fields[to];
                let byte = self.doc.text[dest.clone()]
                    .char_indices()
                    .nth(offset)
                    .map_or(dest.end, |(i, _)| dest.start + i);
                Some((t.span.len(), byte))
            })
            .min_by_key(|(length, _)| *length)
            .map(|(_, byte)| byte);
        if let Some(byte) = target {
            self.set_cursor(byte, false);
            true
        } else {
            false
        }
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
        self.move_cursor(position, select, false);
    }
    pub(crate) fn set_textbook_cursor(&mut self, position: usize, select: bool) {
        self.move_cursor(position, select, true);
    }
    fn move_cursor(&mut self, position: usize, select: bool, textbook: bool) {
        let mut position = position.min(self.doc.text.len());
        while !self.doc.text.is_char_boundary(position) {
            position -= 1;
        }
        if select {
            self.doc.anchor = Some(SelectionAnchor {
                byte: self
                    .doc
                    .anchor
                    .map_or(self.doc.cursor, |anchor| anchor.byte),
                textbook,
            });
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
    /// Recalled or pasted executable source carries its editable math fields.
    pub fn insert_source(&mut self, text: &str) {
        self.insert(text);
        self.doc.templates = recover_templates(&self.doc.text);
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
        let hash = prefix.rfind('#').map(|index| &prefix[index + 1..]);
        let word = hash
            .and_then(|body| ppl_math::HpInt::parse_literal(body, ppl_math::IntBase::Hex))
            .is_some();
        let continuation = hash.is_some_and(|body| {
            let one = text.chars().next().filter(|_| text.chars().count() == 1);
            let (digits, width) = body
                .split_once(':')
                .map_or((body, None), |(a, b)| (a, Some(b)));
            let digits = digits
                .strip_prefix('-')
                .or_else(|| digits.strip_prefix('−'))
                .unwrap_or(digits);
            if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
                return false;
            }
            match width {
                Some(width) => {
                    let width = width
                        .strip_prefix('-')
                        .or_else(|| width.strip_prefix('−'))
                        .unwrap_or(width);
                    !width.is_empty()
                        && width.bytes().all(|b| b.is_ascii_digit())
                        && one.is_some_and(|c| "hHbBoOdD".contains(c))
                }
                None => one.is_some_and(|c| c.is_ascii_hexdigit() || "hHoO".contains(c)),
            }
        });
        let number = word
            || suffix
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_digit() || ch == '.')
                && !suffix.starts_with('#')
                && !suffix.contains('_');
        let operand =
            previous.is_some_and(|ch| ch.is_alphanumeric() || matches!(ch, ')' | ']' | '}'));
        let constant = matches!(text, "π" | "ⅈ") || (complete && matches!(text, "i" | "Ans"));
        let product = !self.inside_string()
            && !continuation
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
        if text.contains(',')
            && self.doc.templates.iter().any(|t| {
                matches!(
                    t.kind,
                    TemplateKind::Palette(
                        palette::Form::Matrix { .. }
                            | palette::Form::RowVector
                            | palette::Form::ColumnVector
                    )
                )
            })
        {
            self.doc.templates = recover_templates(&self.doc.text);
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
                // Desktop '(' is an explicit opening delimiter, including
                // inside an existing call. The calculator's combined () key
                // retains its separate close-current-field behavior.
                self.insert_template_inner(TemplateKind::Parentheses, false);
            } else if matches!(ch, ')' | ']' | '}')
                && self.doc.text[self.doc.cursor..].starts_with(ch)
                && self.selection().is_empty()
            {
                self.set_cursor(self.doc.cursor + ch.len_utf8(), false);
            } else {
                self.insert_math_text(&ch.to_string());
            }
        }
        self.doc.templates = recover_templates(&self.doc.text);
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
        self.insert_template_inner(kind, true);
    }
    fn insert_template_inner(&mut self, kind: TemplateKind, close_current: bool) {
        if close_current
            && kind == TemplateKind::Parentheses
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
                TemplateKind::Fraction
                    | TemplateKind::Power
                    | TemplateKind::Palette(
                        palette::Form::Index
                            | palette::Form::MixedFraction
                            | palette::Form::Sexagesimal
                    )
            )
        {
            let operand = self.operand_range();
            if !operand.is_empty() {
                self.doc.anchor = Some(SelectionAnchor::plain(operand.start));
                self.doc.cursor = operand.end;
            }
        }
        let selected = self.selected_text();
        let start = self.selection().start;
        let (text, fields): (String, Vec<Range<usize>>) = match kind {
            TemplateKind::Palette(form) => form.build(&selected),
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
            TemplateKind::Palette(palette::Form::Index) => false,
            TemplateKind::Function(_)
            | TemplateKind::Root
            | TemplateKind::NthRoot
            | TemplateKind::Palette(_) => previous.is_some_and(|ch| {
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
        let first = match kind {
            TemplateKind::Palette(form) => form.initial_field(),
            _ => 0,
        };
        let focus = if matches!(kind, TemplateKind::Palette(_)) {
            if !selected.is_empty() && !fields[first].is_empty() {
                fields
                    .iter()
                    .find(|field| field.is_empty())
                    .unwrap_or(&fields[first])
            } else {
                &fields[first]
            }
        } else if !selected.is_empty() && fields.len() > 1 {
            &fields[1]
        } else {
            &fields[0]
        };
        self.doc.cursor = focus.start;
        self.doc.anchor = (!focus.is_empty()).then_some(SelectionAnchor::plain(focus.end));
        self.doc.templates.push(Template {
            kind,
            span: start + offset..start + text.len(),
            fields,
        });
    }
    /// Produce executable text without mutating the edited document or its
    /// caret. Optional calculus bounds may remain empty in their outline.
    pub fn evaluation_text(&self, cas: bool) -> Result<String, String> {
        self.evaluation_text_depth(cas, 0)
    }
    fn evaluation_text_depth(&self, cas: bool, depth: usize) -> Result<String, String> {
        if depth >= 64 {
            return Err("Math template nesting exceeds the entry limit".into());
        }
        let mut edits = Vec::new();
        let mut templates = self.doc.templates.iter().collect::<Vec<_>>();
        templates.sort_by_key(|t| (t.span.start, std::cmp::Reverse(t.span.end)));
        let mut covered_end = 0;
        for template in templates {
            if template.span.end <= covered_end {
                continue;
            }
            if let TemplateKind::Palette(form) = template.kind {
                if cas && form == palette::Form::Sexagesimal {
                    covered_end = template.span.end;
                }
                edits.extend(palette::evaluation_edits(
                    &self.doc.text,
                    template,
                    form,
                    cas,
                    depth,
                )?);
            }
        }
        edits.sort_by_key(|(range, _)| (range.start, std::cmp::Reverse(range.end)));
        let mut disjoint: Vec<(Range<usize>, String)> = Vec::new();
        for edit in edits {
            if let Some((previous, _)) = disjoint.last()
                && edit.0.start < previous.end
            {
                if edit.0.end <= previous.end {
                    continue;
                }
                return Err("Overlapping math template fields".into());
            }
            disjoint.push(edit);
        }
        disjoint.reverse();
        let mut text = self.doc.text.clone();
        for (range, replacement) in disjoint {
            text.replace_range(range, &replacement);
        }
        Ok(text)
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
    /// Algebraic entry edits the visible one-dimensional source, including
    /// delimiters. Textbook entry retains its structural deletion/navigation.
    pub fn apply_linear(&mut self, command: EditCommand) {
        use EditCommand::*;
        match command {
            Left(select) | Right(select) => {
                let right = matches!(command, Right(_));
                let selection = self.selection();
                let pos = if !select && !selection.is_empty() {
                    if right {
                        selection.end
                    } else {
                        selection.start
                    }
                } else {
                    self.adjacent(right)
                };
                self.set_cursor(pos, select);
            }
            Backspace | Delete => {
                let mut range = self.selection();
                if range.is_empty() {
                    // Preserve the user's atomic empty-call deletion preference
                    // in either entry mode, including recalled incomplete calls.
                    if let Some((span, text, cursor)) =
                        self.structural_deletion(command == Backspace)
                        && text.is_empty()
                    {
                        self.checkpoint();
                        self.replace(span, "");
                        self.doc.cursor = cursor;
                        return;
                    }
                    if command == Backspace {
                        range.start = self.adjacent(false);
                    } else {
                        range.end = self.adjacent(true);
                    }
                }
                if !range.is_empty() {
                    self.checkpoint();
                    self.replace(range, "");
                }
            }
            Home(select) => self.set_cursor(0, select),
            End(select) => self.set_cursor(self.doc.text.len(), select),
            _ => self.apply(command),
        }
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
                self.doc.anchor = Some(SelectionAnchor::plain(0));
                self.doc.cursor = self.doc.text.len();
            }
            Clear => self.set_text(""),
            Home(select) => self.set_textbook_cursor(0, select),
            End(select) => self.set_textbook_cursor(self.doc.text.len(), select),
            Left(select) | Right(select) => {
                let right = matches!(command, Right(_));
                let selection = self.selection();
                let position = if !select && !selection.is_empty() {
                    if right {
                        selection.end
                    } else {
                        selection.start
                    }
                } else if let Some(position) = self.stacked_absolute_opening_step(right) {
                    position
                } else if let Some(position) = self.fraction_factorial_step(right) {
                    position
                } else if !select && let Some(position) = self.powered_base_step(right) {
                    position
                } else if !select {
                    let jump = self.doc.templates.iter().rev().find_map(|t| {
                        if let TemplateKind::Palette(form) = t.kind
                            && !right
                            && self.doc.cursor == t.span.end
                        {
                            return t.fields.get(form.last_field(t.fields.len())).map(|f| f.end);
                        }
                        t.fields.iter().enumerate().find_map(|(i, f)| {
                            if right && self.doc.cursor == f.end {
                                Some(if t.kind == TemplateKind::Fraction {
                                    t.span.end
                                } else if let TemplateKind::Palette(form) = t.kind {
                                    form.neighbor(i, 1, 0, t.fields.len())
                                        .map_or(t.span.end, |n| t.fields[n].start)
                                } else {
                                    t.fields.get(i + 1).map_or(t.span.end, |n| n.start)
                                })
                            } else if !right && self.doc.cursor == f.start {
                                Some(if t.kind == TemplateKind::Fraction {
                                    t.span.start
                                } else if matches!(t.kind, TemplateKind::Function(name) if name != "ABS")
                                    && i == 0
                                    && self.doc.text[..f.start].ends_with('(')
                                {
                                    // The opening mark and the call name are separately editable.
                                    f.start - 1
                                } else if let TemplateKind::Palette(form) = t.kind {
                                    form.neighbor(i, -1, 0, t.fields.len())
                                        .map_or(t.span.start, |n| t.fields[n].end)
                                } else if i > 0 {
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
                self.set_textbook_cursor(position, select);
            }
            Backspace | Delete => {
                let mut range = self.selection();
                if range.is_empty() {
                    if command == Backspace
                        && let Some(position) = self.templates().iter().rev().find_map(|group| {
                            if group.span.end != self.cursor() {
                                return None;
                            }
                            self.fraction_factorial_operand(group)?
                                .fields
                                .get(1)
                                .map(|f| f.end)
                        })
                    {
                        self.set_textbook_cursor(position, false);
                        return;
                    }
                    if let Some((span, text, cursor)) =
                        self.structural_deletion(command == Backspace)
                    {
                        self.checkpoint();
                        self.replace(span, &text);
                        self.doc.cursor = cursor;
                        return;
                    }
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
    /// Math nodes are editing units. Backspace at an argument's left edge
    /// removes its operator/wrapper, preserving any entered operand. In
    /// particular an empty SIN() must never turn into the broken text SIN).
    fn structural_deletion(&self, backward: bool) -> Option<(Range<usize>, String, usize)> {
        if self.inside_string() {
            return None;
        }
        let recovered = recover_templates(&self.doc.text);
        let cursor = self.doc.cursor;
        let template = self
            .doc
            .templates
            .iter()
            .chain(recovered.iter())
            .filter(|template| {
                let empty = template.fields.iter().all(Range::is_empty);
                if backward {
                    template.fields.iter().any(|field| field.start == cursor)
                        || (empty && template.span.end == cursor)
                } else {
                    template.span.start == cursor
                }
            })
            // Recovered function/fraction nodes may enclose a parenthesis
            // placeholder created while typing; prefer the complete operation.
            .min_by_key(|template| {
                (
                    matches!(template.kind, TemplateKind::Parentheses) as u8,
                    !template.fields.iter().all(Range::is_empty),
                    if template.fields.iter().all(Range::is_empty) {
                        usize::MAX - template.span.len()
                    } else {
                        template.span.len()
                    },
                )
            });
        if let Some(template) = template {
            let text = template
                .fields
                .iter()
                .filter(|field| !field.is_empty())
                .map(|field| &self.doc.text[field.clone()])
                .collect::<Vec<_>>()
                .join(",");
            let start = if text.is_empty() && self.doc.text[..template.span.start].ends_with('*') {
                template.span.start - 1
            } else {
                template.span.start
            };
            let position = if backward
                && template
                    .fields
                    .first()
                    .is_some_and(|field| field.start != cursor)
            {
                start + text.len()
            } else {
                start
            };
            return Some((start..template.span.end, text, position));
        }
        // An incomplete function pasted/recalled without a closing delimiter
        // still denotes one calculator token, rather than editable name letters.
        if backward && cursor == self.doc.text.len() && self.doc.text[..cursor].ends_with('(') {
            let open = cursor - 1;
            let start = self.doc.text[..open]
                .char_indices()
                .rev()
                .take_while(|(_, ch)| ch.is_alphanumeric() || matches!(*ch, '_' | '→'))
                .last()
                .map(|(index, _)| index)?;
            if function_name(&self.doc.text[start..open]).is_some() {
                let start = if self.doc.text[..start].ends_with('*') {
                    start - 1
                } else {
                    start
                };
                return Some((start..cursor, String::new(), start));
            }
        }
        None
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

pub(super) fn parentheses_follow_name(source: &str, open: usize) -> bool {
    source[..open]
        .rsplit(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .next()
        .is_some_and(|name| {
            name.starts_with(|ch: char| {
                ch == '_' || ch.is_alphabetic() && !matches!(ch, 'π' | 'ⅈ' | 'ℯ')
            })
        })
}

fn function_name(name: &str) -> Option<&'static str> {
    ppl_compiler::builtins::BUILTINS
        .iter()
        .find(|spec| spec.name.eq_ignore_ascii_case(name))
        .map(|spec| spec.name)
}

/// Rebuild field structure when a history entry or durable document is opened.
/// The stored source remains authoritative; inferred nodes never rewrite it.
fn recover_templates(text: &str) -> Vec<Template> {
    let mut stack = Vec::new();
    let mut pairs = Vec::new();
    let mut quoted = false;
    let mut escaped = false;
    for (i, ch) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if quoted && ch == '\\' {
            escaped = true;
            continue;
        }
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
            .take_while(|(_, c)| c.is_alphanumeric() || matches!(*c, '_' | '→' | '.'))
            .last()
            .map_or(open, |(i, _)| i);
        let prefix = &text[prefix_start..open];
        let function = function_name(prefix);
        let form = palette::Form::from_call(prefix);
        let kind = match ch {
            '(' if form.is_some() => TemplateKind::Palette(form.unwrap()),
            '[' => TemplateKind::Matrix,
            '{' => TemplateKind::List,
            _ if prefix.eq_ignore_ascii_case("SQRT") => TemplateKind::Root,
            _ if prefix.eq_ignore_ascii_case("NTHROOT") => TemplateKind::NthRoot,
            _ if function.is_some() => TemplateKind::Function(function.unwrap()),
            _ => TemplateKind::Parentheses,
        };
        let start = if matches!(
            kind,
            TemplateKind::Function(_)
                | TemplateKind::Root
                | TemplateKind::NthRoot
                | TemplateKind::Palette(_)
        ) {
            prefix_start
        } else {
            open
        };
        let mut fields =
            if kind == TemplateKind::NthRoot || matches!(kind, TemplateKind::Palette(_)) {
                palette::fields(text, open + 1..close)
            } else {
                std::iter::once(open + 1..close).collect()
            };
        let mut kind = kind;
        if let TemplateKind::Palette(form) = kind {
            let compatible = match form {
                palette::Form::Derivative | palette::Form::LogBase => fields.len() == 2,
                palette::Form::Sexagesimal => {
                    palette::recover_arithmetic(text, open + 1..close, true).is_some()
                }
                palette::Form::Integral => matches!(fields.len(), 2 | 4),
                palette::Form::Limit => matches!(fields.len(), 3 | 4),
                _ => fields.len() == 4,
            };
            if !compatible {
                kind = function.map_or(TemplateKind::Parentheses, TemplateKind::Function);
            }
        }
        if kind == TemplateKind::Palette(palette::Form::Sexagesimal) {
            fields = palette::recover_arithmetic(text, open + 1..close, true).unwrap();
        }
        if ch == '(' && prefix.is_empty() {
            if let Some(parts) = palette::recover_arithmetic(text, open + 1..close, false) {
                kind = TemplateKind::Palette(palette::Form::MixedFraction);
                fields = parts;
            }
        }
        let mut start = start;
        if ch == '[' {
            if !prefix.is_empty() {
                kind = TemplateKind::Palette(palette::Form::Index);
                fields = vec![prefix_start..open, open + 1..close];
                start = prefix_start;
            } else if let Some(&(base, end, base_kind)) =
                pairs.iter().find(|(_, end, _)| *end + 1 == open)
            {
                let call_start = text[..base]
                    .char_indices()
                    .rev()
                    .take_while(|(_, c)| c.is_alphanumeric() || matches!(*c, '_' | '→' | '.'))
                    .last()
                    .map_or(base, |(i, _)| i);
                let call = base_kind == '(' && call_start < base;
                kind = TemplateKind::Palette(palette::Form::Index);
                fields = vec![
                    if call {
                        call_start..end + 1
                    } else if base_kind == '(' {
                        base + 1..end
                    } else {
                        base..end + 1
                    },
                    open + 1..close,
                ];
                start = if call { call_start } else { base };
            } else if let Some((form, parts)) = palette::recover_brackets(text, open, close) {
                kind = TemplateKind::Palette(form);
                fields = parts;
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
    // Grouping parentheses in a palette encoding are invisible syntax, not
    // additional nodes the cursor should visit between adjacent visible slots.
    let field_wrappers = templates
        .iter()
        .filter(|t| matches!(t.kind, TemplateKind::Palette(_)))
        .flat_map(|t| {
            t.fields.iter().filter_map(|f| {
                if f.start > 0
                    && text.as_bytes()[f.start - 1] == b'('
                    && text.as_bytes().get(f.end) == Some(&b')')
                {
                    Some(f.start - 1..f.end + 1)
                } else {
                    None
                }
            })
        })
        .collect::<Vec<_>>();
    templates.retain(|t| t.kind != TemplateKind::Parentheses || !field_wrappers.contains(&t.span));
    templates.sort_by_key(|t| (t.span.start, std::cmp::Reverse(t.span.end)));
    templates
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn textbook_powered_base_separates_the_closing_mark_from_the_outer_exponent() {
        let source = "((2)^(3))^(2)";
        let mut editor = ExpressionEditor::from_text(source);
        editor.set_cursor(8, false);
        for byte in [9, 11, 9, 8] {
            let right = editor.cursor() < byte;
            editor.apply(if right {
                EditCommand::Right(false)
            } else {
                EditCommand::Left(false)
            });
            assert_eq!(editor.cursor(), byte);
        }
        editor.apply(EditCommand::Right(true));
        assert_eq!(editor.selected_text(), ")");
        assert_eq!(editor.text(), source);
        editor.apply_linear(EditCommand::Right(false));
        editor.apply_linear(EditCommand::Right(false));
        assert_eq!(editor.cursor(), 10);
    }

    #[test]
    fn textbook_fraction_factorial_traverses_visible_fields_and_keeps_source() {
        let source = "((6)/(2))!";
        let mut editor = ExpressionEditor::from_text(source);
        for position in [9, 7, 6, 0, 0] {
            editor.apply(EditCommand::Left(false));
            assert_eq!(editor.cursor(), position);
        }
        for position in [6, 7, 9, 10, 10] {
            editor.apply(EditCommand::Right(false));
            assert_eq!(editor.cursor(), position);
        }
        editor.apply(EditCommand::Left(true));
        assert_eq!(editor.selected_text(), "!");
        editor.apply(EditCommand::Right(true));
        assert!(editor.selection().is_empty());
        editor.apply(EditCommand::Left(false));
        for _ in 0..3 {
            editor.apply(EditCommand::Left(true));
        }
        assert_eq!(editor.selected_text(), "((6)/(2))");
        editor.set_textbook_cursor(9, false);
        editor.apply(EditCommand::Backspace);
        assert_eq!(editor.cursor(), 7);
        assert_eq!(editor.text(), source);
        assert!(editor.vertical_field(false));
        assert_eq!(editor.cursor(), 3);

        editor.set_cursor(9, false);
        editor.apply_linear(EditCommand::Left(false));
        assert_eq!(editor.cursor(), 8);
    }

    #[test]
    fn stacked_absolute_bar_selection_preserves_the_complete_expression() {
        let source = "ABS((-3)/(2))";
        let mut editor = ExpressionEditor::from_text(source);
        editor.apply(EditCommand::Home(false));
        editor.apply(EditCommand::Right(false));
        assert_eq!(editor.cursor(), 4);
        editor.apply(EditCommand::Left(true));
        assert_eq!(editor.cursor(), 0);
        assert_eq!(editor.selected_text(), source);
        editor.apply(EditCommand::Right(true));
        assert_eq!(editor.cursor(), 4);
        assert!(editor.selection().is_empty());

        editor.apply(EditCommand::End(false));
        editor.apply(EditCommand::Left(true));
        assert_eq!(editor.cursor(), 12);
        assert_eq!(editor.selected_text(), source);
        editor.apply(EditCommand::Right(true));
        assert!(editor.selection().is_empty());

        editor.set_textbook_cursor(4, false);
        editor.set_textbook_cursor(12, true);
        assert_eq!(editor.selected_text(), "(-3)/(2)");
        editor.set_textbook_cursor(10, false);
        editor.set_textbook_cursor(11, true);
        assert_eq!(editor.selected_text(), "2");

        editor.apply_linear(EditCommand::Home(false));
        editor.apply_linear(EditCommand::Right(true));
        assert_eq!(editor.selected_text(), "A");
        let mut short = ExpressionEditor::from_text("ABS(-3)");
        short.apply(EditCommand::Left(true));
        assert_eq!(short.selected_text(), ")");
    }

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
        assert!(e.vertical_field(true));
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
    fn explicit_opening_delimiters_nest_inside_calls_and_collection_indices() {
        for source in [
            "STRING(L1(2))",
            "SIN(COS(0))",
            "SUM({L1(1),L1(2)})",
            "F(G(H(2)))",
        ] {
            let mut editor = ExpressionEditor::default();
            editor.type_text(source);
            assert_eq!(editor.text(), source);
            assert_eq!(editor.cursor(), source.len());
            editor.apply(EditCommand::Undo);
            assert_eq!(editor.text(), "");
            editor.apply(EditCommand::Redo);
            assert_eq!(editor.text(), source);
        }
        let mut editor = ExpressionEditor::default();
        editor.type_text("STRING(");
        editor.type_text("L1(");
        assert_eq!(editor.text(), "STRING(L1())");
        editor.type_text("2))");
        assert_eq!(editor.text(), "STRING(L1(2))");
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
        e.set_cursor(1, false);
        assert!(e.vertical_field(true));
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
    fn vertical_template_navigation_chooses_inner_fields_and_preserves_unicode_boundaries() {
        let mut e = ExpressionEditor::from_text("(π)/(12)");
        e.set_cursor(3, false);
        assert!(e.vertical_field(true));
        assert_eq!(e.cursor(), 7);
        assert!(e.vertical_field(false));
        assert_eq!(e.cursor(), 3);
        let mut power = ExpressionEditor::from_text("(x)^(23)");
        power.set_cursor(2, false);
        assert!(power.vertical_field(false));
        assert_eq!(power.cursor(), 6);
        let mut root = ExpressionEditor::from_text("NTHROOT(3,8)");
        root.set_cursor(11, false);
        assert!(!root.vertical_field(false));
        assert_eq!(root.cursor(), 11);
        root.set_cursor(9, false);
        assert!(!root.vertical_field(true));
        assert_eq!(root.cursor(), 9);
    }
    #[test]
    fn calculator_templates_insert_implied_products_without_consuming_a_root_degree() {
        let mut editor = ExpressionEditor::from_text("3");
        editor.insert_template(TemplateKind::NthRoot);
        editor.insert("8");
        assert_eq!(editor.text(), "3*NTHROOT(8,)");
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
    fn backspace_removes_empty_keypad_and_typed_functions_as_one_undoable_edit() {
        for name in [
            "SIN", "ASIN", "LOG", "SQRT", "ABS", "CEILING", "→HMS", "HMS→", "B→R",
        ] {
            let mut keypad = ExpressionEditor::default();
            keypad.insert_template(TemplateKind::Function(name));
            let before = keypad.doc.clone();
            keypad.apply(EditCommand::Backspace);
            assert_eq!(keypad.text(), "", "{name}");
            keypad.apply(EditCommand::Undo);
            assert_eq!(keypad.doc, before);
            keypad.apply(EditCommand::Redo);
            assert_eq!(keypad.text(), "");
        }
        for source in [
            "sin(", "cos(", "sqrt(", "asin(", "ceiling(", "→hms(", "HMS→(",
        ] {
            let mut typed = ExpressionEditor::default();
            typed.type_text(source);
            typed.apply(EditCommand::Backspace);
            assert_eq!(typed.text(), "", "{source}");
            let mut pasted = ExpressionEditor::from_text(source);
            pasted.apply(EditCommand::Backspace);
            assert_eq!(pasted.text(), "", "pasted {source}");
        }
        let mut product = ExpressionEditor::from_text("2");
        product.insert_template(TemplateKind::Function("SIN"));
        product.apply(EditCommand::Backspace);
        assert_eq!(product.text(), "2");
        assert_eq!(product.cursor(), 1);
        product.apply(EditCommand::Undo);
        assert_eq!(product.text(), "2*SIN()");
        let mut typed = ExpressionEditor::default();
        typed.type_text("2sin(");
        typed.apply(EditCommand::Backspace);
        assert_eq!(typed.text(), "2");
    }
    #[test]
    fn nested_deletion_keeps_outer_fields_operands_and_cursor() {
        let mut editor = ExpressionEditor::default();
        editor.insert_template(TemplateKind::Function("SIN"));
        editor.insert_template(TemplateKind::Root);
        editor.apply(EditCommand::Backspace);
        assert_eq!(editor.text(), "SIN()");
        assert_eq!(editor.cursor(), 4);
        editor.insert("9");
        editor.set_cursor(4, false);
        editor.apply(EditCommand::Backspace);
        assert_eq!(editor.text(), "9");
        assert_eq!(editor.cursor(), 0);
        for template in [TemplateKind::Fraction, TemplateKind::Power] {
            let mut editor = ExpressionEditor::from_text("12");
            editor.insert_template(template);
            editor.apply(EditCommand::Backspace);
            assert_eq!(editor.text(), "12");
            assert_eq!(editor.cursor(), 2);
            editor.apply(EditCommand::Undo);
            assert_eq!(
                editor.text(),
                if template == TemplateKind::Fraction {
                    "(12)/()"
                } else {
                    "(12)^()"
                }
            );
        }
    }
    #[test]
    fn deleting_text_selections_and_digits_does_not_consume_math_nodes() {
        let mut editor = ExpressionEditor::from_text("SIN(123)");
        editor.set_cursor(6, false);
        editor.apply(EditCommand::Backspace);
        assert_eq!(editor.text(), "SIN(13)");
        editor.set_cursor(4, false);
        editor.apply(EditCommand::Delete);
        assert_eq!(editor.text(), "SIN(3)");
        editor.set_cursor(0, false);
        editor.set_cursor(1, true);
        editor.apply(EditCommand::Backspace);
        assert_eq!(editor.text(), "IN(3)");
        let mut literal = ExpressionEditor::from_text("\"say \\\"sin()\"");
        let position = literal.text().find(')').unwrap();
        literal.set_cursor(position, false);
        literal.apply(EditCommand::Backspace);
        assert_eq!(literal.text(), "\"say \\\"sin)\"");
        assert!(literal.templates().is_empty());
        let mut unknown = ExpressionEditor::from_text("CUSTOM(");
        unknown.apply(EditCommand::Backspace);
        assert_eq!(unknown.text(), "CUSTOM");
    }
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

#[cfg(test)]
mod measurement_entry_tests {
    use super::ExpressionEditor;
    #[test]
    fn unit_suffixes_keep_their_letters_while_ordinary_implicit_products_remain() {
        for source in [
            "20_cm+5_in",
            "CONVERT(5_m,1_ft)",
            "2_µm",
            "2_m/s",
            "3_min+1_h",
        ] {
            let mut editor = ExpressionEditor::default();
            editor.type_text(source);
            assert_eq!(editor.text(), source);
        }
        let mut editor = ExpressionEditor::default();
        editor.type_text("2π+3X");
        assert_eq!(editor.text(), "2*π+3*X");
    }
}

#[cfg(test)]
mod signed_word_entry_tests {
    use super::*;
    #[test]
    fn typed_signed_words_keep_digits_width_and_suffix_in_one_token() {
        for source in [
            "#FF:-8h",
            "#−5:-8h",
            "#−AFh",
            "#FFFFFFFFFFFFFFFF:64h",
            "Setbase(#101B:16h,0)",
        ] {
            let mut e = ExpressionEditor::default();
            e.type_text(source);
            assert_eq!(e.text(), source);
            let (_, diagnostics) = ppl_syntax::parse_expression(e.text());
            assert!(!diagnostics.has_errors(), "{source}");
        }
        let mut e = ExpressionEditor::from_text("#FF:-8h");
        e.type_text("X");
        assert_eq!(e.text(), "#FF:-8h*X");
    }
}
