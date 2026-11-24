//! Original programs run through the production compiler/native engine, not a
//! replacement evaluator or canned response.
use ppl_engine::{Cancellation, Operation, native, source::Source};
use ppl_math::{NumericVersion, Real};
use serde_json::Value;

#[test]
fn exact_json_arguments_roundtrip_nested_values_and_reject_lossy_legacy_import() {
    let settings = ppl_vm::Settings {
        numeric_version: NumericVersion::Current15751,
        ..Default::default()
    };
    let source = r#"[{"type":"list","value":[{"type":"rational","negative":true,"numerator":"1","denominator":"3"},{"type":"pi_multiple","negative":false,"numerator":"2","denominator":"3"},{"type":"real","value":"1234567890123456789"}]}]"#;
    let values = ppl_engine::arguments::json_args(source, &settings).unwrap();
    let encoded = serde_json::json!([ppl_engine::arguments::encode(&values[0])]);
    let restored = ppl_engine::arguments::json_args(&encoded.to_string(), &settings).unwrap();
    assert_eq!(
        encoded,
        serde_json::json!([ppl_engine::arguments::encode(&restored[0])])
    );
    assert!(ppl_engine::arguments::json_args(source, &Default::default()).is_err());
    for (field, value) in [
        ("denominator", serde_json::json!("0")),
        ("numerator", serde_json::json!("18446744073709551616")),
        ("numerator", serde_json::json!("+1")),
        ("negative", serde_json::json!(0)),
        ("presentation", serde_json::json!("future")),
    ] {
        let mut invalid: Value = serde_json::from_str(source).unwrap();
        invalid[0]["value"][0][field] = value;
        assert!(
            ppl_engine::arguments::json_args(&invalid.to_string(), &settings).is_err(),
            "{field}"
        );
    }
}

fn compare(actual: &Value, expected: &Value, path: &str) {
    assert_eq!(
        actual["type"], expected["type"],
        "{path}: {actual} != {expected}"
    );
    match expected["type"].as_str().unwrap() {
        "list" => {
            let (a, b) = (
                actual["value"].as_array().unwrap(),
                expected["value"].as_array().unwrap(),
            );
            assert_eq!(a.len(), b.len(), "{path}");
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                compare(a, b, &format!("{path}[{i}]"));
            }
        }
        "real" => {
            // Versioned parsing retains every exact digit, so this cannot hide
            // a rounded19-digit return behind a12-digit comparison.
            let parse = |value: &Value| {
                Real::parse_versioned(
                    value["value"].as_str().unwrap(),
                    NumericVersion::Current15751,
                )
                .unwrap()
            };
            assert_eq!(
                parse(actual),
                parse(expected),
                "{path}: {actual} != {expected}"
            );
        }
        "complex" => {
            for component in ["real", "imaginary"] {
                let parse =
                    |value: &Value| Real::parse(value[component].as_str().unwrap()).unwrap();
                assert_eq!(
                    parse(actual),
                    parse(expected),
                    "{path}.{component}: {actual} != {expected}"
                );
            }
        }
        _ => assert_eq!(actual, expected, "{path}"),
    }
}
#[test]
fn original_current_number_programs_produce_the_actual_firmware_results() {
    let controls: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-current-number-controls.json"
    ))
    .unwrap();
    for case in controls["cases"].as_array().unwrap() {
        let source = Source::text(
            case["source"].as_str().unwrap(),
            case["name"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(
            source.identity.source_sha256,
            case["source_sha256"].as_str().unwrap()
        );
        let versions = if case.get("legacy_15157_evidence").is_some() {
            vec![NumericVersion::Legacy15157, NumericVersion::Current15751]
        } else {
            vec![NumericVersion::Current15751]
        };
        for version in versions {
            let report = native::execute(
                source.clone(),
                Operation::Run,
                native::Options {
                    entry: None,
                    modules: Default::default(),
                    args: "[]".into(),
                    json_arguments: true,
                    host: Default::default(),
                    settings: ppl_vm::Settings {
                        numeric_version: version,
                        ..Default::default()
                    },
                    limits: Default::default(),
                    seed: 0,
                    png: None,
                    deny_warnings: false,
                    dump_bytecode: false,
                },
                &Cancellation::default(),
            );
            assert_eq!(report.exit_code, 0, "{}: {report:?}", case["name"]);
            compare(
                report.return_value.as_ref().unwrap(),
                &case["expected"],
                case["name"].as_str().unwrap(),
            );
        }
    }
}
