//! A worksheet STAT1 report expands into ordinary adjacent cells. The anchor
//! retains its executable formula and each report preserves destination formats.
use super::*;
use crate::ide::native_statistics::one_data::{Data, Summary};

const DEFAULT_FIELDS: &str = "H h \u{e000} Σ Σ² s s² σ σ² serr ss n min q1 med q3 max";
pub(super) fn body(source: &str) -> Option<&str> {
    let source = source.trim().strip_prefix('=')?.trim_start();
    let (name, rest) = source.split_at_checked(5)?;
    let call = rest.trim_start();
    (name.eq_ignore_ascii_case("STAT1")
        && call.starts_with('(')
        && call_end(call, 0).ok()? == call.len())
    .then(|| &call[1..call.len() - 1])
}

// Report labels use the original LCD glyph for the mean and ordinary Unicode
// for the other rows. Internal summary names also serve typed PPL field access.
fn field(token: &str) -> Option<(&'static str, &'static str, bool)> {
    Some(match token {
        "\u{e000}" | "x̄" => ("\u{e000}", "MeanX", false),
        "Σ" | "ΣX" => ("ΣX", "ΣX", false),
        "Σ²" | "ΣX²" => ("ΣX²", "ΣX2", false),
        "s" | "sX" => ("sX", "sX", false),
        "s²" | "sX²" => ("sX²", "sX", true),
        "σ" | "σX" => ("σX", "σX", false),
        "σ²" | "σX²" => ("σX²", "σX", true),
        "serr" | "serrX" => ("serrX", "serrX", false),
        "ss" | "ssX" => ("ssX", "ssX", false),
        "n" => ("n", "NbItem", false),
        "min" | "Min" => ("Min", "MinVal", false),
        "q1" | "Q1" => ("Q1", "Q₁", false),
        "med" | "Med" => ("Med", "MedVal", false),
        "q3" | "Q3" => ("Q3", "Q₃", false),
        "max" | "Max" => ("Max", "MaxVal", false),
        _ => return None,
    })
}

impl Evaluator<'_> {
    pub(super) fn stat1_table(
        &mut self,
        body: &str,
        address: Address,
    ) -> Result<Vec<Vec<Value>>, String> {
        let args = formula_arguments(body)?;
        let (input, options) = args
            .split_first()
            .ok_or("STAT1 needs an input range or list")?;
        let mut mode = None;
        let mut config = None;
        for &option in options {
            let v = self.expression(option, address, 1)?;
            if v.is_string() && config.is_none() {
                config = Some(
                    v.expect_str("STAT1 configuration")
                        .map_err(|e| e.to_string())?
                        .to_string(),
                );
            } else if mode.is_none() && config.is_none() {
                let n = v.expect_real("STAT1 mode").map_err(|e| e.to_string())?;
                mode = Some(if n == Real::ONE {
                    1
                } else if n == Real::from_i64(2) {
                    2
                } else {
                    return Err("STAT1 mode must be 1 or 2".into());
                });
            } else {
                return Err("Invalid STAT1 options".into());
            }
        }
        let range = self
            .sheet
            .names
            .get(*input)
            .map(|&b| self.named_range(b))
            .or_else(|| Range::parse(input));
        let (labels, rows) = if let Some(range) = range {
            if (range.right - range.left + 1) * (range.bottom - range.top + 1) > MAX_POPULATED_CELLS
            {
                return Err("STAT1 range exceeds worksheet capacity".into());
            }
            let mut rows = Vec::new();
            for r in range.top..=range.bottom {
                let mut row = Vec::new();
                for c in range.left..=range.right {
                    let v = if self.sheet.has_value(c, r) {
                        Some(self.cell(&cell_name(c, r))?)
                    } else {
                        None
                    };
                    row.push(v);
                }
                rows.push(row);
            }
            let labels = (range.left..=range.right)
                .map(|c| cell_name(c, 1).trim_end_matches('1').to_owned())
                .collect();
            (labels, rows)
        } else {
            match self.expression(input, address, 1)? {
                Value::List(values) => (
                    vec!["A".to_string()],
                    values.iter().map(|v| vec![Some(v.clone())]).collect(),
                ),
                Value::Matrix(m) => (
                    (1..=m.cols)
                        .map(|c| cell_name(c, 1).trim_end_matches('1').to_owned())
                        .collect(),
                    (0..m.rows)
                        .map(|r| m.row(r).iter().cloned().map(Some).collect())
                        .collect(),
                ),
                _ => return Err("STAT1 needs an input range or list".into()),
            }
        };
        let (labels, rows) = if rows.len() == 1 && labels.len() > 1 {
            (
                vec![labels[0].clone()],
                rows.into_iter()
                    .next()
                    .unwrap()
                    .into_iter()
                    .map(|v| vec![v])
                    .collect(),
            )
        } else {
            (labels, rows)
        };
        let stride = mode.unwrap_or(if labels.len() == 2 { 2 } else { 1 });
        if labels.len() % stride != 0 {
            return Err("Frequency data needs a column pair".into());
        }
        let mut summaries: Vec<Summary> = Vec::new();
        for c in (0..labels.len()).step_by(stride) {
            let mut data = Vec::new();
            let mut weights = Vec::new();
            for row in &rows {
                let Some(x) = &row[c] else { continue };
                data.push(
                    x.expect_real("STAT1 observation")
                        .map_err(|e| e.to_string())?,
                );
                weights.push(if stride == 1 {
                    Value::from_i64(1)
                } else {
                    row[c + 1].clone().ok_or("STAT1 frequency is missing")?
                });
            }
            summaries.push(Data::worksheet_summary(
                data,
                &weights,
                self.context.settings.numeric_version,
                &self.cancel,
            )?);
        }
        let spec = config
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(DEFAULT_FIELDS);
        let tokens: Vec<_> = spec.split_whitespace().collect();
        let headers = tokens.contains(&"h");
        let mut table = Vec::new();
        if tokens.contains(&"H") {
            let mut row = Vec::new();
            if headers {
                row.push(Value::str("STAT1"));
            }
            row.extend(
                labels
                    .iter()
                    .step_by(stride)
                    .map(|label| Value::str(label.as_str())),
            );
            table.push(row);
        }
        for token in tokens {
            if matches!(token, "h" | "H") {
                continue;
            }
            let (label, key, square) =
                field(token).ok_or_else(|| format!("Unknown STAT1 field {token}"))?;
            let mut row = Vec::new();
            if headers {
                row.push(Value::str(label));
            }
            for data in &summaries {
                let v = data.variables[key].to_value();
                row.push(if square {
                    ppl_vm::ops::binary(ppl_syntax::ast::BinOp::Mul, &v, &v, &self.context.settings)
                        .map_err(|e| e.to_string())?
                } else {
                    v
                });
            }
            table.push(row);
        }
        if table.is_empty() {
            return Err("STAT1 needs at least one output field".into());
        }
        Ok(table)
    }
}

impl Spreadsheet {
    pub(crate) fn refresh_stat1(
        &mut self,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
        geometry: Option<Arc<Book>>,
    ) -> Result<bool, String> {
        let mut changed = false;
        let anchors: Vec<_> = self
            .cells
            .iter()
            .filter_map(|(name, source)| Some((Address::parse(name)?, body(source)?.to_owned())))
            .collect();
        for (address, source) in anchors {
            let mut evaluator = Evaluator::new(self, snapshot, cancel.clone(), geometry.clone())?;
            evaluator.visiting.insert(address.name());
            let result = evaluator.stat1_table(&source, address);
            evaluator.check()?;
            // A live formula reports its own errors on read. Keep the last
            // ordinary values in adjacent cells until a fresh report succeeds.
            let Ok(table) = result else {
                continue;
            };
            changed |= self.store_stat1(address, table, true)?;
        }
        Ok(changed)
    }
    pub(super) fn store_stat1(
        &mut self,
        address: Address,
        table: Vec<Vec<Value>>,
        keep_anchor: bool,
    ) -> Result<bool, String> {
        let mut changed = false;
        if address.row + table.len() - 1 > MAX_ROWS
            || table
                .iter()
                .any(|row| address.col + row.len() - 1 > MAX_COLUMNS)
        {
            return Err("STAT1 output extends past the worksheet".into());
        }
        for (r, row) in table.into_iter().enumerate() {
            for (c, value) in row.into_iter().enumerate() {
                if !keep_anchor || r != 0 || c != 0 {
                    let name = cell_name(address.col + c, address.row + r);
                    let value = PortableValue::from(&value);
                    if self.entered_values.get(&name)
                        != Some(&ppl_vm::session::encode_value(&value))
                    {
                        self.set_typed(&name, &value)?;
                        changed = true;
                    }
                }
            }
        }
        self.validate()?;
        Ok(changed)
    }
}
