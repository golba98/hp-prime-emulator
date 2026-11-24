//! CI uses Native; separate real-HLE runs supply firmware verification evidence.
use ppl_compat::differential::{LoadedScenario, compare, run_native};
use ppl_engine::Cancellation;
use std::path::Path;
#[test]
fn m18_native_linkage_and_ascii_fixtures_execute_without_hardware() {
    for name in ["ascii_imaginary", "exported_variables"] {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../tests/differential/m18/{name}.toml"));
        let scenario = LoadedScenario::load(&fixture).unwrap();
        let run = run_native(&scenario, &Cancellation::default());
        let report = compare(&scenario, vec![run]);
        assert!(report.passed, "{name}: {:?}", report.mismatches);
    }
}
