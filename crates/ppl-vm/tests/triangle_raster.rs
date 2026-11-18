//! Independently captured G1 scalar and vertex triangle controls.
mod common;
use common::Run;
use ppl_vm::Value;
use sha2::{Digest, Sha256};

fn projection(value: &Value) -> serde_json::Value {
    match value {
        Value::Real(value) => serde_json::from_str(&value.to_string()).unwrap(),
        Value::List(values) => values.iter().map(projection).collect(),
        value => panic!("Unexpected triangle result {value:?}"),
    }
}

#[test]
fn scalar_and_vertex_triangles_match_original_frames() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-triangle-raster-controls.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let source = case["source"].as_str().unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(source)),
            case["source_sha256"]
        );
        let name = case["name"].as_str().unwrap();
        let mut run = Run::new(name, source);
        let value = run.call(case["entry"].as_str().unwrap(), &[]).unwrap();
        assert_eq!(projection(&value), case["expected"], "{name}");
    }
}

#[test]
fn extreme_coordinates_only_rasterize_the_visible_grob() {
    let mut run = Run::new(
        "bounded",
        "EXPORT BOUNDED() BEGIN DIMGROB_P(G1,3,3,#FFFFFFh); \
         TRIANGLE_P(G1,-1E200,-1E200,1E200,0,0,1E200,#FE8040h); \
         RETURN {GROBW_P(G1),GROBH_P(G1)}; END;",
    );
    let result = run.call("BOUNDED", &[]).unwrap();
    assert_eq!(projection(&result), serde_json::json!([3, 3]));
}
