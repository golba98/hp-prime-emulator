//! Worksheet hypothesis tests and confidence intervals use the Inference app's
//! distributions, with the worksheet's own field names and critical estimates.
use super::*;
use crate::ide::native_inference::{
    Alternative, InferenceMode, InferenceRequest, MeanSummary, ProportionSample,
};

pub(super) const NAMES: [&str; 12] = [
    "HypZ1mean",
    "HypZ2mean",
    "HypZ1prop",
    "HypZ2prop",
    "HypT1mean",
    "HypT2mean",
    "ConfZ1mean",
    "ConfZ2mean",
    "ConfZ1prop",
    "ConfZ2prop",
    "ConfT1mean",
    "ConfT2mean",
];
struct Field {
    code: &'static str,
    label: String,
    value: f64,
}

fn count(n: f64) -> Result<u64, String> {
    if n.fract() != 0. || !(0. ..=9_007_199_254_740_992.).contains(&n) {
        Err("Invalid inference sample count".into())
    } else {
        Ok(n as u64)
    }
}
fn summary(mean: f64, sd: f64, n: f64) -> Result<MeanSummary, String> {
    Ok(MeanSummary {
        mean,
        sd,
        count: count(n)?,
    })
}
fn proportion(x: f64, n: f64) -> Result<ProportionSample, String> {
    Ok(ProportionSample {
        successes: count(x)?,
        total: count(n)?,
    })
}

pub(super) fn table(
    name: &str,
    values: &[f64],
    config: &str,
    cancel: &AtomicBool,
) -> Result<Vec<Vec<Value>>, String> {
    let test = name.starts_with("Hyp");
    let family = &name[if test { 3 } else { 4 }..];
    let mode = if test {
        let [.., alpha, alternative] = values else {
            return Err("Missing inference significance and alternative".into());
        };
        let alternative = match *alternative {
            1. => Alternative::Less,
            2. => Alternative::Greater,
            3. => Alternative::TwoSided,
            _ => return Err("Invalid hypothesis alternative".into()),
        };
        let null_value = match (family, values) {
            ("Z1mean" | "Z1prop", [_, _, null, ..]) | ("T1mean", [_, _, _, null, ..]) => *null,
            _ => 0.,
        };
        InferenceMode::Test {
            null_value,
            alpha: *alpha,
            alternative,
        }
    } else {
        InferenceMode::Interval {
            confidence: *values.last().ok_or("Missing confidence")?,
        }
    };
    let args = &values[..values.len() - if test { 2 } else { 1 }];
    let request = match (family, args) {
        ("Z1mean", [m, n, _, s]) if test => InferenceRequest::MeanZ {
            mean: *m,
            sigma: *s,
            count: count(*n)?,
            mode,
        },
        ("Z1mean", [m, n, s]) if !test => InferenceRequest::MeanZ {
            mean: *m,
            sigma: *s,
            count: count(*n)?,
            mode,
        },
        ("Z2mean", [m1, m2, n1, n2, s1, s2]) => InferenceRequest::TwoMeansZ {
            first: summary(*m1, *s1, *n1)?,
            second: summary(*m2, *s2, *n2)?,
            mode,
        },
        ("T1mean", [m, s, n, ..]) if args.len() == 3 + usize::from(test) => {
            InferenceRequest::MeanT {
                sample: summary(*m, *s, *n)?,
                mode,
            }
        }
        ("T2mean", [m1, m2, s1, s2, n1, n2, pool]) if matches!(pool, 0. | 1.) => {
            InferenceRequest::TwoMeansT {
                first: summary(*m1, *s1, *n1)?,
                second: summary(*m2, *s2, *n2)?,
                pooled: *pool == 1.,
                mode,
            }
        }
        ("Z1prop", [x, n, ..]) if args.len() == 2 + usize::from(test) => {
            InferenceRequest::OneProportion {
                sample: proportion(*x, *n)?,
                mode,
            }
        }
        ("Z2prop", [x1, x2, n1, n2]) => InferenceRequest::TwoProportions {
            first: proportion(*x1, *n1)?,
            second: proportion(*x2, *n2)?,
            mode,
        },
        _ => return Err(format!("Invalid {name} inputs")),
    };
    let result = request.calculate_with_cancel(cancel)?;
    let estimate = result.estimate.ok_or("Missing inference estimate")?;
    let crit = result.critical_values[0];
    let student = family.starts_with('T');
    let prop = family.ends_with("prop");
    let two = family.contains('2');
    let glyph = if prop { "\u{e001}" } else { "\u{e000}" };
    let sample = if two {
        format!("Δ{glyph}")
    } else {
        glyph.into()
    };
    let mut fields = Vec::new();
    let mut push = |code, label: String, value| fields.push(Field { code, label, value });
    if test {
        push(
            "acc",
            "Reject/Fail Reject".into(),
            f64::from(!result.reject_null.ok_or("Missing test decision")?),
        );
        push(
            if student { "tT" } else { "tZ" },
            if student { "Test T" } else { "Test Z" }.into(),
            result.statistic.ok_or("Missing test statistic")?,
        );
        push(
            if prop { "tP" } else { "tM" },
            format!("Test {sample}"),
            estimate,
        );
        if family == "Z2prop" {
            push("p1", format!("{glyph}₁"), args[0] / args[2]);
            push("p2", format!("{glyph}₂"), args[1] / args[3]);
        }
        push(
            "prob",
            if student { "T" } else { "P" }.into(),
            result.p_value.ok_or("Missing test probability")?,
        );
        if student {
            push("df", "DF".into(), result.degrees_of_freedom[0]);
        }
        push(
            if student { "cT" } else { "cZ" },
            if student { "Crit. T" } else { "Crit. Z" }.into(),
            crit,
        );
        let center = if two {
            estimate
        } else if family == "T1mean" {
            args[3]
        } else {
            args[2]
        };
        // The worksheet reports critical sample estimates with the observed
        // variance even when its hypothesis statistic uses the null variance.
        let se = match family {
            "Z1prop" => (estimate * (1. - estimate) / args[1]).sqrt(),
            "Z2prop" => {
                let (p, q) = (args[0] / args[2], args[1] / args[3]);
                (p * (1. - p) / args[2] + q * (1. - q) / args[3]).sqrt()
            }
            _ => result.standard_error.ok_or("Missing test standard error")?,
        };
        let critical = |q: f64| center + q * se;
        let upper = *result.critical_values.last().unwrap_or(&crit);
        if student {
            push("cX", format!("Crit. {sample}"), critical(crit));
        } else {
            push(
                if prop { "cp1" } else { "cx1" },
                format!("Critical {sample}1"),
                critical(crit),
            );
            push(
                if prop { "cp2" } else { "cx2" },
                format!("Critical {sample}2"),
                critical(upper),
            );
        }
    } else {
        let [lo, hi] = result.interval.ok_or("Missing confidence interval")?;
        if student {
            push("df", "DF".into(), result.degrees_of_freedom[0]);
        }
        push(
            if student { "T" } else { "Z" },
            if student { "Crit. T" } else { "Z" }.into(),
            crit,
        );
        push("low", "Lower".into(), lo);
        push("up", "Upper".into(), hi);
        if prop {
            if two {
                push("p1", format!("{glyph}₁"), args[0] / args[2]);
                push("p2", format!("{glyph}₂"), args[1] / args[3]);
            } else {
                push("p1", format!("{glyph}₁"), estimate);
            }
        }
    }
    let defaults = config.is_empty();
    let spec: Vec<_> = config.split_whitespace().collect();
    let headings = defaults || spec.contains(&"h");
    let title = defaults || spec.contains(&"H");
    let mut rows = vec![];
    if title {
        let mut row = vec![];
        if headings {
            row.push(Value::str(name));
        }
        row.push(Value::str("A"));
        rows.push(row);
    }
    let ordered = if defaults {
        fields.iter().collect::<Vec<_>>()
    } else {
        spec.iter()
            .filter(|&&s| s != "H" && s != "h")
            .map(|&s| {
                fields
                    .iter()
                    .find(|f| f.code == s)
                    .ok_or_else(|| format!("Invalid {name} field {s}"))
            })
            .collect::<Result<_, _>>()?
    };
    for f in ordered {
        let mut row = vec![];
        if headings {
            row.push(Value::str(f.label.as_str()));
        }
        row.push(Value::Real(
            Real::from_f64(f.value).map_err(|e| e.to_string())?,
        ));
        rows.push(row);
    }
    if rows.is_empty() && headings {
        rows.push(vec![Value::str(name)]);
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worksheet_inference_matches_virtual_g1_examples() {
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../../tests/fixtures/native-sheet-inference.json"
        ))
        .unwrap();
        for name in NAMES {
            let case = &cases[name];
            let values: Vec<_> = case["inputs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n.as_f64().unwrap())
                .collect();
            let rows = table(name, &values, "", &AtomicBool::new(false)).unwrap();
            assert_eq!(rows[0][0].expect_str(name).unwrap().as_ref(), name);
            assert_eq!(rows[0][1].expect_str(name).unwrap().as_ref(), "A");
            for (row, expected) in rows[1..].iter().zip(case["rows"].as_array().unwrap()) {
                assert_eq!(
                    row[0].expect_str(name).unwrap().as_ref(),
                    expected[0].as_str().unwrap(),
                    "{name}"
                );
                let value = row[1].as_real().unwrap().to_f64();
                let wanted = expected[1].as_f64().unwrap();
                assert!(
                    (value - wanted).abs() < 1e-8,
                    "{name} {:?}: {value} vs {wanted}",
                    row[0]
                );
            }
            assert_eq!(
                rows.len(),
                if name == "HypZ2prop" {
                    10
                } else {
                    case["rows"].as_array().unwrap().len() + 1
                }
            );
        }
    }

    #[test]
    fn inference_reports_reject_invalid_parameters_and_unknown_fields() {
        let run = |name, args: &[f64], config| table(name, args, config, &AtomicBool::new(false));
        for args in [[51., 50., 0.95], [1., 2.5, 0.95], [1., 2., 1.5]] {
            assert!(run("ConfZ1prop", &args, "").is_err());
        }
        assert!(run("ConfT1mean", &[1., 1., 10., 0.95], "txl").is_err());
        let rows = run(
            "HypZ1mean",
            &[0.461368, 50., 0.5, 0.2887, 0.05, 3.],
            "cx2 cx1",
        )
        .unwrap();
        assert_eq!(rows.len(), 2);
        for (row, wanted) in rows.iter().zip([0.580022086818, 0.419977913182]) {
            assert_eq!(row.len(), 1);
            assert!((row[0].as_real().unwrap().to_f64() - wanted).abs() < 1e-10);
        }
    }
}
