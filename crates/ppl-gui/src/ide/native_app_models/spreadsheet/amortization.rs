//! AMORT places the schedule in its first argument's range and returns a
//! title at the cell containing the formula.
use super::{reports::Output, *};
use crate::ide::{native_app_models::Finance, native_model_plots::amortization_schedule};

impl Evaluator<'_> {
    pub(super) fn amortization_report(
        &mut self,
        body: &str,
        address: Address,
    ) -> Result<Output, String> {
        let args = formula_arguments(body)?;
        let (destination, inputs) = args
            .split_first()
            .ok_or("AMORT needs a destination range")?;
        let range = self
            .sheet
            .names
            .get(*destination)
            .map(|&b| self.named_range(b))
            .or_else(|| Range::parse(destination))
            .ok_or("Invalid AMORT destination")?;
        let origin = Address {
            col: range.left,
            row: range.top,
        };
        let mut values = Vec::new();
        let mut columns = None;
        for (i, input) in inputs.iter().enumerate() {
            let v = self.expression(input, address, 1)?;
            if v.is_string() && i + 1 == inputs.len() {
                columns = Some(
                    v.expect_str("AMORT configuration")
                        .map_err(|e| e.to_string())?
                        .to_string(),
                );
            } else {
                values.push(
                    v.expect_real("AMORT parameter")
                        .map_err(|e| e.to_string())?
                        .to_f64(),
                );
            }
        }
        let [periods, rate, pv, payment, options @ ..] = values.as_slice() else {
            return Err("AMORT needs a payment count, rate, present value and payment".into());
        };
        let group = options.first().copied().unwrap_or(12.);
        let payments = group;
        // In original virtual G1 controls the third optional frequency, when supplied,
        // overrides the second. Payments per year also sets the schedule group size.
        let compounds = options
            .get(2)
            .or_else(|| options.get(1))
            .copied()
            .unwrap_or(payments);
        let begin = options.get(3).copied().unwrap_or(0.);
        let digits = options.get(4).copied();
        if options.len() > 5
            || !(1. ..=10_000.).contains(&group)
            || group.fract() != 0.
            || !matches!(begin, 0. | 1.)
            || digits.is_some_and(|n| !(0. ..=12.).contains(&n) || n.fract() != 0.)
        {
            return Err("Invalid AMORT options".into());
        }
        let model = Finance {
            periods: *periods,
            annual_rate: *rate,
            present_value: *pv,
            payment: *payment,
            future_value: 0.,
            payments_per_year: payments,
            compounds_per_year: compounds,
            begin: begin == 1.,
        };
        let schedule = amortization_schedule(
            &model,
            group as usize,
            digits.map(|n| n as u8),
            &self.cancel,
        )?;
        if columns.as_deref() == Some("") && options.len() < 5 {
            return Err("Invalid AMORT frequency or option".into());
        }
        let spec = columns
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("HhSEBPI");
        let column_headers = spec.starts_with('H');
        let fields_source = spec.strip_prefix('H').unwrap_or(spec);
        // The G1 header parser reads compact field codes literally. In data
        // rows the same fields can be separated by spaces.
        if column_headers
            && fields_source
                .chars()
                .any(|c| !matches!(c, 'h' | 'S' | 'E' | 'B' | 'P' | 'I'))
        {
            return Err("Invalid AMORT column header".into());
        }
        let fields: Vec<_> = fields_source
            .chars()
            .filter_map(|c| match c {
                'h' => Some("Period"),
                'S' => Some("Start"),
                'E' => Some("End"),
                'B' => Some("Balance"),
                'P' => Some("Principal"),
                'I' => Some("Interest"),
                c if c.is_whitespace() || c == 'H' => None,
                _ => Some(""),
            })
            .collect();
        if fields.is_empty() {
            return Ok(Output {
                origin,
                anchor: Value::zero(),
                rows: vec![vec![Value::zero()]],
            });
        }
        let mut rows = Vec::new();
        if column_headers {
            let mut header: Vec<_> = fields
                .iter()
                .map(|&name| {
                    Value::str(if name == "Period" {
                        "Amortization"
                    } else {
                        name
                    })
                })
                .collect();
            if header.is_empty() {
                header.push(Value::zero());
            }
            rows.push(header);
        }
        for entry in schedule.groups {
            let mut row = Vec::new();
            for &field in &fields {
                if field == "Period" {
                    row.push(Value::str("Period"));
                    continue;
                }
                let n = match field {
                    "Start" => entry.first_payment as f64,
                    "End" => entry.last_payment as f64,
                    "Balance" => entry.balance,
                    "Principal" => -entry.principal,
                    "Interest" => -entry.interest,
                    _ => 0.,
                };
                row.push(Value::Real(Real::from_f64(n).map_err(|e| e.to_string())?));
            }
            if row.is_empty() {
                row.push(Value::zero());
            }
            rows.push(row);
        }
        Ok(Output {
            origin,
            anchor: rows[0][0].clone(),
            rows,
        })
    }
}
