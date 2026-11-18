use super::{
    Address, Book, Evaluator, MAX_FORMAT_RULES, Spreadsheet, cell_name,
    clipboard::shift_references,
    format::{Block, FormatRule},
};
use ppl_math::{FormatOptions, HpInt, Real};
use ppl_vm::{SessionSnapshot, Settings, Value, builtins::strings::default_string_units};
use std::{
    cell::OnceCell,
    cmp::Ordering,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering as AtomicOrdering},
    },
};

struct SortKey {
    value: Value,
    text: OnceCell<Vec<u16>>,
    elements: OnceCell<Vec<Self>>,
}

impl SortKey {
    fn new(value: Value) -> Self {
        Self {
            value,
            text: OnceCell::new(),
            elements: OnceCell::new(),
        }
    }

    fn units(&self, settings: &Settings) -> &[u16] {
        self.text.get_or_init(|| {
            if let Some(text) = self.value.string_units() {
                return text.into_owned();
            }
            match self.value {
                Value::Real(number) => number
                    .format(FormatOptions {
                        exp_char: 'ᴇ',
                        ..settings.format()
                    })
                    .replace('.', &settings.digit_grouping.decimal().to_string())
                    .replace('-', "−")
                    .encode_utf16()
                    .collect(),
                _ => default_string_units(&self.value, settings),
            }
        })
    }

    fn elements(&self, values: &[Value]) -> &[Self] {
        self.elements
            .get_or_init(|| values.iter().cloned().map(Self::new).collect())
    }

    fn compare(&self, other: &Self, settings: &Settings) -> Ordering {
        match (&self.value, &other.value) {
            (Value::Real(left), Value::Real(right)) => left.cmp_value(*right),
            (Value::Int(left), Value::Int(right)) => word_number(*left).cmp(&word_number(*right)),
            (Value::Int(left), Value::Real(right)) => word_real_order(*left, *right),
            (Value::Real(left), Value::Int(right)) => word_real_order(*right, *left).reverse(),
            (Value::List(left), Value::List(right)) => self
                .elements(left)
                .iter()
                .zip(other.elements(right))
                .map(|(a, b)| a.compare(b, settings))
                .find(|order| order.is_ne())
                .unwrap_or_else(|| left.len().cmp(&right.len())),
            (left, right) if left.is_string() || right.is_string() => {
                self.units(settings).cmp(other.units(settings))
            }
            _ => Ordering::Equal,
        }
    }
}

fn word_number(word: HpInt) -> i128 {
    let value = word.numeric_value();
    let mut magnitude = value.abs();
    let mut scale = 1;
    // Sort rounds wide words to 14 significant digits, rounding at every
    // discarded decimal place. Words in the cells retain their original bits.
    while magnitude >= 100_000_000_000_000 {
        magnitude = (magnitude + 5) / 10;
        scale *= 10;
    }
    magnitude * scale * value.signum()
}

fn word_real_order(word: HpInt, real: Real) -> Ordering {
    let integer = real.trunc();
    match integer.to_i128() {
        Some(right) => word_number(word)
            .cmp(&right)
            .then_with(|| integer.cmp_value(real)),
        None if real.is_negative() => Ordering::Greater,
        None => Ordering::Less,
    }
}

fn order_keys(
    keys: &mut [(usize, SortKey)],
    descending: bool,
    settings: &Settings,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let compare = |left: &SortKey, right: &SortKey| {
        let order = left.compare(right, settings);
        if descending { order.reverse() } else { order }
    };
    let first = &keys[0].1.value;
    let key_type = std::mem::discriminant(first);
    if !matches!(first, Value::List(_))
        && keys
            .iter()
            .all(|(_, key)| std::mem::discriminant(&key.value) == key_type)
    {
        keys.sort_by(|(_, left), (_, right)| compare(left, right));
        return Ok(());
    }

    // Numeric order, formatted text order and ties between unorderable keys
    // can disagree, including inside lists. Keep the original adjacent sort.
    for index in 1..keys.len() {
        if cancel.load(AtomicOrdering::Acquire) {
            return Err("Spreadsheet sort interrupted".into());
        }
        let mut start = index;
        while start > 0 && compare(&keys[index].1, &keys[start - 1].1).is_lt() {
            start -= 1;
        }
        keys[start..=index].rotate_right(1);
    }
    Ok(())
}

impl Spreadsheet {
    pub fn sort_rows(
        mut self,
        block: Block,
        key_column: usize,
        descending: bool,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
        geometry: Option<Arc<Book>>,
    ) -> Result<Self, String> {
        block.validate()?;
        if !(block.left..=block.right).contains(&key_column) {
            return Err("Choose a sort column within the selected cells".into());
        }
        if block.top == block.bottom {
            return Ok(self);
        }

        let mut keys = {
            let mut evaluator = Evaluator::new(&self, snapshot, cancel.clone(), geometry)?;
            (block.top..=block.bottom)
                .map(|row| {
                    evaluator.check()?;
                    let value = evaluator.cell(&cell_name(key_column, row))?;
                    if matches!(value, Value::Symbolic(_) | Value::Grob(_)) {
                        return Err(
                            "Choose a sort column without symbolic or graphics values".into()
                        );
                    }
                    Ok((row, SortKey::new(value)))
                })
                .collect::<Result<Vec<_>, String>>()?
        };
        order_keys(&mut keys, descending, &snapshot.settings, &cancel)?;

        let source_rows: Vec<_> = keys.into_iter().map(|(row, _)| row).collect();
        if source_rows.iter().copied().eq(block.top..=block.bottom) {
            return Ok(self);
        }
        let mut destination_rows = vec![0; source_rows.len()];
        for (index, &row) in source_rows.iter().enumerate() {
            destination_rows[row - block.top] = block.top + index;
        }
        let moved = |name: &str| {
            let address = Address::parse(name)?;
            if !block.contains(address.col, address.row) {
                return None;
            }
            let row = destination_rows[address.row - block.top];
            (row != address.row).then(|| {
                (
                    cell_name(address.col, row),
                    row as isize - address.row as isize,
                )
            })
        };
        for (name, mut source) in std::mem::take(&mut self.cells) {
            let name = if let Some((destination, delta)) = moved(&name) {
                if !self.entered_values.contains_key(&name)
                    && let Some(formula) = source.trim().strip_prefix('=')
                {
                    source = format!("={}", shift_references(formula, [0, delta])?);
                }
                destination
            } else {
                name
            };
            self.cells.insert(name, source);
        }
        self.entered_values = std::mem::take(&mut self.entered_values)
            .into_iter()
            .map(|(name, value)| {
                (
                    moved(&name).map_or(name, |(destination, _)| destination),
                    value,
                )
            })
            .collect();
        self.metadata_cells = std::mem::take(&mut self.metadata_cells)
            .into_iter()
            .map(|name| moved(&name).map_or(name, |(destination, _)| destination))
            .collect();
        for named in self.names.values_mut() {
            if named.left == named.right
                && named.top == named.bottom
                && block.contains(named.left, named.top)
            {
                let row = destination_rows[named.top - block.top];
                named.top = row;
                named.bottom = row;
            }
        }
        for rule in std::mem::take(&mut self.formats) {
            if cancel.load(AtomicOrdering::Acquire) {
                return Err("Spreadsheet sort interrupted".into());
            }
            move_format(rule, block, &source_rows, &mut self.formats);
            if self.formats.len() > MAX_FORMAT_RULES {
                return Err("Spreadsheet format limit exceeded".into());
            }
        }
        self.validate()?;
        Ok(self)
    }
}

fn move_format(rule: FormatRule, block: Block, source_rows: &[usize], out: &mut Vec<FormatRule>) {
    let formatted = rule.block;
    let left = formatted.left.max(block.left);
    let right = formatted.right.min(block.right);
    let top = formatted.top.max(block.top);
    let bottom = formatted.bottom.min(block.bottom);
    if formatted.scope() != 3
        || left > right
        || top > bottom
        || source_rows.iter().enumerate().all(|(i, row)| {
            (top..=bottom).contains(row) == (top..=bottom).contains(&(block.top + i))
        })
    {
        out.push(rule);
        return;
    }

    let mut add = |left, right, top, bottom| {
        if left <= right && top <= bottom {
            out.push(FormatRule {
                block: Block {
                    left,
                    right,
                    top,
                    bottom,
                },
                style: rule.style.clone(),
            });
        }
    };
    add(formatted.left, formatted.right, formatted.top, top - 1);
    add(
        formatted.left,
        formatted.right,
        bottom + 1,
        formatted.bottom,
    );
    add(formatted.left, left - 1, top, bottom);
    add(right + 1, formatted.right, top, bottom);

    let mut run_start = None;
    // The final false entry closes a run that reaches the selection's last row.
    for (index, contains) in source_rows
        .iter()
        .map(|row| (top..=bottom).contains(row))
        .chain(std::iter::once(false))
        .enumerate()
    {
        let row = block.top + index;
        if contains {
            run_start.get_or_insert(row);
        } else if let Some(start) = run_start.take() {
            add(left, right, start, row - 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ide::native_app_models::spreadsheet::format::CellFormat;
    use ppl_math::IntBase;

    #[test]
    fn mixed_keys_follow_the_original_adjacent_comparison_order() {
        let settings = Settings::default();
        let values = [
            Value::from_i64(2),
            Value::str("b"),
            Value::from_i64(-3),
            Value::str("A"),
            Value::from_i64(0),
            Value::from_i64(5),
            Value::str("9"),
            Value::from_i64(-1),
            Value::str(""),
            Value::from_i64(0),
        ];
        let mut keys: Vec<_> = values
            .into_iter()
            .enumerate()
            .map(|(i, value)| (i + 1, SortKey::new(value)))
            .collect();
        let cancel = AtomicBool::new(false);
        for (descending, expected) in [
            (true, [2, 1, 3, 4, 7, 6, 5, 10, 8, 9]),
            (false, [9, 1, 7, 4, 2, 3, 8, 5, 10, 6]),
            (true, [2, 4, 7, 6, 1, 5, 10, 8, 3, 9]),
        ] {
            order_keys(&mut keys, descending, &settings, &cancel).unwrap();
            assert_eq!(
                keys.iter().map(|&(row, _)| row).collect::<Vec<_>>(),
                expected
            );
        }
    }

    #[test]
    fn word_keys_round_each_dropped_digit_and_compare_across_the_real_exponent_range() {
        let word = |n| HpInt::new_with_width(n, 64, IntBase::Hex).unwrap();
        let base = 20_000_000_000_000_000;
        assert_eq!(word_number(word(base + 444)), base);
        assert_eq!(word_number(word(base + 445)), base + 1_000);
        assert_eq!(
            word_real_order(word(base - 501), Real::parse("2e16").unwrap()),
            Ordering::Equal
        );
        for (real, expected) in [
            ("1e-400", Ordering::Less),
            ("-1e-400", Ordering::Greater),
            ("1e400", Ordering::Less),
            ("-1e400", Ordering::Greater),
        ] {
            assert_eq!(
                word_real_order(word(0), Real::parse(real).unwrap()),
                expected
            );
        }
    }

    #[test]
    fn sorting_splits_bounded_formats_at_the_selection_edges() {
        let mut sheet = Spreadsheet::default();
        for (name, value) in [("A1", "3"), ("A2", "1"), ("A3", "2"), ("B2", "=D2+1")] {
            sheet.set(name, value).unwrap();
        }
        sheet.set_name("Reloc", Block::cell(2, 2)).unwrap();
        for (block, font) in [
            (
                Block {
                    left: 1,
                    right: 3,
                    top: 1,
                    bottom: 2,
                },
                14,
            ),
            (Block::cell(2, 1), 16),
        ] {
            sheet
                .apply_format(
                    block,
                    CellFormat {
                        font: Some(font),
                        ..Default::default()
                    },
                )
                .unwrap();
        }
        let sorted = sheet
            .sort_rows(
                Block {
                    left: 1,
                    right: 2,
                    top: 1,
                    bottom: 3,
                },
                1,
                false,
                &ppl_vm::SessionContext::default().snapshot(),
                Arc::new(AtomicBool::new(false)),
                None,
            )
            .unwrap();
        assert_eq!(sorted.cells.get("B1").map(String::as_str), Some("=D1+1"));
        assert!(!sorted.cells.contains_key("B2"));
        assert_eq!(sorted.names["Reloc"], Block::cell(2, 1));
        for (col, expected) in [
            (1, [Some(14), None, Some(14)]),
            (2, [Some(14), None, Some(16)]),
            (3, [Some(14), Some(14), None]),
        ] {
            assert_eq!(
                std::array::from_fn(|i| sorted.local_format(col, i + 1).font),
                expected
            );
        }
    }
}
