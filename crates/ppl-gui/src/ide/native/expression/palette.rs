//! The 18 calculator math forms in G1 palette order. Source remains executable;
//! field geometry is supplied by the common textbook renderer.
use super::{Template, TemplateKind};
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    Index,
    Derivative,
    Piecewise,
    Matrix { rows: usize, cols: usize },
    Limit,
    Integral,
    RowVector,
    ColumnVector,
    MixedFraction,
    Sum,
    Product,
    LogBase,
    Sexagesimal,
}
#[derive(Debug, Clone, Copy)]
pub struct Cell {
    pub name: &'static str,
    pub kind: TemplateKind,
}
pub const CELLS: &[Cell; 18] = &[
    Cell {
        name: "Fraction",
        kind: TemplateKind::Fraction,
    },
    Cell {
        name: "Power",
        kind: TemplateKind::Power,
    },
    Cell {
        name: "Subscript",
        kind: TemplateKind::Palette(Form::Index),
    },
    Cell {
        name: "Derivative",
        kind: TemplateKind::Palette(Form::Derivative),
    },
    Cell {
        name: "Piecewise",
        kind: TemplateKind::Palette(Form::Piecewise),
    },
    Cell {
        name: "Matrix",
        kind: TemplateKind::Palette(Form::Matrix { rows: 2, cols: 2 }),
    },
    Cell {
        name: "Square root",
        kind: TemplateKind::Root,
    },
    Cell {
        name: "Nth root",
        kind: TemplateKind::NthRoot,
    },
    Cell {
        name: "Limit",
        kind: TemplateKind::Palette(Form::Limit),
    },
    Cell {
        name: "Integral",
        kind: TemplateKind::Palette(Form::Integral),
    },
    Cell {
        name: "Row vector",
        kind: TemplateKind::Palette(Form::RowVector),
    },
    Cell {
        name: "Column vector",
        kind: TemplateKind::Palette(Form::ColumnVector),
    },
    Cell {
        name: "Absolute value",
        kind: TemplateKind::Function("ABS"),
    },
    Cell {
        name: "Mixed fraction",
        kind: TemplateKind::Palette(Form::MixedFraction),
    },
    Cell {
        name: "Sum",
        kind: TemplateKind::Palette(Form::Sum),
    },
    Cell {
        name: "Product",
        kind: TemplateKind::Palette(Form::Product),
    },
    Cell {
        name: "Logarithm with base",
        kind: TemplateKind::Palette(Form::LogBase),
    },
    Cell {
        name: "Sexagesimal",
        kind: TemplateKind::Palette(Form::Sexagesimal),
    },
];
#[derive(Default)]
struct Builder {
    text: String,
    fields: Vec<Range<usize>>,
}
impl Builder {
    fn text(&mut self, s: &str) {
        self.text.push_str(s);
    }
    fn field(&mut self, s: &str) {
        let start = self.text.len();
        self.text(s);
        self.fields.push(start..self.text.len());
    }
    fn finish(self) -> (String, Vec<Range<usize>>) {
        (self.text, self.fields)
    }
}
impl Form {
    pub fn call(self) -> Option<(&'static str, usize, usize)> {
        Some(match self {
            Self::Derivative => ("CAS.diff", 2, 0),
            Self::Piecewise => ("CAS.piecewise", 4, 0),
            Self::Limit => ("CAS.limit", 4, 1),
            Self::Integral => ("CAS.integrate", 4, 2),
            Self::Sum => ("CAS.sum", 4, 0),
            Self::Product => ("CAS.product", 4, 0),
            Self::LogBase => ("LOG", 2, 0),
            _ => return None,
        })
    }
    pub fn build(self, selected: &str) -> (String, Vec<Range<usize>>) {
        let mut b = Builder::default();
        if let Some((name, count, _)) = self.call() {
            b.text(name);
            b.text("(");
            for i in 0..count {
                if i > 0 {
                    b.text(",");
                }
                b.field(if i == if self == Self::Piecewise { 1 } else { 0 } {
                    selected
                } else {
                    ""
                });
            }
            b.text(")");
            return b.finish();
        }
        match self {
            Self::Index => {
                b.text("(");
                b.field(selected);
                b.text(")[");
                b.field("");
                b.text("]");
            }
            Self::Matrix { rows, cols } => {
                b.text("[");
                for row in 0..rows {
                    if row > 0 {
                        b.text(",");
                    }
                    b.text("[");
                    for col in 0..cols {
                        if col > 0 {
                            b.text(",");
                        }
                        b.field(if row == 0 && col == 0 { selected } else { "" });
                    }
                    b.text("]");
                }
                b.text("]");
            }
            Self::RowVector => {
                b.text("[");
                b.field(selected);
                b.text(",");
                b.field("");
                b.text("]");
            }
            Self::ColumnVector => {
                b.text("[[");
                b.field(selected);
                b.text("],[");
                b.field("");
                b.text("]]");
            }
            Self::MixedFraction => {
                b.text("((");
                b.field(selected);
                b.text(")+(");
                b.field("");
                b.text(")/(");
                b.field("");
                b.text("))");
            }
            Self::Sexagesimal => {
                b.text("→HMS((");
                b.field(selected);
                b.text(")+(");
                b.field("");
                b.text(")/60+(");
                b.field("");
                b.text(")/3600)");
            }
            _ => unreachable!(),
        }
        b.finish()
    }
    pub fn from_call(name: &str) -> Option<Self> {
        let name = name.strip_prefix("CAS.").unwrap_or(name);
        Some(match name {
            "diff" => Self::Derivative,
            "piecewise" => Self::Piecewise,
            "limit" => Self::Limit,
            "integrate" | "int" => Self::Integral,
            "sum" => Self::Sum,
            "product" => Self::Product,
            "LOG" => Self::LogBase,
            "→HMS" => Self::Sexagesimal,
            _ => return None,
        })
    }
}
/// Top-level arguments, retaining exact UTF-8 offsets and quoted commas.
pub fn fields(text: &str, range: Range<usize>) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut begin = range.start;
    let mut depth = 0;
    let mut quoted = false;
    let mut escape = false;
    for (offset, ch) in text[range.clone()].char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if quoted && ch == '\\' {
            escape = true;
            continue;
        }
        if ch == '"' {
            quoted = !quoted;
            continue;
        }
        if quoted {
            continue;
        }
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                let end = range.start + offset;
                out.push(begin..end);
                begin = end + 1;
            }
            _ => {}
        }
    }
    out.push(begin..range.end);
    out
}
pub fn recover_brackets(
    text: &str,
    open: usize,
    close: usize,
) -> Option<(Form, Vec<Range<usize>>)> {
    let parts = fields(text, open + 1..close);
    let rows = parts
        .iter()
        .map(|r| {
            let trimmed = text[r.clone()].trim();
            if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
                return None;
            }
            let start = r.start + text[r.clone()].find('[')?;
            let end = r.start + text[r.clone()].rfind(']')?;
            Some(fields(text, start + 1..end))
        })
        .collect::<Option<Vec<_>>>();
    if let Some(rows) = rows {
        let count = rows.len();
        let cols = rows.first()?.len();
        if cols > 0 && rows.iter().all(|r| r.len() == cols) {
            let form = if cols == 1 && count == 2 {
                Form::ColumnVector
            } else {
                Form::Matrix { rows: count, cols }
            };
            return Some((form, rows.into_iter().flatten().collect()));
        }
    }
    (!parts.is_empty()).then_some((Form::RowVector, parts))
}
pub fn evaluation_edits(
    text: &str,
    t: &Template,
    form: Form,
    cas: bool,
    depth: usize,
) -> Result<Vec<(Range<usize>, String)>, String> {
    if form == Form::Sexagesimal && t.fields.len() == 3 && cas {
        if t.fields.iter().any(|r| text[r.clone()].trim().is_empty()) {
            return Err("Complete the required math template fields".into());
        }
        let values = t
            .fields
            .iter()
            .map(|r| {
                super::ExpressionEditor::from_text(&text[r.clone()])
                    .evaluation_text_depth(cas, depth + 1)
            })
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(vec![(
            t.span.clone(),
            format!("({})+({})/60+({})/3600", values[0], values[1], values[2]),
        )]);
    }
    if form == Form::Index && t.fields.iter().any(|r| text[r.clone()].trim().is_empty()) {
        return Err("Complete the required math template fields".into());
    }
    let mut edits = Vec::new();
    if let Some((_, total, optional)) = form.call() {
        let optional_empty = optional > 0
            && t.fields.len() == total
            && t.fields[total - optional..]
                .iter()
                .all(|r| text[r.clone()].trim().is_empty());
        let required = if optional_empty {
            total - optional
        } else {
            t.fields.len()
        };
        if t.fields[..required]
            .iter()
            .any(|r| text[r.clone()].trim().is_empty())
        {
            return Err("Complete the required math template fields".into());
        }
        if optional_empty {
            edits.push((
                t.fields[total - optional].start - 1..t.span.end - 1,
                String::new(),
            ));
        }
        if cas && text[t.span.clone()].starts_with("CAS.") {
            edits.push((t.span.start..t.span.start + 4, String::new()));
        }
    }
    Ok(edits)
}

impl Form {
    pub fn neighbor(self, index: usize, dx: i32, dy: i32, count: usize) -> Option<usize> {
        let dynamic;
        let positions: &[(i32, i32)] = match self {
            Self::Matrix { rows: _, cols } => {
                dynamic = (0..count)
                    .map(|i| ((i % cols) as i32, (i / cols) as i32))
                    .collect::<Vec<_>>();
                &dynamic
            }
            Self::RowVector => {
                dynamic = (0..count).map(|i| (i as i32, 0)).collect::<Vec<_>>();
                &dynamic
            }
            Self::Index => &[(0, 0), (1, 1)],
            Self::Derivative => &[(0, 0), (0, 1)],
            Self::Piecewise => &[(1, 0), (0, 0), (1, 1), (0, 1)],
            Self::Limit => &[(2, 0), (0, 1), (1, 1), (2, 1)],
            Self::Integral => &[(1, 0), (2, 0), (0, 1), (0, -1)],
            Self::ColumnVector => &[(0, 0), (0, 1)],
            Self::MixedFraction => &[(0, 0), (1, 0), (1, 1)],
            Self::Sum | Self::Product => &[(2, 0), (0, 1), (1, 1), (0, -1)],
            Self::LogBase => &[(1, 0), (0, 1)],
            Self::Sexagesimal => &[(0, 0), (1, 0), (2, 0)],
        };
        let &(x, y) = positions.get(index)?;
        positions
            .iter()
            .take(count)
            .enumerate()
            .filter_map(|(i, &(nx, ny))| {
                let a = (nx - x) * dx + (ny - y) * dy;
                if a <= 0 {
                    return None;
                }
                let cross = if dx != 0 {
                    (ny - y).abs()
                } else {
                    (nx - x).abs()
                };
                Some((cross * 4 + a, i))
            })
            .min()
            .map(|(_, i)| i)
    }
}

impl Form {
    pub fn last_field(self, count: usize) -> usize {
        match self {
            Self::Limit | Self::Sum | Self::Product | Self::LogBase => 0,
            Self::Integral => 1,
            Self::Piecewise => 2,
            _ => count.saturating_sub(1),
        }
    }
    pub fn initial_field(self) -> usize {
        match self {
            Self::LogBase | Self::Piecewise | Self::Sum | Self::Product | Self::Limit => 1,
            Self::Integral => 2,
            _ => 0,
        }
    }
}

/// Parenthesized top-level components of the palette's arithmetic encodings.
pub fn groups(text: &str, range: Range<usize>) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut begin = range.start;
    let mut quoted = false;
    let mut escaped = false;
    for (offset, ch) in text[range.clone()].char_indices() {
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
        if ch == '(' {
            if depth == 0 {
                begin = range.start + offset + 1;
            }
            depth += 1;
        }
        if ch == ')' {
            depth -= 1;
            if depth == 0 {
                out.push(begin..range.start + offset);
            }
        }
    }
    out
}
pub fn recover_arithmetic(
    text: &str,
    range: Range<usize>,
    sexagesimal: bool,
) -> Option<Vec<Range<usize>>> {
    let parts = groups(text, range.clone());
    if parts.len() != 3 {
        return None;
    }
    if parts[0].start != range.start + 1 {
        return None;
    }
    let middle1 = text[parts[0].end + 1..parts[1].start - 1].trim();
    let middle2 = text[parts[1].end + 1..parts[2].start - 1].trim();
    let tail = text[parts[2].end + 1..range.end].trim();
    if middle1 == "+"
        && ((sexagesimal && middle2 == "/60+" && tail == "/3600")
            || (!sexagesimal && middle2 == "/" && tail.is_empty()))
    {
        Some(parts)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ide::native::expression::{EditCommand, ExpressionEditor};

    fn filled(kind: TemplateKind, values: &[&str]) -> ExpressionEditor {
        let mut e = ExpressionEditor::default();
        e.insert_template(kind);
        for (i, value) in values.iter().enumerate().rev() {
            let field = e.templates().last().unwrap().fields[i].clone();
            e.set_cursor(field.start, false);
            e.insert(value);
        }
        e
    }
    #[test]
    fn all_eighteen_forms_keep_executable_source_and_recover_editable_fields() {
        let cases: &[(&[&str], &str)] = &[
            (&["1", "2"], "(1)/(2)"),
            (&["2", "3"], "(2)^(3)"),
            (&["L1", "2"], "(L1)[2]"),
            (&["x^2", "x"], "CAS.diff(x^2,x)"),
            (&["1>0", "2", "1<0", "3"], "CAS.piecewise(1>0,2,1<0,3)"),
            (&["1", "2", "3", "4"], "[[1,2],[3,4]]"),
            (&["9"], "SQRT(9)"),
            (&["3", "8"], "NTHROOT(3,8)"),
            (&["SIN(x)/x", "x", "0", ""], "CAS.limit(SIN(x)/x,x,0)"),
            (&["x", "x", "0", "1"], "CAS.integrate(x,x,0,1)"),
            (&["2", "3"], "[2,3]"),
            (&["2", "3"], "[[2],[3]]"),
            (&["-2"], "ABS(-2)"),
            (&["2", "1", "3"], "((2)+(1)/(3))"),
            (&["k", "k", "1", "5"], "CAS.sum(k,k,1,5)"),
            (&["k", "k", "1", "4"], "CAS.product(k,k,1,4)"),
            (&["8", "2"], "LOG(8,2)"),
            (&["12", "30", "0"], "→HMS((12)+(30)/60+(0)/3600)"),
        ];
        for (cell, (values, source)) in CELLS.iter().zip(cases) {
            let e = filled(cell.kind, values);
            assert_eq!(e.evaluation_text(false).unwrap(), *source, "{}", cell.name);
            let recalled = ExpressionEditor::from_text(e.text());
            assert_eq!(recalled.text(), e.text());
            let t = recalled
                .templates()
                .iter()
                .find(|t| t.kind == cell.kind)
                .unwrap_or_else(|| {
                    panic!("{} lost its form: {:?}", cell.name, recalled.templates())
                });
            assert_eq!(
                t.fields
                    .iter()
                    .map(|r| &e.text()[r.clone()])
                    .collect::<Vec<_>>(),
                *values,
                "{}",
                cell.name
            );
            assert_eq!(recalled.evaluation_text(false).unwrap(), *source);
        }
    }
    #[test]
    fn calculus_optional_fields_lower_without_changing_history_or_cursor() {
        for (form, values, home, cas) in [
            (
                Form::Integral,
                vec!["x", "x", "", ""],
                "CAS.integrate(x,x)",
                "integrate(x,x)",
            ),
            (
                Form::Limit,
                vec!["SIN(x)/x", "x", "0", ""],
                "CAS.limit(SIN(x)/x,x,0)",
                "limit(SIN(x)/x,x,0)",
            ),
        ] {
            let e = filled(TemplateKind::Palette(form), &values);
            let before = e.doc.clone();
            assert_eq!(e.evaluation_text(false).unwrap(), home);
            assert_eq!(e.evaluation_text(true).unwrap(), cas);
            assert_eq!(e.doc, before);
        }
        let e = filled(TemplateKind::Palette(Form::Integral), &["x", "x", "0", ""]);
        assert!(e.evaluation_text(false).is_err());
        let e = ExpressionEditor::from_text("CAS.limit(x,x,0)+CAS.integrate(x,x,,)");
        assert_eq!(
            e.evaluation_text(true).unwrap(),
            "limit(x,x,0)+integrate(x,x)"
        );
    }
    #[test]
    fn nested_cas_and_sexagesimal_normalization_respects_quotes_and_empty_vectors() {
        let e = filled(
            TemplateKind::Palette(Form::Sexagesimal),
            &["CAS.integrate(x,x,,)", "30", "0"],
        );
        assert_eq!(
            e.evaluation_text(true).unwrap(),
            "(integrate(x,x))+(30)/60+(0)/3600"
        );
        assert_eq!(
            ExpressionEditor::from_text("[]")
                .evaluation_text(false)
                .unwrap(),
            "[]"
        );
        let e = ExpressionEditor::from_text("CAS.piecewise(1,\"a,b\",0,\"(x)\")");
        assert_eq!(
            e.evaluation_text(true).unwrap(),
            "piecewise(1,\"a,b\",0,\"(x)\")"
        );
        assert!(
            ExpressionEditor::from_text("()[2]")
                .evaluation_text(false)
                .is_err()
        );
        assert_eq!(
            ExpressionEditor::from_text("(L1)[2]")
                .evaluation_text(false)
                .unwrap(),
            "(L1)[2]"
        );
        for source in ["L1[2]", "MAKELIST(X,X,1,3)[2]", "[1,2][1]"] {
            let e = ExpressionEditor::from_text(source);
            assert_eq!(e.evaluation_text(false).unwrap(), source);
            let t = e
                .templates()
                .iter()
                .find(|t| t.kind == TemplateKind::Palette(Form::Index))
                .unwrap();
            assert_eq!(t.span, 0..source.len());
        }
    }
    #[test]
    fn fresh_forms_focus_the_g1_field_and_empty_backspace_undo_is_atomic() {
        for cell in CELLS {
            let mut e = ExpressionEditor::default();
            e.insert_template(cell.kind);
            let t = e.templates().last().unwrap();
            let first = match cell.kind {
                TemplateKind::Palette(f) => f.initial_field(),
                _ => 0,
            };
            assert_eq!(e.cursor(), t.fields[first].start, "{}", cell.name);
            let source = e.text().to_owned();
            e.apply(EditCommand::Backspace);
            assert_eq!(e.text(), "", "{}", cell.name);
            e.apply(EditCommand::Undo);
            assert_eq!(e.text(), source);
        }
    }
    #[test]
    fn comma_growth_rebuilds_rectangular_matrix_and_vector_fields() {
        let mut e = ExpressionEditor::default();
        e.insert_template(TemplateKind::Palette(Form::RowVector));
        e.type_text("1,2");
        e.apply(EditCommand::Right(false));
        e.type_text("3");
        assert_eq!(e.text(), "[1,2,3]");
        let t = e
            .templates()
            .iter()
            .find(|t| t.kind == TemplateKind::Palette(Form::RowVector))
            .unwrap();
        assert_eq!(t.fields.len(), 3);
        let e = ExpressionEditor::from_text("[[1,2,3],[4,5,6],[7,8,9]]");
        let t = e
            .templates()
            .iter()
            .find(|t| t.kind == TemplateKind::Palette(Form::Matrix { rows: 3, cols: 3 }))
            .unwrap();
        assert_eq!(t.fields.len(), 9);
        assert_eq!(
            Form::Matrix { rows: 3, cols: 3 }.neighbor(1, 0, 1, 9),
            Some(4)
        );
    }
    #[test]
    fn typing_and_recall_skip_the_hidden_arithmetic_delimiters() {
        for (form, a, b, c, source) in [
            (Form::MixedFraction, "2", "1", "3", "((2)+(1)/(3))"),
            (
                Form::Sexagesimal,
                "12",
                "30",
                "0",
                "→HMS((12)+(30)/60+(0)/3600)",
            ),
        ] {
            let mut e = ExpressionEditor::default();
            e.insert_template(TemplateKind::Palette(form));
            e.type_text(a);
            e.apply(EditCommand::Right(false));
            e.type_text(b);
            if form == Form::MixedFraction {
                assert!(e.vertical_field(true));
            } else {
                e.apply(EditCommand::Right(false));
            }
            e.type_text(c);
            assert_eq!(e.text(), source);
            e = ExpressionEditor::from_text(source);
            e.apply(EditCommand::Left(false));
            let t = e
                .templates()
                .iter()
                .find(|t| t.kind == TemplateKind::Palette(form))
                .unwrap();
            assert_eq!(e.cursor(), t.fields[2].end);
            e.type_text("1");
            assert!(e.evaluation_text(false).is_ok());
        }
    }
}
