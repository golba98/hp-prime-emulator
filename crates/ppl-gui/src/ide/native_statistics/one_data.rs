//! Typed one-variable observations. Calculator memory remains the source of D
//! columns; projection to binary floats belongs only to plot geometry.
use super::OneAnalysis;
use ppl_math::{NumericVersion, Real};
use ppl_vm::{PortableValue, SessionSnapshot, Value};
use std::{
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

#[derive(Debug, Clone)]
pub struct Data {
    pub values: Vec<Real>,
    pub weights: Vec<u64>,
    pub count: u64,
    pub numeric_version: NumericVersion,
}
#[derive(Debug, Clone)]
pub struct Summary {
    pub variables: BTreeMap<String, Real>,
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
fn weight(value: &Value) -> Result<u64, String> {
    let value = real(value)?;
    if !value.is_integer() || value.is_negative() {
        return Err("Statistics frequency must be a nonnegative integer".into());
    }
    value
        .to_i128()
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| "Statistics frequency exceeds its integer range".into())
}
impl Data {
    pub fn evaluate(
        analysis: &OneAnalysis,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        check(&cancel)?;
        let evaluate = |source: &str| {
            super::super::native_readonly::evaluate(
                source,
                "Statistics 1Var",
                snapshot,
                [],
                cancel.clone(),
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
        let source = analysis.frequency_expression();
        let weights = if source.is_empty() {
            vec![1; values.len()]
        } else {
            match evaluate(&source)? {
                Value::List(frequencies) => frequencies
                    .iter()
                    .map(weight)
                    .collect::<Result<Vec<_>, _>>()?,
                scalar => vec![weight(&scalar)?; values.len()],
            }
        };
        Self::new(values, weights, snapshot.settings.numeric_version)
    }
    pub fn new(
        values: Vec<Real>,
        weights: Vec<u64>,
        numeric_version: NumericVersion,
    ) -> Result<Self, String> {
        if values.len() != weights.len() {
            return Err("Statistics frequencies must match the observations".into());
        }
        let count = weights
            .iter()
            .try_fold(0u64, |total, &weight| total.checked_add(weight))
            .ok_or("Statistics frequency total exceeds its integer range")?;
        Ok(Self {
            values,
            weights,
            count,
            numeric_version,
        })
    }
    pub fn summary(&self, cancel: &AtomicBool) -> Result<Summary, String> {
        check(cancel)?;
        if self.count < 2 {
            return Err("Insufficient statistics data".into());
        }
        // Statistics accumulators retain the exact stored observations through
        // intermediate arithmetic. Returned results are twelve-digit decimal
        // objects: original15751 MeanX*3 is 6.99999999999 for a mean of7/3.
        let mut sorted = self
            .values
            .iter()
            .copied()
            .map(|value| match value.origin() {
                ppl_math::RealOrigin::LegacyBcd | ppl_math::RealOrigin::Decimal => {
                    Real::parse_versioned(&value.to_string(), NumericVersion::Current15751)
                        .expect("a stored real has valid calculator source")
                }
                _ => value,
            })
            .zip(self.weights.iter().copied())
            .filter(|&(_, weight)| weight > 0)
            .collect::<Vec<_>>();
        sorted.sort_by(|a, b| a.0.cmp_value(b.0));
        let mut sum = Real::ZERO;
        let mut squares = Real::ZERO;
        for &(x, weight) in &sorted {
            check(cancel)?;
            let w = Real::from_u64(weight);
            sum = sum
                .add(x.mul(w).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            squares = squares
                .add(x.mul(x).and_then(|x| x.mul(w)).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        }
        let n = Real::from_u64(self.count);
        let mean = sum.div(n).map_err(|e| e.to_string())?;
        let mut deviations = Real::ZERO;
        for &(x, weight) in &sorted {
            check(cancel)?;
            let difference = x.sub(mean).map_err(|e| e.to_string())?;
            deviations = deviations
                .add(
                    difference
                        .mul(difference)
                        .and_then(|square| square.mul(Real::from_u64(weight)))
                        .map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
        }
        let rank = |rank: u64| -> Real {
            let mut total = 0u64;
            for &(value, weight) in &sorted {
                total += weight;
                if rank < total {
                    return value;
                }
            }
            unreachable!("validated frequency total bounds every requested rank")
        };
        let median = |start: u64, length: u64| -> Result<Real, String> {
            rank(start + (length - 1) / 2)
                .add(rank(start + length / 2))
                .and_then(|sum| sum.div(Real::from_i64(2)))
                .map_err(|e| e.to_string())
        };
        let sample_variance = deviations
            .div(Real::from_u64(self.count - 1))
            .map_err(|e| e.to_string())?;
        let population_variance = deviations.div(n).map_err(|e| e.to_string())?;
        let variables = [
            ("NbItem", n),
            ("MinVal", sorted[0].0),
            ("Q₁", median(0, self.count / 2)?),
            ("MedVal", median(0, self.count)?),
            ("Q₃", median(self.count.div_ceil(2), self.count / 2)?),
            ("MaxVal", sorted.last().unwrap().0),
            ("ΣX", sum),
            ("ΣX2", squares),
            ("MeanX", mean),
            ("sX", sample_variance.sqrt().map_err(|e| e.to_string())?),
            ("σX", population_variance.sqrt().map_err(|e| e.to_string())?),
            (
                "serrX",
                sample_variance
                    .div(n)
                    .and_then(Real::sqrt)
                    .map_err(|e| e.to_string())?,
            ),
            ("ssX", deviations),
        ];
        Ok(Summary {
            variables: variables
                .into_iter()
                .map(|(name, value)| {
                    (
                        name.into(),
                        if self.numeric_version == NumericVersion::Current15751 {
                            value.decimal()
                        } else {
                            Real::parse(&value.to_string())
                                .expect("a calculated real has valid calculator source")
                        },
                    )
                })
                .collect(),
        })
    }
}
impl Summary {
    pub fn portable(&self) -> BTreeMap<String, PortableValue> {
        self.variables
            .iter()
            .map(|(name, &value)| (name.clone(), PortableValue::Real(value)))
            .collect()
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
                summary.variables[name].format(Default::default()),
                text.as_str().unwrap(),
                "{name}"
            );
        }
        assert_eq!(summary.portable().len(), 13);
        assert!(
            summary
                .variables
                .values()
                .all(|value| value.origin() == ppl_math::RealOrigin::Decimal)
        );
        assert_eq!(
            summary.variables["MeanX"]
                .mul(parse("3"))
                .unwrap()
                .format(Default::default()),
            "6.99999999999"
        );
        assert_eq!(
            summary.variables["NbItem"]
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
            frequency_source: Some("1+1".into()),
            ..Default::default()
        };
        let data = Data::evaluate(&analysis, &snapshot, Arc::new(AtomicBool::new(false))).unwrap();
        assert_eq!(data.values[0].ppl_source(), "1.234567890123456789e18");
        assert_eq!(data.values[1].format(Default::default()), "1E400");
        assert_eq!(data.weights, vec![2, 2]);
        assert_eq!(session.snapshot().globals, snapshot.globals);
        let cancelled = Arc::new(AtomicBool::new(true));
        assert!(Data::evaluate(&analysis, &snapshot, cancelled).is_err());
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
        assert_eq!(summary.variables["MinVal"], parse("1"));
        assert_eq!(summary.variables["MeanX"], parse("2"));
        for source in ["-1", "1.5"] {
            assert!(weight(&Value::Real(parse(source))).is_err());
        }
        assert!(
            Data::new(vec![parse("1")], vec![1], NumericVersion::Current15751)
                .unwrap()
                .summary(&AtomicBool::new(false))
                .is_err()
        );
        assert!(Data::new(vec![parse("1")], vec![1, 2], NumericVersion::Current15751).is_err());
    }
}
