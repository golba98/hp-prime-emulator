//! A worksheet statistics report expands into ordinary adjacent cells. The anchor
//! retains its executable formula and each report preserves destination formats.
use super::*;
use crate::ide::native_statistics::one_data::Data;

const DEFAULT_FIELDS: &str = "H h \u{e000} Σ Σ² s s² σ σ² serr ss n min q1 med q3 max";
const PAIRED_FIELDS: &str =
    "H h \u{e000} ΣX ΣX² sX sX² σX σX² serrX ssX n ȳ ΣY ΣY² sY sY² σY σY² serrY ssY ΣXY";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Report {
    One,
    Two,
}

pub(super) fn body(source: &str) -> Option<(Report, &str)> {
    let source = source.trim().strip_prefix('=')?.trim_start();
    let (name, rest) = source.split_at_checked(5)?;
    let call = rest.trim_start();
    let kind = if name.eq_ignore_ascii_case("STAT1") {
        Report::One
    } else if name.eq_ignore_ascii_case("STAT2") {
        Report::Two
    } else {
        return None;
    };
    (call.starts_with('(') && call_end(call, 0).ok()? == call.len())
        .then(|| (kind, &call[1..call.len() - 1]))
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

// Configuration tokens and the labels displayed in the grid use different case.
fn paired_field(token: &str) -> Option<&'static str> {
    Some(match token {
        "n" => "n",
        "Σx" => "ΣX",
        "Σx²" => "ΣX²",
        "Σy" => "ΣY",
        "Σy²" => "ΣY²",
        "σx" => "σX",
        "σy" => "σY",
        "σx²" => "σX²",
        "σy²" => "σY²",
        "sx" => "sX",
        "sy" => "sY",
        "sx²" => "sX²",
        "sy²" => "sY²",
        "serrx" => "serrX",
        "serry" => "serrY",
        "ssx" => "ssX",
        "ssy" => "ssY",
        "ȳ" => "ȳ",
        "\u{e000}" => "\u{e000}",
        _ => return None,
    })
}

impl Evaluator<'_> {
    pub(super) fn report_table(
        &mut self,
        (kind, body): (Report, &str),
        address: Address,
    ) -> Result<Vec<Vec<Value>>, String> {
        let args = formula_arguments(body)?;
        let (input, options) = args
            .split_first()
            .ok_or("Worksheet statistics need an input range or list")?;
        let mut mode = None;
        let mut config = None;
        for &option in options {
            let v = self.expression(option, address, 1)?;
            if v.is_string() && config.is_none() {
                config = Some(
                    v.expect_str("Worksheet statistics configuration")
                        .map_err(|e| e.to_string())?
                        .to_string(),
                );
            } else if mode.is_none() && config.is_none() {
                let n = v
                    .expect_real("Worksheet statistics mode")
                    .map_err(|e| e.to_string())?;
                mode = Some(if n == Real::ONE {
                    1
                } else if n == Real::from_i64(2) {
                    2
                } else {
                    return Err("Worksheet statistics mode must be 1 or 2".into());
                });
            } else {
                return Err("Invalid worksheet statistics options".into());
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
                return Err("Input range exceeds worksheet capacity".into());
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
                _ => return Err("Worksheet statistics need an input range or list".into()),
            }
        };
        let (labels, rows) = if kind == Report::One && rows.len() == 1 && labels.len() > 1 {
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
        let paired = kind == Report::Two;
        if paired && rows.len() < 2 {
            return Err("Paired statistics need two rows".into());
        }
        let dimensions = if paired { 2 } else { 1 };
        let weighted = mode.map_or(labels.len() == dimensions + 1, |mode| mode == 2);
        let stride = dimensions + usize::from(weighted);
        if labels.is_empty() || labels.len() % stride != 0 {
            return Err("Worksheet statistics need complete data columns".into());
        }
        let mut summaries = Vec::new();
        for c in (0..labels.len()).step_by(stride) {
            let mut xs = Vec::new();
            let mut ys = Vec::new();
            let mut weights = Vec::new();
            let mut products = Real::ZERO;
            for row in &rows {
                let Some(x) = &row[c] else { continue };
                let x = x
                    .expect_real("Worksheet observation")
                    .map_err(|e| e.to_string())?;
                let weight = if weighted {
                    row[c + dimensions]
                        .clone()
                        .ok_or("Missing worksheet frequency")?
                } else {
                    Value::from_i64(1)
                };
                if paired {
                    let Some(y) = &row[c + 1] else { continue };
                    let y = y
                        .expect_real("Worksheet observation")
                        .map_err(|e| e.to_string())?;
                    let w = weight
                        .expect_real("Worksheet frequency")
                        .map_err(|e| e.to_string())?;
                    products = x
                        .mul(y)
                        .and_then(|p| p.mul(w))
                        .and_then(|p| products.add(p))
                        .map_err(|e| e.to_string())?;
                    ys.push(y);
                }
                xs.push(x);
                weights.push(weight);
            }
            let summarize = |values| {
                Data::worksheet_summary(
                    values,
                    &weights,
                    self.context.settings.numeric_version,
                    &self.cancel,
                )
            };
            let x = summarize(xs)?;
            let y = if paired { Some(summarize(ys)?) } else { None };
            summaries.push((x, y, products));
        }
        let custom = config.as_deref().is_some_and(|spec| !spec.is_empty());
        let spec = config
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(if paired {
                PAIRED_FIELDS
            } else {
                DEFAULT_FIELDS
            });
        let tokens: Vec<_> = spec.split_whitespace().collect();
        let headers = tokens.contains(&"h");
        let mut table = Vec::new();
        if tokens.contains(&"H") {
            let mut row = Vec::new();
            if headers {
                row.push(Value::str(if paired { "STAT2" } else { "STAT1" }));
            }
            row.extend(labels.chunks_exact(stride).map(|cols| {
                if paired {
                    Value::str(cols.join(":"))
                } else {
                    Value::str(cols[0].as_str())
                }
            }));
            table.push(row);
        }
        for token in tokens {
            if matches!(token, "h" | "H") {
                continue;
            }
            let (label, key, square, y_axis) = if paired {
                let key = if custom {
                    paired_field(token).ok_or_else(|| format!("Unknown STAT2 field {token}"))?
                } else {
                    token
                };
                match key {
                    "ȳ" => ("ȳ", "MeanX", false, true),
                    "ΣY" => ("ΣY", "ΣX", false, true),
                    "ΣY²" => ("ΣY²", "ΣX2", false, true),
                    "sY" => ("sY", "sX", false, true),
                    "sY²" => ("sY²", "sX", true, true),
                    "σY" => ("σY", "σX", false, true),
                    "σY²" => ("σY²", "σX", true, true),
                    "serrY" => ("serrY", "serrX", false, true),
                    "ssY" => ("ssY", "ssX", false, true),
                    "ΣXY" => ("ΣXY", "ΣXY", false, false),
                    _ => {
                        let (label, key, square) =
                            field(key).ok_or_else(|| format!("Unknown STAT2 field {token}"))?;
                        (label, key, square, false)
                    }
                }
            } else {
                let (label, key, square) =
                    field(token).ok_or_else(|| format!("Unknown STAT1 field {token}"))?;
                (label, key, square, false)
            };
            let mut row = Vec::new();
            if headers {
                row.push(Value::str(label));
            }
            for (x, y, product) in &summaries {
                let v = if key == "ΣXY" {
                    Value::Real(*product)
                } else {
                    let summary = if y_axis {
                        y.as_ref().ok_or("Missing paired observations")?
                    } else {
                        x
                    };
                    summary.variables[key].to_value()
                };
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
            return Err("Worksheet statistics need at least one output field".into());
        }
        Ok(table)
    }
}

impl Spreadsheet {
    pub(crate) fn refresh_reports(
        &mut self,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
        geometry: Option<Arc<Book>>,
    ) -> Result<bool, String> {
        let mut changed = false;
        let anchors: Vec<_> = self
            .cells
            .iter()
            .filter_map(|(name, source)| {
                let (kind, args) = body(source)?;
                Some((Address::parse(name)?, kind, args.to_owned()))
            })
            .collect();
        for (address, kind, source) in anchors {
            let mut evaluator = Evaluator::new(self, snapshot, cancel.clone(), geometry.clone())?;
            evaluator.visiting.insert(address.name());
            let result = evaluator.report_table((kind, &source), address);
            evaluator.check()?;
            // A live formula reports its own errors on read. Keep the last
            // ordinary values in adjacent cells until a fresh report succeeds.
            let Ok(table) = result else {
                continue;
            };
            changed |= self.store_report(address, table, true)?;
        }
        Ok(changed)
    }
    pub(super) fn store_report(
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
            return Err("Report output extends past the worksheet".into());
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
