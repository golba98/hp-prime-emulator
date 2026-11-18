//! Statistics data columns and five independent analysis definitions.
//! G1 User Guide chapters11–12: D1..D9/D0 and C1..C9/C0; frequency
//! columns select integer counts, without expanding observations in memory.
use super::{
    native_app_models::{Statistics1, Statistics2, Summary1},
    native_regression::{FitKind, FitOptions, PairedSummary, Regression},
};
use ppl_vm::SessionSnapshot;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
pub mod evaluation;
pub mod one;
pub mod one_data;
pub mod one_plots;
pub mod one_scale;
mod one_trace;
pub mod plots;
mod two;
pub const COLUMN_COUNT: usize = 10;
pub const ANALYSIS_COUNT: usize = 5;
pub const ANALYSIS_COLORS: [u32; ANALYSIS_COUNT] =
    [0x0000ff, 0xff0000, 0x008000, 0xffa500, 0x800080];
pub const MAX_ROWS: usize = 1000;
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Columns(pub [Vec<f64>; COLUMN_COUNT]);
impl Columns {
    pub fn from_snapshot(
        snapshot: &SessionSnapshot,
        two: bool,
    ) -> Result<(Self, [Vec<ppl_math::Real>; COLUMN_COUNT]), String> {
        let mut columns = Columns::default();
        let mut typed = std::array::from_fn(|_| Vec::new());
        for (index, data) in columns.0.iter_mut().enumerate() {
            let name = Columns::label(index, two);
            if let Some(value) = snapshot.globals.get(&name) {
                let ppl_vm::PortableValue::List(values) = value else {
                    return Err(format!("{name} is not a statistics data list"));
                };
                typed[index] = values
                    .iter()
                    .map(|value| {
                        value
                            .to_value()
                            .expect_real(&name)
                            .map_err(|error| error.to_string())
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                // Binary geometry is a projection. Preserve calculator values
                // separately so valid exponents and exact values remain editable.
                *data = typed[index]
                    .iter()
                    .map(|number| {
                        let projected = number.to_f64();
                        if projected.is_infinite() {
                            if number.is_negative() {
                                -f64::MAX
                            } else {
                                f64::MAX
                            }
                        } else if projected == 0. && !number.is_zero() {
                            if number.is_negative() {
                                -f64::MIN_POSITIVE
                            } else {
                                f64::MIN_POSITIVE
                            }
                        } else {
                            projected
                        }
                    })
                    .collect();
            }
        }
        columns.validate()?;
        Ok((columns, typed))
    }
    pub fn label(index: usize, two: bool) -> String {
        format!("{}{}", if two { 'C' } else { 'D' }, (index + 1) % 10)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self
            .0
            .iter()
            .any(|column| column.len() > MAX_ROWS || column.iter().any(|v| !v.is_finite()))
        {
            return Err("Statistics columns need finite real values and at most1000 rows".into());
        }
        Ok(())
    }
    pub fn get(&self, index: Option<usize>) -> Result<&[f64], String> {
        self.0
            .get(index.ok_or("Choose a data column")?)
            .map(Vec::as_slice)
            .ok_or_else(|| "Invalid statistics column".into())
    }
    pub fn set(&mut self, column: usize, row: usize, value: f64) -> Result<(), String> {
        if !value.is_finite() || row >= MAX_ROWS {
            return Err("Statistics cell exceeds its numeric or row limit".into());
        }
        let data = self.0.get_mut(column).ok_or("Invalid statistics column")?;
        if row > data.len() {
            return Err("Enter the preceding rows before this statistics cell".into());
        }
        if row == data.len() {
            data.push(value);
        } else {
            data[row] = value;
        }
        Ok(())
    }
    pub fn remove(&mut self, column: usize, row: usize) -> Result<(), String> {
        let data = self.0.get_mut(column).ok_or("Invalid statistics column")?;
        if row >= data.len() {
            return Err("This statistics cell is empty".into());
        }
        data.remove(row);
        Ok(())
    }
    pub fn frequencies(&self, column: Option<usize>, length: usize) -> Result<Vec<u64>, String> {
        let Some(column) = column else {
            return Ok(Vec::new());
        };
        let data = self.get(Some(column))?;
        if data.len() != length {
            return Err("The frequency column must match the number of observations".into());
        }
        data.iter()
            .map(|&v| {
                if v.is_finite() && (0. ..=9_007_199_254_740_992.).contains(&v) && v.fract() == 0. {
                    Ok(v as u64)
                } else {
                    Err(
                        "Frequencies must be nonnegative integers within exact numerical range"
                            .into(),
                    )
                }
            })
            .collect()
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OneAnalysis {
    pub enabled: bool,
    pub data: Option<usize>,
    pub frequency: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_source: Option<String>,
    pub chart: one::Chart,
    pub option: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TwoAnalysis {
    pub enabled: bool,
    pub x: Option<usize>,
    pub y: Option<usize>,
    pub frequency: Option<usize>,
    pub fit: FitKind,
    pub user_expression: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observation_color: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fit_color: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fit_override: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition_number: Option<i64>,
}
impl Default for TwoAnalysis {
    fn default() -> Self {
        Self {
            enabled: false,
            x: None,
            y: None,
            frequency: None,
            fit: FitKind::Linear,
            user_expression: String::new(),
            x_source: None,
            y_source: None,
            frequency_source: None,
            observation_color: None,
            fit_color: None,
            fit_override: None,
            definition_number: None,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OneBook {
    pub columns: Columns,
    pub analyses: [OneAnalysis; ANALYSIS_COUNT],
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub result_values: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
    /// The first result column of the most recently successful calculation.
    /// Checkbox edits do not rewrite the retained results table.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_analysis: Option<usize>,
}
impl Default for OneBook {
    fn default() -> Self {
        let mut analyses = std::array::from_fn(|_| OneAnalysis::default());
        analyses[0] = OneAnalysis {
            enabled: true,
            data: Some(0),
            frequency: None,
            ..Default::default()
        };
        Self {
            columns: Columns::default(),
            analyses,
            result_values: BTreeMap::new(),
            result_analysis: None,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TwoBook {
    pub columns: Columns,
    pub analyses: [TwoAnalysis; ANALYSIS_COUNT],
    #[serde(skip_serializing_if = "two::empty_equations")]
    pub fit_equations: [Option<String>; ANALYSIS_COUNT],
    // The saved equation survives Off. The G1 discards its freshness on reopen.
    #[serde(skip)]
    pub fit_columns_revisions: [Option<u64>; ANALYSIS_COUNT],
}
impl Default for TwoBook {
    fn default() -> Self {
        let mut analyses = std::array::from_fn(|_| TwoAnalysis::default());
        analyses[0] = TwoAnalysis {
            enabled: true,
            x: Some(0),
            y: Some(1),
            ..Default::default()
        };
        Self {
            columns: Columns::default(),
            analyses,
            fit_equations: Default::default(),
            fit_columns_revisions: Default::default(),
        }
    }
}
fn check(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Statistics calculation interrupted".into())
    } else {
        Ok(())
    }
}
fn valid_reference(reference: Option<usize>) -> bool {
    reference.is_none_or(|index| index < COLUMN_COUNT)
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OneResult {
    #[serde(flatten)]
    pub basic: Summary1,
    pub quartile1: f64,
    pub quartile3: f64,
    pub sum_squares: f64,
    pub sample_standard_deviation: f64,
    pub population_standard_deviation: f64,
    pub standard_error: f64,
    pub sum_squared_deviations: f64,
}
impl OneResult {
    fn calculate(model: &Statistics1) -> Result<Self, String> {
        let basic = model.calculate()?;
        if basic.count < 2 {
            return Err("Insufficient statistics data".into());
        }
        let mut values = model
            .samples
            .iter()
            .enumerate()
            .filter_map(|(index, &value)| {
                let weight = model.frequencies.get(index).copied().unwrap_or(1);
                (weight > 0).then_some((value, weight))
            })
            .collect::<Vec<_>>();
        values.sort_by(|a, b| a.0.total_cmp(&b.0));
        let rank = |rank: u64| {
            let mut total = 0;
            for &(value, weight) in &values {
                total += weight;
                if rank < total {
                    return value;
                }
            }
            unreachable!()
        };
        let median = |start: u64, length: u64| {
            rank(start + (length - 1) / 2) / 2. + rank(start + length / 2) / 2.
        };
        let half = basic.count / 2;
        let quartile1 = median(0, half);
        let quartile3 = median(basic.count.div_ceil(2), half);
        let sum_squares = values.iter().map(|&(x, w)| x * x * w as f64).sum::<f64>();
        let sample_variance = basic.sample_variance.unwrap();
        let sample_standard_deviation = sample_variance.sqrt();
        let population_standard_deviation = basic.population_variance.sqrt();
        let standard_error = (sample_variance / basic.count as f64).sqrt();
        let sum_squared_deviations = basic.population_variance * basic.count as f64;
        if [
            quartile1,
            quartile3,
            sum_squares,
            sample_standard_deviation,
            population_standard_deviation,
            standard_error,
            sum_squared_deviations,
        ]
        .iter()
        .any(|v| !v.is_finite())
        {
            return Err("Statistics result exceeds numerical range".into());
        }
        Ok(Self {
            basic,
            quartile1,
            quartile3,
            sum_squares,
            sample_standard_deviation,
            population_standard_deviation,
            standard_error,
            sum_squared_deviations,
        })
    }
}
impl OneBook {
    pub fn from_legacy(model: &Statistics1) -> Result<Self, String> {
        if model.frequencies.iter().any(|&f| f > 9_007_199_254_740_992) {
            return Err("Legacy frequency exceeds exact numerical range".into());
        }
        let mut book = Self::default();
        book.columns.0[0] = model.samples.clone();
        if !model.frequencies.is_empty() {
            book.columns.0[1] = model.frequencies.iter().map(|&n| n as f64).collect();
            book.analyses[0].frequency = Some(1);
        }
        Ok(book)
    }
    pub fn validate(&self) -> Result<(), String> {
        self.columns.validate()?;
        for analysis in &self.analyses {
            analysis.validate_definition()?;
        }
        self.validate_results()?;
        if self
            .analyses
            .iter()
            .any(|a| !valid_reference(a.data) || !valid_reference(a.frequency))
        {
            return Err("Invalid one-variable analysis column".into());
        }
        Ok(())
    }
    pub fn model(&self, index: usize) -> Result<Statistics1, String> {
        let a = self
            .analyses
            .get(index)
            .ok_or("Invalid one-variable analysis")?;
        let samples = self.columns.get(a.data)?.to_vec();
        let frequencies = self.columns.frequencies(a.frequency, samples.len())?;
        Ok(Statistics1 {
            samples,
            frequencies,
        })
    }
    pub fn calculate(&self, cancel: &AtomicBool) -> Result<BTreeMap<String, OneResult>, String> {
        self.validate()?;
        let mut result = BTreeMap::new();
        for (index, _) in self.analyses.iter().enumerate().filter(|(_, a)| a.enabled) {
            check(cancel)?;
            result.insert(
                format!("H{}", index + 1),
                OneResult::calculate(&self.model(index)?)
                    .map_err(|e| format!("H{}: {e}", index + 1))?,
            );
        }
        if result.is_empty() {
            return Err("Select at least one H analysis".into());
        }
        Ok(result)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwoResult {
    pub regression: Regression,
    pub summary: PairedSummary,
    pub residuals: Vec<Option<f64>>,
}
impl TwoBook {
    pub fn from_legacy(model: &Statistics2) -> Self {
        let mut book = Self::default();
        book.columns.0[0] = model.pairs.iter().map(|p| p[0]).collect();
        book.columns.0[1] = model.pairs.iter().map(|p| p[1]).collect();
        book.analyses[0].fit = model.fit_kind;
        book
    }
    pub fn validate(&self) -> Result<(), String> {
        self.columns.validate()?;
        if self
            .fit_equations
            .iter()
            .flatten()
            .any(|source| source.len() > 65536)
        {
            return Err("Statistics fit expression exceeds 64 KiB".into());
        }
        if self.analyses.iter().any(|a| {
            !valid_reference(a.x)
                || !valid_reference(a.y)
                || !valid_reference(a.frequency)
                || !a.valid_definition()
        }) {
            return Err("Invalid two-variable analysis definition".into());
        }
        Ok(())
    }
    fn model(
        &self,
        index: usize,
        snapshot: &SessionSnapshot,
        cancel: &Arc<AtomicBool>,
    ) -> Result<(Statistics2, Vec<u64>), String> {
        let a = self
            .analyses
            .get(index)
            .ok_or("Invalid two-variable analysis")?;
        // The G1 Median–Median fit rejects every explicit frequency expression.
        if a.fit == FitKind::MedianMedian && (a.frequency.is_some() || a.frequency_source.is_some())
        {
            return Err("Median-median fit requires an empty Frequency field".into());
        }
        let data = |field, column| {
            if let Some(value) = a.evaluate_reference(field, snapshot, cancel)? {
                let ppl_vm::Value::List(values) = value else {
                    return Err("A statistics axis expression must return a list".into());
                };
                if values.len() > MAX_ROWS {
                    return Err("Statistics columns need at most 1000 rows".into());
                }
                values
                    .iter()
                    .map(|value| {
                        value
                            .expect_real("statistics observation")
                            .map(|value| value.to_f64())
                            .map_err(|error| error.to_string())
                    })
                    .collect()
            } else {
                self.columns.get(column).map(<[f64]>::to_vec)
            }
        };
        let x = data(1, a.x)?;
        let y = data(2, a.y)?;
        if x.len() != y.len() {
            return Err("The X and Y columns must have the same number of observations".into());
        }
        let frequencies = if let Some(value) = a.evaluate_reference(3, snapshot, cancel)? {
            let frequency = |value: &ppl_vm::Value| {
                let real = value
                    .expect_real("statistics frequency")
                    .map_err(|error| error.to_string())?;
                real.is_integer()
                    .then(|| real.to_i128())
                    .flatten()
                    .and_then(|count| u64::try_from(count).ok())
                    .filter(|&count| count <= 9_007_199_254_740_992)
                    .ok_or_else(|| {
                        "Frequencies must be nonnegative integers within exact numerical range"
                            .into()
                    })
            };
            match value {
                ppl_vm::Value::List(values) if values.len() == x.len() => values
                    .iter()
                    .map(frequency)
                    .collect::<Result<Vec<_>, String>>()?,
                ppl_vm::Value::List(_) => {
                    return Err("Statistics frequencies must match the observations".into());
                }
                _ => return Err("A Statistics 2Var frequency expression must return a list".into()),
            }
        } else {
            self.columns.frequencies(a.frequency, x.len())?
        };
        Ok((
            Statistics2 {
                fit_kind: a.fit,
                pairs: x.into_iter().zip(y).map(|(x, y)| [x, y]).collect(),
            },
            frequencies,
        ))
    }
    #[cfg(test)]
    fn fit(
        &self,
        index: usize,
        snapshot: &SessionSnapshot,
        cancel: &Arc<AtomicBool>,
    ) -> Result<Regression, String> {
        self.validate()?;
        check(cancel)?;
        let (model, frequencies) = self.model(index, snapshot, cancel)?;
        self.fit_model(index, snapshot, cancel, &model, &frequencies)
    }

    fn fit_model(
        &self,
        index: usize,
        snapshot: &SessionSnapshot,
        cancel: &AtomicBool,
        model: &Statistics2,
        frequencies: &[u64],
    ) -> Result<Regression, String> {
        check(cancel)?;
        let a = &self.analyses[index];
        let settings = snapshot
            .app_settings
            .get("Statistics 2Var")
            .cloned()
            .unwrap_or_default()
            .effective(&snapshot.settings);
        let limit = if a.fit == FitKind::Logistic {
            match snapshot.globals.get("L") {
                None => None,
                Some(value) => Some(
                    value
                        .to_value()
                        .as_real()
                        .ok_or("Logistic saturation L must be a real number")?
                        .to_f64(),
                ),
            }
        } else {
            None
        };
        let options = FitOptions {
            frequencies,
            logistic_limit: limit,
            user_expression: Some(a.fit_override.as_deref().unwrap_or(&a.user_expression)),
            angle: settings.angle,
            cancel: Some(cancel),
        };
        if a.fit_override.is_some() {
            let manual = Statistics2 {
                fit_kind: FitKind::UserDefined,
                pairs: model.pairs.clone(),
            };
            manual.fit_with_options(&options)
        } else {
            model.fit_with_options(&options)
        }
    }
    pub fn calculate(
        &self,
        snapshot: &SessionSnapshot,
        cancel: &Arc<AtomicBool>,
    ) -> Result<BTreeMap<String, TwoResult>, String> {
        self.validate()?;
        let mut result = BTreeMap::new();
        for (index, _) in self.analyses.iter().enumerate().filter(|(_, a)| a.enabled) {
            check(cancel)?;
            let (model, frequencies) = self
                .model(index, snapshot, cancel)
                .map_err(|error| format!("S{}: {error}", index + 1))?;
            let regression = self
                .fit_model(index, snapshot, cancel, &model, &frequencies)
                .map_err(|e| format!("S{}: {e}", index + 1))?;
            let summary = model.summary_with_frequencies(&frequencies)?;
            let residuals = model
                .pairs
                .iter()
                .enumerate()
                .map(|(row, &[x, y])| {
                    check(cancel)?;
                    if frequencies.get(row).copied().unwrap_or(1) == 0 {
                        return Ok(None);
                    }
                    regression.predict(x).map(|predicted| Some(y - predicted))
                })
                .collect::<Result<Vec<_>, String>>()?;
            result.insert(
                format!("S{}", index + 1),
                TwoResult {
                    regression,
                    summary,
                    residuals,
                },
            );
        }
        if result.is_empty() {
            return Err("Select at least one S analysis".into());
        }
        Ok(result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn five_definitions_share_columns_without_sharing_frequency_or_fit_choices() {
        let mut book = TwoBook::default();
        book.columns.0[0] = vec![0., 1., 2., 3.];
        book.columns.0[1] = vec![1., 2., 2., 4.];
        book.columns.0[2] = vec![1., 2., 3., 4.];
        book.analyses[0].frequency = Some(2);
        book.analyses[1] = TwoAnalysis {
            enabled: true,
            x: Some(0),
            y: Some(1),
            fit: FitKind::UserDefined,
            user_expression: "2*X+1".into(),
            ..Default::default()
        };
        let out = book
            .calculate(
                &ppl_vm::SessionContext::default().snapshot(),
                &Arc::new(AtomicBool::new(false)),
            )
            .unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out["S1"].summary.count, 10);
        assert!((out["S1"].regression.predict(1.23).unwrap() - 1.93).abs() < 1e-10);
        assert_eq!(out["S2"].summary.count, 4);
        assert_eq!(out["S2"].regression.predict(3.).unwrap(), 7.);
        let saved = serde_json::to_value(&book).unwrap();
        let restored: TwoBook = serde_json::from_value(saved.clone()).unwrap();
        assert_eq!(serde_json::to_value(restored).unwrap(), saved);
    }
    #[test]
    fn documented_height_frequency_example_and_repeated_column_references() {
        let mut book = OneBook::default();
        book.columns.0[0] = vec![160., 165., 170., 175., 180.];
        book.columns.0[1] = vec![5., 3., 8., 2., 1.];
        book.analyses[0].frequency = Some(1);
        book.analyses[4] = OneAnalysis {
            enabled: true,
            data: Some(0),
            frequency: None,
            ..Default::default()
        };
        let out = book.calculate(&AtomicBool::new(false)).unwrap();
        assert_eq!(out["H1"].basic.count, 19);
        assert!((out["H1"].basic.mean - 3185. / 19.).abs() < 1e-10);
        assert_eq!(out["H5"].basic.mean, 170.);
        assert_eq!(out["H5"].basic.count, 5);
    }
    #[test]
    fn original_g1_four_value_statistics_and_single_observation_error() {
        let mut book = OneBook::default();
        book.columns.0[0] = vec![1., 2., 3., 4.];
        let out = book.calculate(&AtomicBool::new(false)).unwrap();
        let h = &out["H1"];
        assert_eq!(
            (
                h.basic.count,
                h.basic.minimum,
                h.quartile1,
                h.basic.median,
                h.quartile3,
                h.basic.maximum
            ),
            (4, 1., 1.5, 2.5, 3.5, 4.)
        );
        assert_eq!((h.basic.sum, h.sum_squares, h.basic.mean), (10., 30., 2.5));
        assert!((h.sample_standard_deviation - 1.29099444874).abs() < 1e-11);
        book.columns.0[0] = vec![7.];
        assert!(
            book.calculate(&AtomicBool::new(false))
                .unwrap_err()
                .contains("Insufficient statistics data")
        );
    }
    #[test]
    fn excluded_domain_rows_do_not_break_a_valid_fit_or_expand_frequencies() {
        let mut book = TwoBook::default();
        book.columns.0[0] = vec![-1., 1., 2., 3.];
        book.columns.0[1] = vec![7., 2., 8., 18.];
        book.columns.0[2] = vec![0., 1., 1., 1.];
        book.analyses[0].fit = FitKind::Power;
        book.analyses[0].frequency = Some(2);
        let out = book
            .calculate(
                &ppl_vm::SessionContext::default().snapshot(),
                &Arc::new(AtomicBool::new(false)),
            )
            .unwrap();
        assert_eq!(out["S1"].summary.count, 3);
        assert_eq!(out["S1"].residuals.len(), 4);
        assert!(out["S1"].residuals[0].is_none());
        assert!((out["S1"].regression.predict(4.).unwrap() - 32.).abs() < 1e-9);
        assert!(
            OneBook::from_legacy(&Statistics1 {
                samples: vec![1.],
                frequencies: vec![9_007_199_254_740_993]
            })
            .is_err()
        );
    }
    #[test]
    fn invalid_lengths_frequencies_selection_and_cancellation_are_explicit_errors() {
        let mut book = TwoBook::default();
        book.columns.0[0] = vec![0., 1.];
        book.columns.0[1] = vec![1.];
        assert!(
            book.model(
                0,
                &ppl_vm::SessionContext::default().snapshot(),
                &Arc::new(AtomicBool::new(false))
            )
            .is_err()
        );
        book.columns.0[1].push(3.);
        book.analyses[0].frequency = Some(2);
        book.columns.0[2] = vec![1., 0.5];
        assert!(
            book.model(
                0,
                &ppl_vm::SessionContext::default().snapshot(),
                &Arc::new(AtomicBool::new(false))
            )
            .is_err()
        );
        book.analyses[0].frequency = None;
        assert!(
            book.calculate(
                &ppl_vm::SessionContext::default().snapshot(),
                &Arc::new(AtomicBool::new(true))
            )
            .is_err()
        );
        book.analyses[0].enabled = false;
        assert!(
            book.calculate(
                &ppl_vm::SessionContext::default().snapshot(),
                &Arc::new(AtomicBool::new(false))
            )
            .is_err()
        );
        assert_eq!(Columns::label(9, true), "C0");
        assert_eq!(Columns::label(9, false), "D0");
    }
    #[test]
    fn logistic_definition_uses_the_current_home_saturation_variable() {
        let mut book = TwoBook::default();
        book.columns.0[0] = vec![0., 1., 2., 3., 4., 5.];
        book.columns.0[1] = book.columns.0[0]
            .iter()
            .map(|x| 10. / (1. + 2. * (-0.7_f64 * x).exp()))
            .collect();
        book.analyses[0].fit = FitKind::Logistic;
        let mut snapshot = ppl_vm::SessionContext::default().snapshot();
        snapshot.globals.insert(
            "L".into(),
            ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(10)),
        );
        let fit = book
            .fit(0, &snapshot, &Arc::new(AtomicBool::new(false)))
            .unwrap();
        assert_eq!(fit.coefficients[0], 10.);
        assert!((fit.predict(2.).unwrap() - book.columns.0[1][2]).abs() < 1e-8);
        snapshot.globals.insert(
            "L".into(),
            ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(-1)),
        );
        assert!(
            book.fit(0, &snapshot, &Arc::new(AtomicBool::new(false)))
                .unwrap_err()
                .contains("L must be zero or positive")
        );
    }
    #[test]
    fn user_curve_uses_the_statistics_app_angle_without_changing_home() {
        let mut snapshot = ppl_vm::SessionContext::default().snapshot();
        snapshot.settings.angle = ppl_math::AngleMode::Radians;
        snapshot.app_settings.insert(
            "Statistics 2Var".into(),
            ppl_vm::AppSettings {
                angle: 2,
                ..Default::default()
            },
        );
        let mut book = TwoBook::default();
        book.columns.0[0] = vec![0., 30., 90.];
        book.columns.0[1] = vec![0., 0.5, 1.];
        book.analyses[0].fit = FitKind::UserDefined;
        book.analyses[0].user_expression = "SIN(X)".into();
        let fit = book
            .fit(0, &snapshot, &Arc::new(AtomicBool::new(false)))
            .unwrap();
        assert!((fit.predict(30.).unwrap() - 0.5).abs() < 1e-12);
        assert_eq!(snapshot.settings.angle, ppl_math::AngleMode::Radians);
    }
    fn median_median_original_frequency_cases(observations: usize) {
        let controls: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/native-statistics-two-reference-controls.json"
        ))
        .unwrap();
        let snapshot = ppl_vm::SessionContext::default().snapshot();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut exercised = 0;
        for case in controls["median_median_cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|case| case["observations"].as_u64() == Some(observations as u64))
        {
            let mut book = TwoBook::default();
            book.columns.0[0] = case["x"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap())
                .collect();
            book.columns.0[1] = case["y"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap())
                .collect();
            book.analyses[0].fit = FitKind::MedianMedian;
            if case["frequency"]["kind"] == "scalar" {
                book.analyses[0].frequency_source =
                    Some(case["frequency"]["source"].as_str().unwrap().to_owned());
            } else if case["frequency"]["kind"] == "column" {
                book.columns.0[2] = case["frequency"]["values"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_f64().unwrap())
                    .collect();
                book.analyses[0].frequency = Some(2);
            }
            assert!(
                book.validate().is_ok(),
                "Definitions remain editable before calculation"
            );
            if case["expected_success"].as_bool().unwrap() {
                let results = book.calculate(&snapshot, &cancel).unwrap();
                assert_eq!(results["S1"].summary.count, observations);
                assert!((results["S1"].regression.predict(2.).unwrap() - 5.).abs() < 1e-10);
                assert!(plots::two(&book, &snapshot, &cancel).is_ok());
            } else {
                for error in [
                    book.fit(0, &snapshot, &cancel).unwrap_err(),
                    book.calculate(&snapshot, &cancel).unwrap_err(),
                    plots::two(&book, &snapshot, &cancel).unwrap_err(),
                ] {
                    assert!(
                        error.contains("empty Frequency"),
                        "{}: {error}",
                        case["case"]
                    );
                }
            }
            exercised += 1;
        }
        assert_eq!(exercised, if observations == 3 { 5 } else { 4 });
    }
    #[test]
    fn original_median_median_n3_accepts_blank_and_rejects_unit_and_nonunit_columns() {
        median_median_original_frequency_cases(3);
    }
    #[test]
    fn original_median_median_n4_accepts_blank_and_rejects_unit_column() {
        median_median_original_frequency_cases(4);
    }
    #[test]
    fn original_median_median_n5_accepts_blank_and_rejects_unit_column() {
        median_median_original_frequency_cases(5);
    }
    #[test]
    fn original_median_median_n6_accepts_blank_and_rejects_unit_column() {
        median_median_original_frequency_cases(6);
    }
    #[test]
    fn original_invalid_five_analysis_workflow_requires_only_median_frequency_correction() {
        let controls: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/native-statistics-two-reference-controls.json"
        ))
        .unwrap();
        let original = &controls["invalid_five"];
        let mut book = TwoBook::default();
        for (column, key) in [(0, "x"), (1, "y"), (2, "frequency_values")] {
            book.columns.0[column] = original[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n.as_f64().unwrap())
                .collect();
        }
        for (analysis, fit) in book.analyses.iter_mut().zip([
            FitKind::Linear,
            FitKind::Power,
            FitKind::Inverse,
            FitKind::MedianMedian,
            FitKind::Linear,
        ]) {
            *analysis = TwoAnalysis {
                enabled: true,
                x: Some(0),
                y: Some(1),
                frequency: Some(2),
                fit,
                ..Default::default()
            };
        }
        let mut snapshot = ppl_vm::SessionContext::default().snapshot();
        snapshot.active_app = "Statistics 2Var".into();
        for (name, value) in ["Xmin", "Xmax", "Ymin", "Ymax"].into_iter().zip(
            controls["valid_five_s4_blank"]["after_startview_1_1"]
                .as_array()
                .unwrap(),
        ) {
            snapshot.globals.insert(
                name.into(),
                ppl_vm::PortableValue::Real(
                    ppl_math::Real::from_f64(value.as_f64().unwrap()).unwrap(),
                ),
            );
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let before = serde_json::to_value(&book).unwrap();
        let error = book.calculate(&snapshot, &cancel).unwrap_err();
        assert!(
            error.starts_with("S4:") && error.contains("empty Frequency"),
            "{error}"
        );
        assert!(
            plots::two(&book, &snapshot, &cancel)
                .unwrap_err()
                .contains("empty Frequency")
        );
        assert_eq!(serde_json::to_value(&book).unwrap(), before);
        book.analyses[3].frequency = None;
        let results = book.calculate(&snapshot, &cancel).unwrap();
        for (index, count) in controls["valid_five_s4_blank"]["counts"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            assert_eq!(
                results[&format!("S{}", index + 1)].summary.count,
                count.as_u64().unwrap() as usize
            );
        }
        let plot = plots::two(&book, &snapshot, &cancel).unwrap();
        let inverse = plot
            .labels
            .iter()
            .zip(&plot.curves)
            .filter(|(label, _)| label.starts_with("S3 ") && label.ends_with("fit"))
            .map(|(_, curve)| curve)
            .collect::<Vec<_>>();
        assert_eq!(
            inverse.len(),
            2,
            "The genuine preserved window straddles the inverse pole"
        );
        assert!(inverse.iter().any(|curve| curve.iter().all(|p| p[0] < 0.)));
        assert!(inverse.iter().any(|curve| curve.iter().all(|p| p[0] > 0.)));
    }
}
