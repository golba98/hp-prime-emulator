#![cfg(feature = "giac-integration")]
//! Original-firmware approximation controls through the genuine algebra helper.
mod common;
use common::Run;
use ppl_vm::Value;
fn projection(value: &Value) -> serde_json::Value {
    match value {
        Value::Real(value) => serde_json::from_str(&value.to_string()).unwrap(),
        Value::List(values) => values.iter().map(projection).collect(),
        value => panic!("Unexpected approximation projection {value:?}"),
    }
}
fn text(units: &serde_json::Value) -> String {
    String::from_utf16(
        &units
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_u64().unwrap() as u16)
            .collect::<Vec<_>>(),
    )
    .unwrap()
}
#[test]
fn approximation_recurses_preserving_strings_and_vector_shape() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-approx-controls.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let mut run = Run::new(
            case["name"].as_str().unwrap(),
            case["source"].as_str().unwrap(),
        );
        let value = run.call(case["entry"].as_str().unwrap(), &[]).unwrap();
        let actual = projection(&value);
        let expected = &case["expected"];
        if case["name"] != "approx3" {
            assert_eq!(&actual, expected, "{}", case["name"]);
        } else {
            assert_eq!(actual[0], expected[0]);
            assert_eq!(actual[2], expected[2]);
            // The numeric function agrees; Giac and HP print symbolic products
            // in different factor order. Check the complete returned expression
            // without claiming those different strings are pixel/text parity.
            for index in [1, 3] {
                let as_expression = |units: &serde_json::Value| {
                    let source = text(units);
                    if source.starts_with('{') {
                        format!("list[{}]", &source[1..source.len() - 1])
                    } else {
                        source
                    }
                };
                let source = format!(
                    "normal(({})-({}))",
                    as_expression(&actual[index]),
                    as_expression(&expected[index])
                );
                let query = format!("CAS({})", serde_json::to_string(&source).unwrap());
                let mut session = ppl_vm::SessionContext::default();
                let difference = session
                    .evaluate_home(
                        &query,
                        Box::new(ppl_vm::HeadlessHost::new()),
                        ppl_vm::Limits::default(),
                    )
                    .result
                    .unwrap();
                let zero = match &difference {
                    Value::List(values) => values
                        .iter()
                        .all(|v| v.as_real().is_some_and(|v| v.is_zero())),
                    value => value.as_real().is_some_and(|v| v.is_zero()),
                };
                assert!(zero, "{query}: {difference:?}");
            }
        }
    }
}

#[test]
fn home_numeric_projection_keeps_the_independent_cas_answer_exact() {
    let mut session = ppl_vm::SessionContext::default();
    let value = session
        .evaluate_home(
            "CAS(\"[1/3,sqrt(2)]\")",
            Box::new(ppl_vm::HeadlessHost::new()),
            ppl_vm::Limits::default(),
        )
        .result
        .unwrap();
    let Value::Matrix(matrix) = value else {
        panic!("expected numeric Home vector")
    };
    assert!(matrix.vector);
    assert_eq!(
        matrix.data[0].as_real().unwrap().to_string(),
        "0.333333333333"
    );
    let Some(ppl_cas::CasValue::Vector(answer)) = &session.cas.last_value else {
        panic!("CAS answer lost vector type")
    };
    assert!(
        matches!(&answer[0], ppl_cas::CasValue::Rational { numerator, denominator } if numerator == "1" && denominator == "3")
    );
    assert!(matches!(&answer[1], ppl_cas::CasValue::Symbolic(source) if source == "sqrt(2)"));
}
