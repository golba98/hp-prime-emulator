//! Spreadsheet REGRS uses the Statistics 2Var fitter. The worksheet numbers
//! trigonometric and polynomial models differently from the statistics app.
use super::*;
use crate::ide::{
    native_app_models::Statistics2,
    native_regression::{FitKind, FitOptions, PairedSummary, Regression},
};

struct Series {
    fit: Regression,
    stats: PairedSummary,
}

impl Series {
    fn field(&self, mode: usize, token: &str) -> Result<(&str, f64), String> {
        let c = &self.fit.coefficients;
        let value = match token {
            "sl" if mode <= 6 => ("Slope", Some(c[1])),
            "int" if mode <= 6 => ("Intercept", Some(c[0])),
            "cor" if mode <= 6 => ("Corr", self.stats.correlation),
            "sCov" if mode <= 6 => ("SampCov", self.stats.sample_covariance),
            "pCov" if mode <= 6 => ("PopCov", Some(self.stats.population_covariance)),
            "cd" if mode <= 6 => ("CoefDet", self.stats.correlation.map(|r| r * r)),
            "cd" if mode >= 8 => {
                let total = self.stats.y.population_variance * self.stats.count as f64;
                (
                    "CoefDet",
                    (total > 0.).then(|| 1. - self.fit.residual_sum_squares / total),
                )
            }
            "L" if mode == 7 => ("L", Some(c[0])),
            "a" if mode >= 7 => ("a", Some(c[usize::from(mode == 7)])),
            "b" if mode >= 7 => ("b", Some(c[1 + usize::from(mode == 7)])),
            "c" if mode >= 8 => ("c", Some(c[2])),
            "d" if mode == 8 || mode >= 10 => ("d", Some(c[3])),
            "e" if mode == 11 => ("e", Some(c[4])),
            _ => return Err(format!("Invalid REGRS field {token} for model {mode}")),
        };
        Ok((
            value.0,
            value
                .1
                .ok_or_else(|| format!("Undefined REGRS field {token}"))?,
        ))
    }
}

pub(super) fn table(
    labels: &[String],
    rows: &[Vec<Option<Value>>],
    mode: usize,
    config: Option<&str>,
    settings: &Settings,
    cancel: &AtomicBool,
) -> Result<Vec<Vec<Value>>, String> {
    let kind = match mode {
        1..=7 => FitKind::ALL[mode - 1],
        8 => FitKind::Trigonometric,
        9 => FitKind::Quadratic,
        10 => FitKind::Cubic,
        11 => FitKind::Quartic,
        _ => return Err("Invalid REGRS model".into()),
    };
    if labels.is_empty() || labels.len() % 2 != 0 {
        return Err("REGRS needs pairs of input columns".into());
    }
    let mut series = Vec::new();
    for c in (0..labels.len()).step_by(2) {
        let mut pairs = Vec::new();
        for row in rows {
            if let (Some(x), Some(y)) = (&row[c], &row[c + 1]) {
                let number = |v: &Value| {
                    v.expect_real("REGRS")
                        .map(|n| n.to_f64())
                        .map_err(|e| e.to_string())
                };
                pairs.push([number(x)?, number(y)?]);
            }
        }
        let mut data = Statistics2 {
            pairs,
            fit_kind: kind,
        };
        let fit = data.fit_with_options(&FitOptions {
            angle: settings.angle,
            cancel: Some(cancel),
            ..Default::default()
        })?;
        for pair in &mut data.pairs {
            *pair = kind.transformed_pair(pair[0], pair[1])?;
        }
        series.push(Series {
            fit,
            stats: data.summary_with_frequencies(&[])?,
        });
    }
    let default = match mode {
        1..=6 => "H h sl int cor cd sCov pCov",
        7 => "H h L a b",
        8 => "H h a b c d",
        9 => "H h a b c cd",
        10 => "H h a b c d cd",
        _ => "H h a b c d e cd",
    };
    let spec: Vec<_> = config
        .filter(|s| !s.is_empty())
        .unwrap_or(default)
        .split_whitespace()
        .collect();
    let headers = spec.contains(&"h");
    let mut table = Vec::new();
    if spec.contains(&"H") {
        let mut row = Vec::new();
        if headers {
            row.push(Value::str("REGRS"));
        }
        row.extend(
            labels
                .chunks_exact(2)
                .map(|cols| Value::str(cols.join(":"))),
        );
        table.push(row);
    }
    for token in spec {
        if matches!(token, "H" | "h") {
            continue;
        }
        let mut row = Vec::new();
        for series in &series {
            let (name, value) = series.field(mode, token)?;
            if row.is_empty() && headers {
                row.push(Value::str(name));
            }
            row.push(Value::Real(
                Real::from_f64(value).map_err(|e| e.to_string())?,
            ));
        }
        table.push(row);
    }
    if table.is_empty() {
        return Err("REGRS needs at least one output field".into());
    }
    Ok(table)
}
