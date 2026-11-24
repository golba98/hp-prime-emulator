//! Complete original-G1 target buffers and error admission, not just whether a
//! graphics screen opens. No graphics responses are supplied by the test host.
mod common;
use common::Run;
use sha2::{Digest, Sha256};

fn projection(value: &ppl_vm::Value) -> serde_json::Value {
    match value {
        ppl_vm::Value::Real(value) => serde_json::json!(value.to_i64().unwrap()),
        ppl_vm::Value::List(values) => values.iter().map(projection).collect(),
        other => panic!("Unexpected original result {other:?}"),
    }
}

#[test]
fn original_blit_admission_and_every_target_pixel_match() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-blit-boundary-controls.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let source = case["source"].as_str().unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(source.as_bytes())),
            case["source_sha256"]
        );
        if case["accepted"] == false {
            let (_, diagnostics) = ppl_compiler::compile_source(
                &ppl_syntax::SourceFile::new("original rejected call", source),
                &Default::default(),
            );
            assert!(diagnostics.errors().any(|d| d.code == "wrong-arg-count"));
            continue;
        }
        let entry = source
            .split_whitespace()
            .nth(1)
            .unwrap()
            .split('(')
            .next()
            .unwrap();
        let mut run = Run::new(case["name"].as_str().unwrap(), source);
        assert_eq!(
            projection(&run.call(entry, &[]).unwrap()),
            case["expected"],
            "{}",
            case["name"]
        );
    }
}
