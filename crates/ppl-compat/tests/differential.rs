use ppl_compat::differential::*;
use ppl_engine::Cancellation;
use serde_json::json;
use std::{collections::BTreeMap, path::Path};

fn scenario(source: &str, steps: Vec<Step>, expect: Vec<Expectation>) -> LoadedScenario {
    LoadedScenario::from_spec(
        Scenario {
            schema_version: 1,
            id: "test".into(),
            program: Program {
                source: Some(source.into()),
                file: None,
            },
            initial: InitialState::default(),
            steps,
            expect,
            constraints: BTreeMap::new(),
        },
        Path::new("."),
        "fixture-hash".into(),
    )
    .unwrap()
}
fn call(id: &str, entry: &str) -> Step {
    Step::Run {
        id: id.into(),
        entry: entry.into(),
        args: vec![],
    }
}
fn expected(step: &str, path: &str, value: serde_json::Value) -> Expectation {
    Expectation {
        step: step.into(),
        path: path.into(),
        value,
    }
}
fn counter() -> LoadedScenario {
    scenario(
        "LOCAL COUNTER:=0; EXPORT INC() BEGIN COUNTER:=COUNTER+1; RETURN COUNTER; END;",
        vec![
            call("first", "INC"),
            call("second", "INC"),
            Step::Observe {
                id: "variables".into(),
                names: vec!["COUNTER".into()],
            },
        ],
        vec![
            expected("first", "/return_value", json!({"type":"real","value":"1"})),
            expected(
                "second",
                "/return_value",
                json!({"type":"real","value":"2"}),
            ),
            expected("variables", "/variables/COUNTER/value", json!("2")),
        ],
    )
}
#[test]
fn native_calls_retain_real_program_globals() {
    let s = counter();
    let r = execute(&s, &[&Native], &Cancellation::default());
    assert!(r.passed, "{:?}", r.mismatches);
    assert_eq!(r.assertions, 3);
}
#[test]
fn setup_and_settings_are_applied_before_steps() {
    let mut s = scenario(
        "EXPORT SETUP() BEGIN A:=9; END; EXPORT MAIN() BEGIN RETURN A+HAngle; END;",
        vec![
            call("run", "MAIN"),
            Step::Observe {
                id: "state".into(),
                names: vec!["A".into(), "HAngle".into()],
            },
        ],
        vec![
            expected("run", "/return_value/value", json!("10")),
            expected("state", "/variables/A/value", json!("9")),
        ],
    );
    s.scenario.initial.setup_entry = Some("SETUP".into());
    s.scenario.initial.settings.insert("HAngle".into(), 1);
    assert!(execute(&s, &[&Native], &Cancellation::default()).passed);
}
#[test]
fn exact_values_never_equate_strings_reals_or_list_order() {
    let s = scenario(
        "EXPORT MAIN() BEGIN RETURN {1,2}; END;",
        vec![call("run", "MAIN")],
        vec![],
    );
    let a = run_native(&s, &Cancellation::default());
    let mut b = a.clone();
    b.backend = BackendId::FirmwareHle;
    b.observations[0].data["return_value"] =
        json!({"type":"list","value":[{"type":"real","value":"2"},{"type":"real","value":"1"}]});
    let r = compare(&s, vec![a, b]);
    assert!(!r.passed);
    assert!(
        r.mismatches
            .iter()
            .any(|m| m.path == "/return_value/value/0/value")
    );
}
#[test]
fn unrelated_errors_never_match() {
    let s = scenario(
        "EXPORT MAIN() BEGIN RETURN 1/0; END;",
        vec![call("run", "MAIN")],
        vec![expected("run", "/status", json!("runtime_error"))],
    );
    let a = run_native(&s, &Cancellation::default());
    let mut b = a.clone();
    b.backend = BackendId::FirmwareHle;
    b.observations[0].data["error"]["message"] = json!("a different error");
    let r = compare(&s, vec![a, b]);
    assert!(!r.passed);
    assert!(r.mismatches.iter().any(|m| m.path == "/error/message"));
}
#[test]
fn metadata_differences_are_not_semantic_but_source_hash_is_binding() {
    let s = counter();
    let a = run_native(&s, &Cancellation::default());
    let mut b = a.clone();
    b.backend = BackendId::FirmwareHle;
    b.raw_reports.push(json!({"wall_seconds":999}));
    b.provenance.origin = "local-firmware".into();
    assert!(compare(&s, vec![a.clone(), b.clone()]).passed);
    b.provenance.source_sha256 = "wrong".into();
    assert!(!compare(&s, vec![a, b]).passed);
}
#[test]
fn unavailable_and_zero_comparisons_cannot_pass() {
    let s = counter();
    assert!(
        !execute(
            &s,
            &[&Unavailable {
                backend: BackendId::FullSystem,
                reason: "not implemented".into()
            }],
            &Cancellation::default()
        )
        .passed
    );
    let empty = scenario(
        "EXPORT MAIN() BEGIN RETURN 1; END;",
        vec![call("run", "MAIN")],
        vec![],
    );
    assert!(!execute(&empty, &[&Native], &Cancellation::default()).passed);
}
#[test]
fn missing_duplicate_or_unobserved_fields_fail() {
    let s = counter();
    let mut r = run_native(&s, &Cancellation::default());
    r.observations.pop();
    assert!(!compare(&s, vec![r]).passed);
    let mut r = run_native(&s, &Cancellation::default());
    r.observations.push(r.observations[0].clone());
    assert!(!compare(&s, vec![r]).passed);
    let s = scenario(
        "EXPORT MAIN() BEGIN END;",
        vec![Step::Observe {
            id: "probe".into(),
            names: vec!["UNDECLARED".into()],
        }],
        vec![expected("probe", "/status", json!("unobserved"))],
    );
    assert!(!execute(&s, &[&Native], &Cancellation::default()).passed);
}
#[test]
fn firmware_stateful_scenarios_are_rejected_before_boot() {
    let s = counter();
    let r = run_firmware(&s, &Default::default(), &Cancellation::default());
    assert_eq!(r.status, RunStatus::Unsupported);
    assert!(r.observations.is_empty());
}
#[test]
fn physical_and_power_actions_are_not_native_noops() {
    let s = scenario(
        "EXPORT MAIN() BEGIN END;",
        vec![Step::PowerOff { id: "off".into() }],
        vec![expected("off", "/status", json!("success"))],
    );
    assert_eq!(
        run_native(&s, &Cancellation::default()).status,
        RunStatus::Unsupported
    );
}
#[test]
fn shape_pixels_and_encoding_are_explicit() {
    let a = Image {
        width: 2,
        height: 1,
        pixels: vec![0, 0xff0000],
    };
    let b = Image {
        width: 2,
        height: 1,
        pixels: vec![1, 0xff0000],
    };
    let d = compare_images(&a, &b, false).unwrap();
    assert_eq!(d.changed_pixels, 1);
    assert_eq!(d.first, Some([0, 0]));
    assert!(compare_images(&a, &b, true).unwrap().equal);
    assert!(
        !compare_images(
            &a,
            &Image {
                width: 1,
                height: 2,
                pixels: b.pixels.clone()
            },
            false
        )
        .unwrap()
        .dimensions_equal
    );
    assert!(
        compare_images(
            &a,
            &Image {
                width: 9,
                height: 1,
                pixels: vec![]
            },
            false
        )
        .is_err()
    );
}
#[test]
fn captures_measure_actual_drawing() {
    let s = scenario(
        "EXPORT DRAW() BEGIN RECT_P(0,0,319,239,RGB(255,0,0)); END;",
        vec![call("draw", "DRAW"), Step::Capture { id: "image".into() }],
        vec![
            expected("image", "/image/width", json!(320)),
            expected("image", "/image/pixels/0", json!(0xff0000)),
        ],
    );
    assert!(execute(&s, &[&Native], &Cancellation::default()).passed);
}
#[test]
fn fixture_loader_rejects_ambiguous_sources_and_bad_settings() {
    let mut s = counter().scenario;
    s.program.file = Some("elsewhere.ppl".into());
    assert!(LoadedScenario::from_spec(s, Path::new("."), "hash".into()).is_err());
    let mut s = counter().scenario;
    s.initial.settings.insert("NotASetting".into(), 1);
    assert!(LoadedScenario::from_spec(s, Path::new("."), "hash".into()).is_err());
    let mut s = counter().scenario;
    s.schema_version = 2;
    assert!(LoadedScenario::from_spec(s, Path::new("."), "hash".into()).is_err());
}
#[test]
fn committed_scenarios_execute_their_declared_expectations() {
    for name in ["arithmetic", "state", "graphics", "rational_difference"] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../tests/differential/{name}.toml"));
        let s = LoadedScenario::load(&path).unwrap();
        let result = execute(&s, &[&Native], &Cancellation::default());
        assert!(result.passed, "{name}: {:?}", result.mismatches);
    }
}
