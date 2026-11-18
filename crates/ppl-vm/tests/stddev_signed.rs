//! Numeric signed STDDEV controls from the original virtual G1.
use ppl_compiler::{CompileOptions, compile_source};
use ppl_math::NumericVersion;
use ppl_syntax::SourceFile;
use ppl_vm::{HeadlessHost, Limits, Machine, Settings, Value};
use sha2::{Digest, Sha256};
use std::rc::Rc;

fn project(value: &Value) -> serde_json::Value {
    match value {
        Value::Real(value) => serde_json::from_str(&value.to_string()).unwrap(),
        Value::Str(value) => serde_json::json!(value.as_ref()),
        Value::List(values) => values.iter().map(project).collect(),
        value => panic!("Unexpected signed STDDEV result {value:?}"),
    }
}

#[test]
fn signed_list_results_match_the_original_in_both_numeric_modes() {
    verify(include_str!(
        "../../../tests/fixtures/native-stddev-signed-controls.json"
    ));
}

#[test]
fn signed_vector_results_match_the_original_in_both_numeric_modes() {
    verify(include_str!(
        "../../../tests/fixtures/native-stddev-signed-vector-controls.json"
    ));
}

#[test]
fn signed_matrix_results_match_the_original_in_both_numeric_modes() {
    verify(include_str!(
        "../../../tests/fixtures/native-stddev-signed-matrix-controls.json"
    ));
}

fn verify(fixture: &str) {
    let fixture: serde_json::Value = serde_json::from_str(fixture).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(source)),
            case["source_sha256"]
        );
        let (program, diagnostics) =
            compile_source(&SourceFile::new(name, source), &CompileOptions::default());
        assert!(!diagnostics.has_errors(), "{name}: {diagnostics:?}");
        let program = Rc::new(program.unwrap());
        for numeric_version in [NumericVersion::Legacy15157, NumericVersion::Current15751] {
            let mut machine = Machine::new(
                program.clone(),
                Box::new(HeadlessHost::default()),
                Settings {
                    numeric_version,
                    ..Settings::default()
                },
                Limits::default(),
            );
            let value = machine
                .call_function(case["entry"].as_str().unwrap(), vec![])
                .unwrap();
            assert_eq!(
                project(&value),
                case["expected"],
                "{name} {numeric_version:?}"
            );
        }
    }
}

#[test]
fn signed_vector_specialization_respects_source_shape_and_legacy_calls() {
    let mut session = ppl_vm::SessionContext::default();
    for source in [
        "stddev([1,3],[-.5,1.5])",
        "stddev([0+1,3],{-.5,1.5})",
        "stddev({1,3},[-.5,1.5])",
        "STDDEV([1,3],{-.5,1.5})",
    ] {
        let error = session
            .evaluate_home(source, Box::new(HeadlessHost::new()), Limits::default())
            .result
            .unwrap_err();
        assert_eq!(error.kind, ppl_vm::ErrorKind::BadArgValue, "{source}");
    }
}

#[test]
fn signed_matrix_specialization_preserves_literal_and_call_boundaries() {
    let mut session = ppl_vm::SessionContext::default();
    for source in [
        "stddev([[1,1],[3,3]],[[-.5,1.5],[1.5,-.5]])",
        "stddev([[0+1,1],[3,3]],[[-.5,1.5]])",
        "stddev([[0+1,1],[3,3]],{-.5,1.5,1.5,-.5})",
        "STDDEV([[0+1,1],[3,3]],[[-.5,1.5],[1.5,-.5]])",
    ] {
        let error = session
            .evaluate_home(source, Box::new(HeadlessHost::new()), Limits::default())
            .result
            .unwrap_err();
        assert_eq!(error.kind, ppl_vm::ErrorKind::BadArgValue, "{source}");
    }
}
