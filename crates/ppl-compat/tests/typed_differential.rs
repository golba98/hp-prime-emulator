//! These are comparator controls, not physical or live-HLE observations.
use ppl_compat::differential::*;
use ppl_engine::Cancellation;
use serde_json::json;
use std::path::Path;
#[test]
fn typed_scenario_runs_in_native_and_mismatches_remain_precise() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/differential/m17/typed_values.toml");
    let s = LoadedScenario::load(&fixture).unwrap();
    let native = run_native(&s, &Cancellation::default());
    let baseline = compare(&s, vec![native.clone()]);
    assert!(baseline.passed, "{:?}", baseline.mismatches);
    for (path, changed) in [
        ("/return_value/value/1/vector", json!(true)),
        ("/return_value/value/1/rows", json!(1)),
        ("/return_value/value/1/value/0/type", json!("integer")),
        ("/return_value/value/3/imaginary", json!("-3")),
        ("/return_value/value/4/bits", json!(16)),
        ("/return_value/value/4/literal", json!("#255d")),
        ("/return_value/value/0/value/0/value/0/value", json!("2")),
    ] {
        let mut controlled = native.clone();
        controlled.backend = BackendId::FirmwareHle;
        let data = &mut controlled
            .observations
            .iter_mut()
            .find(|o| o.step == "nested_types")
            .unwrap()
            .data;
        *data.pointer_mut(path).expect(path) = changed;
        let compared = compare(&s, vec![native.clone(), controlled]);
        assert!(!compared.passed, "masked mismatch at {path}");
        assert!(
            compared.mismatches.iter().any(|m| m.path == path),
            "{:?}",
            compared.mismatches
        );
    }
}
