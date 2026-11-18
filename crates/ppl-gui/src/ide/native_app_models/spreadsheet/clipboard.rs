//! Worksheet clipboard snapshots keep typed values and relative references.
use super::{
    Address, Book, Evaluator, MAX_COLUMNS, MAX_POPULATED_CELLS, MAX_ROWS, Spreadsheet, cell_name,
    format, identifier_end, number_end, quoted_end,
};
use ppl_math::Real;
use ppl_syntax::ast::ExprKind;
use ppl_vm::{PortableValue, SessionSnapshot, Value};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, atomic::AtomicBool};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasteMode {
    Values,
    Formulas,
    Format,
    ValuesAndFormat,
    FormulasAndFormat,
}
impl PasteMode {
    pub const CHOICES: [(Self, &'static str); 5] = [
        (Self::Values, "Values"),
        (Self::Formulas, "Formulas"),
        (Self::Format, "Formatting"),
        (Self::ValuesAndFormat, "Values and formatting"),
        (Self::FormulasAndFormat, "Formulas and formatting"),
    ];
    fn takes_formulas(self) -> bool {
        matches!(self, Self::Formulas | Self::FormulasAndFormat)
    }
    fn takes_format(self) -> bool {
        matches!(
            self,
            Self::Format | Self::ValuesAndFormat | Self::FormulasAndFormat
        )
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClipboardBlock {
    origin: format::Block,
    entries: Vec<ClipboardCell>,
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ClipboardCell {
    value: Option<serde_json::Value>,
    formula: Option<String>,
    #[serde(default)]
    inherited: bool,
    format: format::CellFormat,
}

impl ClipboardBlock {
    pub fn rows(&self) -> usize {
        self.origin.bottom - self.origin.top + 1
    }
    pub fn columns(&self) -> usize {
        self.origin.right - self.origin.left + 1
    }
}

impl Spreadsheet {
    pub(crate) fn is_clipboard_collection(text: &str) -> bool {
        let text = text.trim();
        if !text.starts_with(['{', '[']) {
            return false;
        }
        let (expression, diagnostics) = ppl_syntax::parse_expression(text);
        !diagnostics.has_errors()
            && matches!(expression.kind, ExprKind::List(_) | ExprKind::Bracket(_))
    }

    pub fn paste_collection(
        self,
        destination: [usize; 2],
        text: &str,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
        geometry: Option<Arc<Book>>,
    ) -> Result<(Self, Option<format::Block>), String> {
        let [col, row] = destination;
        let value = {
            let mut evaluator = Evaluator::new(&self, snapshot, cancel.clone(), geometry)?;
            evaluator.expression(text.trim(), Address { col, row }, 0)?
        };
        let (rows, columns, values): (_, _, &[Value]) = match &value {
            Value::List(items) => (items.len(), 1, items),
            Value::Matrix(matrix) if matrix.rows == 1 => (matrix.data.len(), 1, &matrix.data),
            Value::Matrix(matrix) => (matrix.rows, matrix.cols, &matrix.data),
            _ => return Err("Clipboard text does not produce a list or matrix".into()),
        };
        if values.is_empty() {
            return Ok((self, None));
        }
        if values.len() > MAX_POPULATED_CELLS {
            return Err("Paste up to 10000 worksheet cells".into());
        }
        let clipboard = ClipboardBlock {
            origin: format::Block {
                left: 1,
                top: 1,
                right: columns,
                bottom: rows,
            },
            text: String::new(),
            entries: values
                .iter()
                .map(|value| ClipboardCell {
                    value: Some(ppl_vm::session::encode_value(&PortableValue::from(value))),
                    formula: None,
                    inherited: false,
                    format: format::CellFormat::default(),
                })
                .collect(),
        };
        let sheet = self.paste_block(col, row, &clipboard, PasteMode::Values, &cancel)?;
        Ok((
            sheet,
            Some(format::Block {
                left: col,
                right: col + columns - 1,
                top: row,
                bottom: row + rows - 1,
            }),
        ))
    }

    pub fn copy_block(
        &self,
        block: format::Block,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
        geometry: Option<Arc<Book>>,
    ) -> Result<ClipboardBlock, String> {
        block.validate()?;
        let count = (block.bottom - block.top + 1) * (block.right - block.left + 1);
        if count > MAX_POPULATED_CELLS {
            return Err("Select up to 10000 cells to copy".into());
        }
        let mut evaluator = Evaluator::new(self, snapshot, cancel, geometry)?;
        let mut entries = Vec::with_capacity(count);
        let mut text = String::new();
        for row in block.top..=block.bottom {
            if row != block.top {
                text.push('\n');
            }
            for col in block.left..=block.right {
                evaluator.check()?;
                if col != block.left {
                    text.push('\t');
                }
                let name = cell_name(col, row);
                let value = if self.has_value(col, row) {
                    let value = evaluator.cell(&name)?;
                    let style = self.format(col, row);
                    let mut settings = evaluator.context.settings.clone();
                    settings.set_format(
                        style.number.filter(|v| *v >= 0).unwrap_or(settings.hformat),
                        style.digits.filter(|v| *v >= 0).unwrap_or(settings.hdigits),
                    );
                    if let Value::Str(s) = &value {
                        if style.quotes == Some(1) {
                            text.push('"');
                            text.push_str(s);
                            text.push('"');
                        } else {
                            text.push_str(s);
                        }
                    } else {
                        text.push_str(&value.string_form(settings.home_format()));
                    }
                    Some(ppl_vm::session::encode_value(&PortableValue::from(&value)))
                } else {
                    None
                };
                let formula = self
                    .cells
                    .get(&name)
                    .and_then(|source| source.trim().strip_prefix('='))
                    .filter(|_| !self.entered_values.contains_key(&name))
                    .map(str::to_owned);
                entries.push(ClipboardCell {
                    inherited: value.is_some() && !self.cells.contains_key(&name),
                    value,
                    formula,
                    format: self.local_format(col, row),
                });
            }
        }
        Ok(ClipboardBlock {
            origin: block,
            entries,
            text,
        })
    }

    pub fn paste_block(
        mut self,
        col: usize,
        row: usize,
        clipboard: &ClipboardBlock,
        mode: PasteMode,
        cancel: &AtomicBool,
    ) -> Result<Self, String> {
        clipboard.origin.validate()?;
        if !(1..=MAX_COLUMNS).contains(&col)
            || !(1..=MAX_ROWS).contains(&row)
            || col + clipboard.columns() - 1 > MAX_COLUMNS
            || row + clipboard.rows() - 1 > MAX_ROWS
            || clipboard.entries.len() != clipboard.rows() * clipboard.columns()
        {
            return Err("Copied block does not fit at the selected cell".into());
        }
        let delta = [
            col as isize - clipboard.origin.left as isize,
            row as isize - clipboard.origin.top as isize,
        ];
        for (i, entry) in clipboard.entries.iter().enumerate() {
            if cancel.load(std::sync::atomic::Ordering::Acquire) {
                return Err("Spreadsheet paste interrupted".into());
            }
            let dest_col = col + i % clipboard.columns();
            let dest_row = row + i / clipboard.columns();
            let name = cell_name(dest_col, dest_row);
            if mode != PasteMode::Format {
                match mode
                    .takes_formulas()
                    .then_some(entry.formula.as_deref())
                    .flatten()
                {
                    Some(formula) => {
                        self.set(&name, &format!("={}", shift_references(formula, delta)?))?
                    }
                    None => {
                        let value = if mode.takes_formulas() && entry.inherited {
                            None
                        } else {
                            entry.value.as_ref()
                        };
                        let value = value
                            .map(ppl_vm::session::decode_value)
                            .transpose()?
                            .unwrap_or(PortableValue::Real(Real::ZERO));
                        self.set_typed(&name, &value)?;
                    }
                }
            }
            if mode.takes_format() {
                let current = self.local_format(dest_col, dest_row);
                if current != entry.format {
                    self.apply_format(
                        format::Block::cell(dest_col, dest_row),
                        replacement_format(&current, &entry.format),
                    )?;
                }
                if mode == PasteMode::Format {
                    self.materialize(Address {
                        col: dest_col,
                        row: dest_row,
                    })
                    .map_err(|error| error.to_string())?;
                }
            }
        }
        self.validate()?;
        Ok(self)
    }
}

fn replacement_format(
    current: &format::CellFormat,
    incoming: &format::CellFormat,
) -> format::CellFormat {
    let mut style = incoming.clone();
    macro_rules! reset {
        ($($field:ident = $auto:expr),* $(,)?) => {
            $(if style.$field.is_none() && current.$field.is_some() {
                style.$field = Some($auto);
            })*
        };
    }
    reset!(
        number = -1,
        digits = -1,
        font = 0,
        color = format::AUTO_COLOR,
        fill = format::AUTO_COLOR,
        horizontal = 3,
        vertical = 3,
        quotes = 0,
        textbook = 0,
    );
    style
}

pub(super) fn shift_references(source: &str, delta: [isize; 2]) -> Result<String, String> {
    if let Some(body) = super::worksheet_cas::body(source) {
        shift_reference_tokens(&body, delta).map(|body| super::worksheet_cas::wrap(&body))
    } else {
        shift_reference_tokens(source, delta)
    }
}

fn shift_reference_tokens(source: &str, [dx, dy]: [isize; 2]) -> Result<String, String> {
    let mut result = String::with_capacity(source.len());
    let mut pos = 0;
    while pos < source.len() {
        let start = pos;
        let ch = source[pos..].chars().next().unwrap();
        if ch == '"' {
            pos = quoted_end(source, pos)?;
        } else if ch == '#' {
            pos += 1;
            while pos < source.len() && source.as_bytes()[pos].is_ascii_alphanumeric() {
                pos += 1;
            }
        } else if ch.is_ascii_digit()
            || (ch == '.' && source[pos + 1..].starts_with(|c: char| c.is_ascii_digit()))
        {
            pos = number_end(source, pos);
        } else if ch.is_alphabetic() || matches!(ch, '_' | '$') {
            pos = identifier_end(source, pos);
            let token = &source[start..pos];
            if !source[..start].trim_end().ends_with('.')
                && !source[pos..].trim_start().starts_with('.')
                && let Some(address) = Address::parse(token)
            {
                let absolute_col = token.starts_with('$');
                let absolute_row = token[usize::from(absolute_col)..].contains('$');
                // The original clips relative references at the worksheet edge.
                let col = (address.col as isize + if absolute_col { 0 } else { dx })
                    .clamp(1, MAX_COLUMNS as isize) as usize;
                let row = (address.row as isize + if absolute_row { 0 } else { dy })
                    .clamp(1, MAX_ROWS as isize) as usize;
                let shifted = cell_name(col, row);
                let split = shifted.bytes().position(|ch| ch.is_ascii_digit()).unwrap();
                if absolute_col {
                    result.push('$');
                }
                result.push_str(&shifted[..split]);
                if absolute_row {
                    result.push('$');
                }
                result.push_str(&shifted[split..]);
                continue;
            }
        } else {
            pos += ch.len_utf8();
        }
        result.push_str(&source[start..pos]);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formula_paste_shifts_only_relative_worksheet_references() {
        assert_eq!(
            shift_references(
                r#"SUM(A1:B3)+$C1+D$2+$E$4+1E2+#A1:64h+Geometry.F1+"A1""#,
                [3, 2]
            )
            .unwrap(),
            r#"SUM(D3:E5)+$C3+G$2+$E$4+1E2+#A1:64h+Geometry.F1+"A1""#
        );
        assert_eq!(shift_references("A1", [-2, 1]).unwrap(), "A2");
        assert_eq!(shift_references("B1", [0, -2]).unwrap(), "B1");
    }

    #[test]
    fn mixed_block_uses_values_or_translated_formulas_and_separate_format() {
        let mut sheet = Spreadsheet::default();
        sheet.set("A1", "10").unwrap();
        sheet.set("B1", "\"alpha\"").unwrap();
        sheet.set("A2", "=A1+1").unwrap();
        sheet.set("A3", "\"\"").unwrap();
        sheet.set("B3", "20").unwrap();
        sheet.set("D4", "101").unwrap();
        sheet.set("E4", "102").unwrap();
        sheet.set("D5", "103").unwrap();
        sheet.set("E5", "104").unwrap();
        for (col, row, font) in [(1, 1, 14), (4, 4, 16), (5, 4, 16)] {
            sheet
                .apply_format(
                    format::Block::cell(col, row),
                    format::CellFormat {
                        font: Some(font),
                        ..Default::default()
                    },
                )
                .unwrap();
        }
        let snapshot = ppl_vm::SessionContext::default().snapshot();
        let cancel = Arc::new(AtomicBool::new(false));
        let block = sheet
            .copy_block(
                format::Block {
                    left: 1,
                    right: 2,
                    top: 1,
                    bottom: 3,
                },
                &snapshot,
                cancel.clone(),
                None,
            )
            .unwrap();
        assert_eq!(block.text, "10\talpha\n11\t\n\t20");
        assert!(
            sheet
                .clone()
                .paste_block(MAX_COLUMNS, 4, &block, PasteMode::Values, &cancel)
                .is_err()
        );
        for (mode, formula, format, writes) in [
            (PasteMode::Values, false, false, true),
            (PasteMode::Formulas, true, false, true),
            (PasteMode::Format, false, true, false),
            (PasteMode::ValuesAndFormat, false, true, true),
            (PasteMode::FormulasAndFormat, true, true, true),
        ] {
            let mut pasted = sheet
                .clone()
                .paste_block(4, 4, &block, mode, &cancel)
                .unwrap();
            assert_eq!(
                pasted.local_format(4, 4).font,
                Some(if format { 14 } else { 16 })
            );
            assert_eq!(
                pasted.local_format(5, 4).font,
                Some(if format { 0 } else { 16 })
            );
            let address = |sheet: &Spreadsheet, name: &str| {
                sheet
                    .evaluate_in(name, &snapshot, cancel.clone())
                    .unwrap()
                    .portable_value()
                    .unwrap()
            };
            assert_eq!(
                address(&pasted, "E5").to_value().as_real(),
                Some(Real::from_i64(if writes { 0 } else { 104 }))
            );
            if writes {
                assert_eq!(
                    address(&pasted, "E4")
                        .to_value()
                        .expect_str("text")
                        .unwrap()
                        .as_ref(),
                    "alpha"
                );
                assert_eq!(
                    address(&pasted, "D6")
                        .to_value()
                        .expect_str("blank text")
                        .unwrap()
                        .as_ref(),
                    ""
                );
            }
            pasted.set("D4", "50").unwrap();
            assert_eq!(
                address(&pasted, "D5").to_value().as_real(),
                Some(Real::from_i64(if formula {
                    51
                } else if writes {
                    11
                } else {
                    103
                }))
            );
        }
    }
}
#[test]
fn collection_paste_validates_its_shape_before_writing_cells() {
    let mut sheet = Spreadsheet::default();
    let edge = cell_name(MAX_COLUMNS, MAX_ROWS);
    sheet.set(&edge, "=42").unwrap();
    let snapshot = ppl_vm::SessionContext::default().snapshot();
    let cancel = Arc::new(AtomicBool::new(false));
    assert!(
        sheet
            .clone()
            .paste_collection(
                [MAX_COLUMNS, MAX_ROWS],
                "[[1,2]]",
                &snapshot,
                cancel.clone(),
                None,
            )
            .is_err()
    );
    assert!(
        sheet
            .clone()
            .paste_collection([1, 1], "[[1,2],[3]]", &snapshot, cancel, None,)
            .is_err()
    );
    assert_eq!(sheet.cells.get(&edge).map(String::as_str), Some("=42"));
    assert!(Spreadsheet::is_clipboard_collection("[[1,2],[3,4]]"));
    assert!(Spreadsheet::is_clipboard_collection("{1,\"text\"}"));
    assert!(!Spreadsheet::is_clipboard_collection("=[[1,2],[3,4]]"));
    assert!(!Spreadsheet::is_clipboard_collection("1\t2\n3\t4"));
}
