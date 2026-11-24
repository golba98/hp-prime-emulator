//! Typed one-variable observations. Calculator memory remains the source of D
//! columns; projection to binary floats belongs only to plot geometry.
use super::OneAnalysis;
use crate::ide::native_application_access::geometry::ReadSnapshot;
use ppl_math::{NumericVersion, Real};
use ppl_vm::{PortableValue, SessionSnapshot, Value};
use std::{
    cmp::Ordering as Comparison,
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub const VARIABLES: [&str; 13] = [
    "NbItem", "MinVal", "Q₁", "MedVal", "Q₃", "MaxVal", "ΣX", "ΣX2", "MeanX", "sX", "σX", "serrX",
    "ssX",
];

/// Persisted result names paired with the numeric report's stable JSON paths.
pub const DISPLAY_FIELDS: [(&str, &str); 13] = [
    ("count", "NbItem"),
    ("minimum", "MinVal"),
    ("quartile1", "Q₁"),
    ("median", "MedVal"),
    ("quartile3", "Q₃"),
    ("maximum", "MaxVal"),
    ("sum", "ΣX"),
    ("sum_squares", "ΣX2"),
    ("mean", "MeanX"),
    ("sample_standard_deviation", "sX"),
    ("population_standard_deviation", "σX"),
    ("standard_error", "serrX"),
    ("sum_squared_deviations", "ssX"),
];

#[derive(Debug, Clone)]
pub struct Data {
    pub values: Vec<Real>,
    pub(super) weights: Vec<Frequency>,
    pub(super) count: Frequency,
    pub numeric_version: NumericVersion,
    explicit_weights: bool,
}
#[derive(Debug, Clone)]
pub struct Summary {
    pub variables: BTreeMap<String, PortableValue>,
}
fn check(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Statistics calculation interrupted".into())
    } else {
        Ok(())
    }
}
fn real(value: &Value) -> Result<Real, String> {
    value
        .expect_real("statistics observation")
        .map_err(|error| error.to_string())
}
/// Retain exact whole counts for grouped plots and trace labels where they fit.
/// Larger or fractional weights use calculator arithmetic throughout aggregation.
#[derive(Debug, Clone, Copy)]
pub(super) enum Frequency {
    Whole(u64),
    Decimal(Real),
}
impl Frequency {
    pub(super) const ZERO: Self = Self::Whole(0);

    fn from_value(value: &Value) -> Result<Self, String> {
        let value = real(value)?;
        if value.is_negative() {
            return Err("Statistics frequency must be nonnegative".into());
        }
        if value.is_integer()
            && let Some(count) = value
                .to_i128()
                .and_then(|integer| u64::try_from(integer).ok())
        {
            return Ok(Self::Whole(count));
        }
        Ok(Self::Decimal(exact_input(value)))
    }

    pub(super) fn is_zero(self) -> bool {
        match self {
            Self::Whole(value) => value == 0,
            Self::Decimal(value) => value.is_zero(),
        }
    }

    pub(super) fn to_f64(self) -> f64 {
        match self {
            Self::Whole(value) => value as f64,
            Self::Decimal(value) => value.to_f64(),
        }
    }

    pub(super) fn split(self) -> Result<(u64, Option<f64>), String> {
        match self {
            Self::Whole(value) => Ok((value, None)),
            Self::Decimal(value) => {
                let whole = value.floor();
                let count = whole
                    .to_i128()
                    .and_then(|n| u64::try_from(n).ok())
                    .ok_or("Dot frequency exceeds its integer range")?;
                let remainder = value
                    .sub(whole)
                    .map_err(|error| error.to_string())?
                    .to_f64();
                Ok((count, (remainder > 0.).then_some(remainder)))
            }
        }
    }

    fn real(self) -> Real {
        match self {
            Self::Whole(value) => Real::from_u64(value),
            Self::Decimal(value) => value,
        }
    }

    fn exact(self) -> Real {
        match self {
            Self::Whole(value) => Real::exact_rational(false, value, 1).expect("whole frequency"),
            Self::Decimal(value) => value,
        }
    }

    pub(super) fn checked_add(self, other: Self) -> Result<Self, String> {
        if let (Self::Whole(left), Self::Whole(right)) = (self, other)
            && let Some(total) = left.checked_add(right)
        {
            return Ok(Self::Whole(total));
        }
        self.exact()
            .add(other.exact())
            .map(Self::Decimal)
            .map_err(|error| error.to_string())
    }
}

fn exact_input(value: Real) -> Real {
    match value.origin() {
        ppl_math::RealOrigin::LegacyBcd | ppl_math::RealOrigin::Decimal => {
            Real::parse_versioned(&value.to_string(), NumericVersion::Current15751)
                .expect("a stored real has valid calculator source")
        }
        _ => value,
    }
}

impl Data {
    pub fn evaluate(
        analysis: &OneAnalysis,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        Self::evaluate_with_geometry(analysis, snapshot, cancel, None)
    }

    pub(crate) fn evaluate_with_geometry(
        analysis: &OneAnalysis,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
        geometry: ReadSnapshot,
    ) -> Result<Self, String> {
        check(&cancel)?;
        let evaluate = |source: &str| {
            super::super::native_readonly::evaluate_with_geometry(
                source,
                "Statistics 1Var",
                snapshot,
                [],
                cancel.clone(),
                geometry.clone(),
            )
        };
        let source = analysis.data_expression();
        if source.is_empty() {
            return Err("Choose a statistics data expression".into());
        }
        let Value::List(values) = evaluate(&source)? else {
            return Err("A statistics data expression must return a list".into());
        };
        let values = values.iter().map(real).collect::<Result<Vec<_>, _>>()?;
        // Charts that count raw observations retain the saved frequency
        // expression. It becomes active again when the chart uses weights.
        let source = if analysis.chart.uses_frequencies() {
            analysis.frequency_expression()
        } else {
            String::new()
        };
        let explicit_weights = !source.is_empty();
        let weights = if !explicit_weights {
            vec![Frequency::Whole(1); values.len()]
        } else {
            match evaluate(&source)? {
                Value::List(frequencies) => frequencies
                    .iter()
                    .map(Frequency::from_value)
                    .collect::<Result<Vec<_>, _>>()?,
                scalar => {
                    let mut diagnostics = ppl_syntax::Diagnostics::default();
                    let tokens = ppl_syntax::lexer::lex(&source, &mut diagnostics);
                    if diagnostics.has_errors()
                        || !matches!(tokens.as_slice(), [number, end]
                            if matches!(number.tok, ppl_syntax::lexer::Tok::Number(_))
                                && matches!(end.tok, ppl_syntax::lexer::Tok::Eof))
                    {
                        return Err(
                            "A scalar statistics frequency requires a numeric literal".into()
                        );
                    }
                    vec![Frequency::from_value(&scalar)?; values.len()]
                }
            }
        };
        Self::from_frequencies(
            values,
            weights,
            snapshot.settings.numeric_version,
            explicit_weights,
        )
    }
    pub fn new(
        values: Vec<Real>,
        weights: Vec<u64>,
        numeric_version: NumericVersion,
    ) -> Result<Self, String> {
        Self::from_frequencies(
            values,
            weights.into_iter().map(Frequency::Whole).collect(),
            numeric_version,
            true,
        )
    }

    fn from_frequencies(
        values: Vec<Real>,
        weights: Vec<Frequency>,
        numeric_version: NumericVersion,
        explicit_weights: bool,
    ) -> Result<Self, String> {
        if values.len() != weights.len() {
            return Err("Statistics frequencies must match the observations".into());
        }
        let count = weights
            .iter()
            .copied()
            .try_fold(Frequency::ZERO, Frequency::checked_add)?;
        Ok(Self {
            values,
            weights,
            count,
            numeric_version,
            explicit_weights,
        })
    }

    fn quartile(
        &self,
        sorted: &[(Real, Frequency)],
        quarter: u64,
        cancel: &AtomicBool,
    ) -> Result<Real, String> {
        let mut cumulative = Frequency::ZERO;
        let midpoint = |left: Real, right: Real| {
            left.add(right)
                .and_then(|sum| sum.div(Real::from_i64(2)))
                .map_err(|error| error.to_string())
        };
        let target = match self.count {
            Frequency::Whole(_) => None,
            Frequency::Decimal(total) => Some(
                total
                    .mul(Real::exact_rational(false, quarter, 4).unwrap())
                    .map_err(|error| error.to_string())?,
            ),
        };
        // With the blank default frequency, G1 excludes the middle row when
        // forming the two halves of an odd-length sample.
        let whole_target = match self.count {
            Frequency::Whole(count) => {
                let n = u128::from(count);
                match (self.explicit_weights, quarter) {
                    (false, 1) => (n / 2) * 2,
                    (false, 3) => n.div_ceil(2) * 4 + (n / 2) * 2,
                    _ => n * u128::from(quarter),
                }
            }
            Frequency::Decimal(_) => 0,
        };
        for (index, &(value, weight)) in sorted.iter().enumerate() {
            check(cancel)?;
            cumulative = cumulative.checked_add(weight)?;
            let comparison = if let Some(target) = target {
                cumulative.exact().cmp_value(target)
            } else {
                let Frequency::Whole(count) = cumulative else {
                    unreachable!("whole total requires whole frequencies");
                };
                (u128::from(count) * 4).cmp(&whole_target)
            };
            match comparison {
                Comparison::Greater => return Ok(value),
                Comparison::Equal => {
                    let next = sorted.get(index + 1).map_or(value, |&(next, _)| next);
                    return midpoint(value, next);
                }
                Comparison::Less => {}
            }
        }
        Err("Statistics quartile exceeds the frequency total".into())
    }

    pub fn summary(&self, cancel: &AtomicBool) -> Result<Summary, String> {
        check(cancel)?;
        let n = self.count.real();
        if self.values.len() < 2 || n.is_zero() {
            return Err("Insufficient statistics data".into());
        }
        // Statistics accumulators retain the exact stored observations through
        // intermediate arithmetic. Returned results are twelve-digit decimal
        // objects: original15751 MeanX*3 is 6.99999999999 for a mean of7/3.
        let mut sorted = self
            .values
            .iter()
            .copied()
            .map(exact_input)
            .zip(self.weights.iter().copied())
            .collect::<Vec<_>>();
        sorted.sort_by(|a, b| a.0.cmp_value(b.0));
        // Report extrema and moments exclude zero weights. The exact-boundary
        // quartile rule still uses the next sorted row, even if it has weight 0.
        let positive = sorted.iter().filter(|(_, weight)| !weight.is_zero());
        let mut sum = Real::ZERO;
        let mut squares = Real::ZERO;
        for &(x, weight) in positive.clone() {
            check(cancel)?;
            let w = weight.real();
            sum = sum
                .add(x.mul(w).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            squares = squares
                .add(x.mul(x).and_then(|x| x.mul(w)).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        }
        let mean = sum.div(n).map_err(|e| e.to_string())?;
        let mut deviations = Real::ZERO;
        for &(x, weight) in positive.clone() {
            check(cancel)?;
            let difference = x.sub(mean).map_err(|e| e.to_string())?;
            deviations = deviations
                .add(
                    difference
                        .mul(difference)
                        .and_then(|square| square.mul(weight.real()))
                        .map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
        }
        let population_variance = deviations.div(n).map_err(|e| e.to_string())?;
        let finite = |value: Real| {
            PortableValue::Real(if self.numeric_version == NumericVersion::Current15751 {
                value.decimal()
            } else {
                Real::parse(&value.to_string())
                    .expect("a calculated real has valid calculator source")
            })
        };
        let undefined = PortableValue::Symbolic(ppl_cas::CasValue::Undefined);
        let (sample_deviation, standard_error) = match n.cmp_value(Real::ONE) {
            Comparison::Less if deviations.is_zero() => (finite(Real::ZERO), finite(Real::ZERO)),
            Comparison::Equal if !deviations.is_zero() => {
                let infinity = PortableValue::Symbolic(ppl_cas::CasValue::Infinity(
                    ppl_cas::InfinitySign::Positive,
                ));
                (infinity.clone(), infinity)
            }
            Comparison::Less | Comparison::Equal => (undefined.clone(), undefined),
            Comparison::Greater => {
                let denominator = match self.count {
                    Frequency::Whole(count) => Real::from_u64(count - 1),
                    Frequency::Decimal(count) => count.sub(Real::ONE).map_err(|e| e.to_string())?,
                };
                let sample_variance = deviations.div(denominator).map_err(|e| e.to_string())?;
                (
                    finite(sample_variance.sqrt().map_err(|e| e.to_string())?),
                    finite(
                        sample_variance
                            .div(n)
                            .and_then(Real::sqrt)
                            .map_err(|e| e.to_string())?,
                    ),
                )
            }
        };
        let variables = [
            ("NbItem", n),
            ("MinVal", positive.clone().next().unwrap().0),
            ("Q₁", self.quartile(&sorted, 1, cancel)?),
            ("MedVal", self.quartile(&sorted, 2, cancel)?),
            ("Q₃", self.quartile(&sorted, 3, cancel)?),
            ("MaxVal", positive.clone().next_back().unwrap().0),
            ("ΣX", sum),
            ("ΣX2", squares),
            ("MeanX", mean),
            ("σX", population_variance.sqrt().map_err(|e| e.to_string())?),
            ("ssX", deviations),
        ];
        Ok(Summary {
            variables: variables
                .into_iter()
                .map(|(name, value)| (name.into(), finite(value)))
                .chain([
                    ("sX".into(), sample_deviation),
                    ("serrX".into(), standard_error),
                ])
                .collect(),
        })
    }
}
impl Summary {
    /// The five-number distribution summary always consists of finite reals.
    pub(super) fn finite(&self, name: &str) -> Real {
        let PortableValue::Real(value) = self.variables[name] else {
            unreachable!("a distribution summary field must be finite");
        };
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(source: &str) -> Real {
        Real::parse_versioned(source, ppl_math::NumericVersion::Current15751).unwrap()
    }
    #[test]
    fn original_frequency_three_control_keeps_all_thirteen_named_results() {
        let data = Data::new(
            vec![parse("1"), parse("2"), parse("4")],
            vec![3, 3, 3],
            NumericVersion::Current15751,
        )
        .unwrap();
        let summary = data.summary(&AtomicBool::new(false)).unwrap();
        let controls: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/native-statistics-one-definition-controls.json"
        ))
        .unwrap();
        for (name, text) in controls["frequency_three"]["results"].as_object().unwrap() {
            assert_eq!(
                summary.finite(name).format(Default::default()),
                text.as_str().unwrap(),
                "{name}"
            );
        }
        assert_eq!(summary.variables.len(), 13);
        assert!(
            summary
                .variables
                .values()
                .all(|value| matches!(value, PortableValue::Real(value) if value.origin() == ppl_math::RealOrigin::Decimal))
        );
        assert_eq!(
            summary
                .finite("MeanX")
                .mul(parse("3"))
                .unwrap()
                .format(Default::default()),
            "6.99999999999"
        );
        assert_eq!(
            summary
                .finite("NbItem")
                .div(parse("7"))
                .unwrap()
                .mul(parse("7"))
                .unwrap()
                .format(Default::default()),
            "8.99999999997"
        );
    }
    #[test]
    fn live_vm_columns_scalar_expressions_and_exact_cells_do_not_use_float_caches() {
        let mut session = ppl_vm::SessionContext::default();
        session.settings.numeric_version = ppl_math::NumericVersion::Current15751;
        session
            .set_variable(
                "D1",
                PortableValue::List(vec![
                    PortableValue::Real(parse("1234567890123456789")),
                    PortableValue::Real(parse("1E400")),
                ]),
            )
            .unwrap();
        let snapshot = session.snapshot();
        let analysis = OneAnalysis {
            data_source: Some("D1".into()),
            frequency_source: Some("2".into()),
            ..Default::default()
        };
        let data = Data::evaluate(&analysis, &snapshot, Arc::new(AtomicBool::new(false))).unwrap();
        assert_eq!(data.values[0].ppl_source(), "1.234567890123456789e18");
        assert_eq!(data.values[1].format(Default::default()), "1E400");
        assert!(matches!(
            &data.weights[..],
            [Frequency::Whole(2), Frequency::Whole(2)]
        ));
        assert_eq!(session.snapshot().globals, snapshot.globals);
        let cancelled = Arc::new(AtomicBool::new(true));
        assert!(Data::evaluate(&analysis, &snapshot, cancelled).is_err());
    }
    #[test]
    fn original_chart_selection_controls_summary_weights_without_clearing_the_source() {
        use crate::ide::native_statistics::one::Chart;

        let mut context = ppl_vm::SessionContext::default();
        let list = |values: &[&str]| {
            PortableValue::List(
                values
                    .iter()
                    .map(|value| PortableValue::Real(parse(value)))
                    .collect(),
            )
        };
        context
            .set_variable("D1", list(&["1", "3", "5", "9"]))
            .unwrap();
        context
            .set_variable("D2", list(&["0", "1.25", "3.75", "1"]))
            .unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut analysis = OneAnalysis {
            data_source: Some("D1".into()),
            frequency_source: Some("D2".into()),
            ..Default::default()
        };
        for chart in Chart::ALL {
            analysis.chart = chart;
            let summary = Data::evaluate(&analysis, &context.snapshot(), cancel.clone())
                .unwrap()
                .summary(&cancel)
                .unwrap();
            let expected = match chart.code() {
                1 | 2 | 4 | 8 => ["6", "3", "5", "5.25"],
                _ => ["4", "1", "4", "4.5"],
            };
            for (field, value) in ["NbItem", "MinVal", "MedVal", "MeanX"]
                .into_iter()
                .zip(expected)
            {
                assert_eq!(
                    summary.finite(field).to_string(),
                    value,
                    "{chart:?} {field}"
                );
            }
            assert_eq!(analysis.frequency_expression(), "D2");
        }

        context.set_variable("D2", list(&["1", "2"])).unwrap();
        let snapshot = context.snapshot();
        analysis.chart = Chart::Bar;
        assert!(Data::evaluate(&analysis, &snapshot, cancel.clone()).is_ok());
        analysis.chart = Chart::Histogram;
        assert!(Data::evaluate(&analysis, &snapshot, cancel).is_err());
    }
    #[test]
    fn original_frequency_controls_accept_list_expressions_and_only_literal_scalars() {
        let mut context = ppl_vm::SessionContext::default();
        let list = |numbers: &[i64]| {
            PortableValue::List(
                numbers
                    .iter()
                    .map(|&n| PortableValue::Real(Real::from_i64(n)))
                    .collect(),
            )
        };
        context.set_variable("D1", list(&[1, 2, 4])).unwrap();
        context.set_variable("D2", list(&[2, 2, 2])).unwrap();
        context
            .set_variable("A", PortableValue::Real(Real::from_i64(2)))
            .unwrap();
        let controls: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/native-statistics-one-workflow-controls.json"
        ))
        .unwrap();
        for control in controls["frequency_cases"].as_array().unwrap() {
            let source = control["expression"].as_str().unwrap();
            let wanted = control.get("count").map(|value| value.as_u64().unwrap());
            let analysis = OneAnalysis {
                data_source: Some("D1".into()),
                frequency_source: Some(source.into()),
                ..Default::default()
            };
            let result = Data::evaluate(
                &analysis,
                &context.snapshot(),
                Arc::new(AtomicBool::new(false)),
            );
            match wanted {
                Some(count) => assert!(
                    matches!(result.unwrap().count, Frequency::Whole(value) if value == count),
                    "{source}"
                ),
                None => assert!(result.is_err(), "{source}"),
            }
        }
    }
    #[test]
    fn original_fractional_and_explicit_weight_controls_match_all_finite_results() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/native-statistics-one-fractional-controls.json"
        ))
        .unwrap();
        for control in fixture["summary_cases"].as_array().unwrap() {
            let case = control["case"].as_str().unwrap();
            let mut session = ppl_vm::SessionContext::default();
            session.settings.numeric_version = NumericVersion::Current15751;
            for (name, field) in [("D1", "data"), ("D2", "frequencies")] {
                if let Some(source) = control[field].as_str() {
                    session
                        .evaluate_home(
                            &format!("{name}:={source}"),
                            Box::new(ppl_vm::HeadlessHost::default()),
                            ppl_vm::Limits::default(),
                        )
                        .result
                        .unwrap_or_else(|error| panic!("{case}: {error}"));
                }
            }
            let cancel = Arc::new(AtomicBool::new(false));
            let analysis = OneAnalysis {
                data_source: Some("D1".into()),
                frequency_source: control["frequency_source"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned),
                ..Default::default()
            };
            let outcome = Data::evaluate(&analysis, &session.snapshot(), cancel.clone())
                .and_then(|data| data.summary(&cancel));
            if let Some(expected) = control["results"].as_object() {
                let summary = outcome.unwrap_or_else(|error| panic!("{case}: {error}"));
                for (name, value) in expected {
                    let result = &summary.variables[name];
                    if let Some(expected) = value.as_str() {
                        assert_eq!(
                            result.display(&session.settings),
                            expected,
                            "{case}: {name}"
                        );
                    } else {
                        assert!(value.is_null());
                        assert_eq!(
                            result,
                            &PortableValue::Symbolic(ppl_cas::CasValue::Undefined),
                            "{case}: {name}"
                        );
                    }
                }
            } else {
                assert!(outcome.is_err(), "{case}");
            }
        }
    }

    #[test]
    fn original_low_weight_summary_keeps_defined_results_and_types() {
        let cancel = AtomicBool::new(false);
        let compute = |values: [&str; 2], weights: [&str; 2]| {
            Data::from_frequencies(
                values.into_iter().map(parse).collect(),
                weights
                    .into_iter()
                    .map(|s| Frequency::from_value(&Value::real(parse(s))).unwrap())
                    .collect(),
                NumericVersion::Current15751,
                true,
            )
            .unwrap()
            .summary(&cancel)
        };
        let settings = ppl_vm::Settings::default();
        for (values, weights, count, mean, population, spread) in [
            (["1", "3"], [".5", ".5"], "1", "2", "1", "+Inf"),
            (["1", "3"], [".25", ".25"], "0.5", "2", "1", "undef"),
            (["2", "2"], [".25", ".25"], "0.5", "2", "0", "0"),
            (["4", "4"], [".5", ".5"], "1", "4", "0", "undef"),
            (["1", "3"], ["0", "1"], "1", "3", "0", "undef"),
        ] {
            let summary = compute(values, weights).unwrap();
            for (field, expected) in [
                ("NbItem", count),
                ("MeanX", mean),
                ("σX", population),
                ("sX", spread),
                ("serrX", spread),
            ] {
                let got = &summary.variables[field];
                assert_eq!(
                    got.display(&settings),
                    expected,
                    "{field} for {values:?} / {weights:?}"
                );
                if expected != "undef" {
                    assert_eq!(got.to_value().type_code(), 0);
                }
            }
        }
        assert!(compute(["1", "3"], ["0", "0"]).is_err());
    }

    #[test]
    fn zero_weight_rows_and_invalid_frequency_values_remain_distinct() {
        let data = Data::new(
            vec![parse("-100"), parse("1"), parse("3")],
            vec![0, 1, 1],
            NumericVersion::Current15751,
        )
        .unwrap();
        let summary = data.summary(&AtomicBool::new(false)).unwrap();
        assert_eq!(summary.finite("MinVal"), parse("1"));
        assert_eq!(summary.finite("MeanX"), parse("2"));
        assert!(Frequency::from_value(&Value::Real(parse("-1"))).is_err());
        assert!(
            Data::new(vec![parse("1")], vec![1], NumericVersion::Current15751)
                .unwrap()
                .summary(&AtomicBool::new(false))
                .is_err()
        );
        assert!(Data::new(vec![parse("1")], vec![1, 2], NumericVersion::Current15751).is_err());
    }

    #[test]
    fn statistics_frequency_totals_follow_original_above_the_host_integer_range() {
        let mut context = ppl_vm::SessionContext::default();
        context.settings.numeric_version = NumericVersion::Current15751;
        context
            .set_variable(
                "D1",
                PortableValue::List(vec![
                    PortableValue::Real(Real::from_i64(-1)),
                    PortableValue::Real(Real::from_i64(2)),
                    PortableValue::Real(Real::from_i64(4)),
                ]),
            )
            .unwrap();
        let analysis = OneAnalysis {
            data_source: Some("D1".into()),
            frequency_source: Some("D2".into()),
            ..Default::default()
        };
        for (weights, expected) in [
            (
                "{1e19,1e19,1e19}",
                ["3e19", "-1", "2", "4", "1.66666666667", "5e19"],
            ),
            (
                "{1e20,3e20,5e20}",
                ["9e20", "2", "4", "4", "2.77777777778", "2.5e21"],
            ),
            (
                "{1e50,3e50,5e50}",
                ["9e50", "2", "4", "4", "2.77777777778", "2.5e51"],
            ),
            ("{1e20,0.5,2.25}", ["1e20", "-1", "-1", "-1", "-1", "-1e20"]),
        ] {
            context
                .evaluate_home(
                    &format!("D2:={weights}"),
                    Box::new(ppl_vm::HeadlessHost::default()),
                    ppl_vm::Limits::default(),
                )
                .result
                .unwrap();
            let cancel = Arc::new(AtomicBool::new(false));
            let data = Data::evaluate(&analysis, &context.snapshot(), cancel.clone()).unwrap();
            assert!(matches!(data.count, Frequency::Decimal(_)));
            let summary = data.summary(&cancel).unwrap();
            for (name, value) in ["NbItem", "Q₁", "MedVal", "Q₃", "MeanX", "ΣX"]
                .into_iter()
                .zip(expected)
            {
                let actual = summary.finite(name);
                assert!(
                    actual.cmp_value(parse(value)).is_eq(),
                    "{weights} {name}: {actual}"
                );
            }
        }
    }
}
