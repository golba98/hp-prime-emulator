//! Matches the separately captured genuine firmware boundary controls.
use ppl_compat::differential::{LoadedScenario, compare, run_native};
use ppl_engine::Cancellation;
use std::path::Path;

#[test]
fn nested_complex_and_full_width_integer_boundaries() {
    let scenario = LoadedScenario::load(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/differential/m19/typed_boundaries.toml"),
    )
    .unwrap();
    let report = compare(
        &scenario,
        vec![run_native(&scenario, &Cancellation::default())],
    );
    assert!(report.passed, "{:?}", report.mismatches);
    assert_eq!(report.assertions, 2);
}
