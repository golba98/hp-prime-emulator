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
            programs: BTreeMap::new(),
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
    let s = scenario(
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
    let mut spec = s.scenario;
    spec.initial.setup_entry = Some("SETUP".into());
    spec.initial.settings.insert("HAngle".into(), 1);
    let s = LoadedScenario::from_spec(spec, Path::new("."), "fixture-hash".into()).unwrap();
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
    s.schema_version = 3;
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

#[test]
fn altered_loaded_source_or_initial_state_cannot_reuse_old_identity() {
    let mut s = counter();
    s.source.push_str("\n// altered");
    assert_eq!(
        run_native(&s, &Cancellation::default()).status,
        RunStatus::InfrastructureError
    );
    let mut s = counter();
    s.scenario.initial.seed = 999;
    assert_eq!(
        run_native(&s, &Cancellation::default()).status,
        RunStatus::InfrastructureError
    );
    let mut s = counter();
    s.scenario.expect.clear();
    assert!(s.verify_identity().is_err());
}
#[test]
fn empty_capture_is_not_pixel_parity() {
    let image = Image {
        width: 0,
        height: 240,
        pixels: vec![],
    };
    assert!(compare_images(&image, &image, false).is_err());
}

#[test]
fn floating_point_arguments_require_explicit_decimal_strings() {
    let mut spec = counter().scenario;
    spec.steps = vec![Step::Run {
        id: "float".into(),
        entry: "INC".into(),
        args: vec![json!(1.23456789012345)],
    }];
    spec.expect.clear();
    assert!(
        LoadedScenario::from_spec(spec, Path::new("."), "hash".into())
            .unwrap_err()
            .contains("precision")
    );
}

#[test]
fn version_two_runs_independent_retained_modules_and_authored_probes() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/differential/m16/stateful.toml");
    let s = LoadedScenario::load(&fixture).unwrap();
    let compared = compare(&s, vec![run_native(&s, &Cancellation::default())]);
    assert!(
        compared.passed,
        "{}",
        format!(
            "{:?}; backend reasons: {:?}",
            compared.mismatches,
            compared.runs.iter().map(|r| &r.reason).collect::<Vec<_>>()
        )
    );
}
#[test]
fn version_two_binds_all_sources_and_rejects_unknown_programs() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/differential/m16/stateful.toml");
    let mut s = LoadedScenario::load(&fixture).unwrap();
    s.sources.insert("ZZT_M16OTHER".into(), "changed".into());
    assert!(s.verify_identity().is_err());
    let mut spec = s.scenario.clone();
    spec.steps.push(Step::Import {
        id: "bad".into(),
        program: "UNKNOWN".into(),
    });
    assert!(LoadedScenario::from_spec(spec, Path::new("."), "test".into()).is_err());
}
#[test]
fn real_firmware_stateful_scenario() {
    let (Some(firmware), Some(init), Some(runner)) = (
        std::env::var_os("PRIME_TEST_FIRMWARE"),
        std::env::var_os("PRIME_TEST_INIT_DATA"),
        std::env::var_os("PRIME_TEST_RUNNER"),
    ) else {
        eprintln!("skipping local-firmware scenario: PRIME_TEST_FIRMWARE/INIT_DATA/RUNNER unset");
        return;
    };
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/differential/m16/stateful.toml");
    let mut spec = LoadedScenario::load(&fixture).unwrap().scenario;
    let edited = spec.programs["ZZT_M16COUNT"]
        .source
        .as_ref()
        .unwrap()
        .replace("counter:=0", "counter:=40");
    spec.steps.push(Step::Edit {
        id: "edit".into(),
        program: "ZZT_M16COUNT".into(),
        source: edited,
    });
    spec.steps.push(Step::RunProgram {
        id: "edited_run".into(),
        program: "ZZT_M16COUNT".into(),
        entry: "M16INC".into(),
        args: vec![],
    });
    spec.expect.push(expected(
        "edited_run",
        "/return_value",
        json!({"type":"real","value":"41"}),
    ));
    let s = LoadedScenario::from_spec(spec, Path::new("."), "real-edit-fixture".into()).unwrap();
    let options = ppl_engine::firmware_options::Options {
        runner: Some(runner.into()),
        firmware: Some(firmware.into()),
        init_data: Some(init.into()),
        os_rom: std::env::var_os("PRIME_TEST_OS_ROM").map(Into::into),
        ..Default::default()
    };
    let compared = compare(
        &s,
        vec![
            run_native(&s, &Cancellation::default()),
            run_firmware(&s, &options, &Cancellation::default()),
        ],
    );
    if let Some(out) = std::env::var_os("PRIME_M16_EVIDENCE") {
        std::fs::write(out, serde_json::to_vec_pretty(&compared).unwrap()).unwrap();
    }
    assert!(
        compared.passed,
        "{}",
        format!(
            "{:?}; backend reasons: {:?}",
            compared.mismatches,
            compared.runs.iter().map(|r| &r.reason).collect::<Vec<_>>()
        )
    );
}

#[test]
fn version_two_edit_resets_one_module_and_binds_revision() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/differential/m16/stateful.toml");
    let mut spec = LoadedScenario::load(&fixture).unwrap().scenario;
    let edited = spec.programs["ZZT_M16COUNT"]
        .source
        .as_ref()
        .unwrap()
        .replace("counter:=0", "counter:=40");
    spec.steps.push(Step::Edit {
        id: "edit".into(),
        program: "ZZT_M16COUNT".into(),
        source: edited,
    });
    spec.steps.push(Step::RunProgram {
        id: "edited_run".into(),
        program: "ZZT_M16COUNT".into(),
        entry: "M16INC".into(),
        args: vec![],
    });
    spec.expect.push(expected(
        "edited_run",
        "/return_value",
        json!({"type":"real","value":"41"}),
    ));
    let s = LoadedScenario::from_spec(spec, Path::new("."), "edited-fixture".into()).unwrap();
    assert!(s.source_manifest().contains_key("ZZT_M16COUNT@edit"));
    let result = execute(&s, &[&Native], &Cancellation::default());
    assert!(result.passed, "{:#?}", result.mismatches);
}

#[test]
fn real_firmware_lifecycle_scenario() {
    let (Some(firmware), Some(init), Some(runner)) = (
        std::env::var_os("PRIME_TEST_FIRMWARE"),
        std::env::var_os("PRIME_TEST_INIT_DATA"),
        std::env::var_os("PRIME_TEST_RUNNER"),
    ) else {
        eprintln!("skipping local-firmware scenario: PRIME_TEST_FIRMWARE/INIT_DATA/RUNNER unset");
        return;
    };
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/differential/m16/firmware_lifecycle.toml");
    let s = LoadedScenario::load(&fixture).unwrap();
    let options = ppl_engine::firmware_options::Options {
        runner: Some(runner.into()),
        firmware: Some(firmware.into()),
        init_data: Some(init.into()),
        os_rom: std::env::var_os("PRIME_TEST_OS_ROM").map(Into::into),
        ..Default::default()
    };
    let compared = compare(
        &s,
        vec![run_firmware(&s, &options, &Cancellation::default())],
    );
    if let Some(out) = std::env::var_os("PRIME_M16_EVIDENCE") {
        std::fs::write(out, serde_json::to_vec_pretty(&compared).unwrap()).unwrap();
    }
    assert!(
        compared.passed,
        "{}",
        format!(
            "{:?}; backend reasons: {:?}",
            compared.mismatches,
            compared.runs.iter().map(|r| &r.reason).collect::<Vec<_>>()
        )
    );
}

#[test]
fn real_firmware_async_input_scenario() {
    let (Some(firmware), Some(init), Some(runner)) = (
        std::env::var_os("PRIME_TEST_FIRMWARE"),
        std::env::var_os("PRIME_TEST_INIT_DATA"),
        std::env::var_os("PRIME_TEST_RUNNER"),
    ) else {
        eprintln!("skipping local-firmware scenario: PRIME_TEST_FIRMWARE/INIT_DATA/RUNNER unset");
        return;
    };
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/differential/m16/async_input.toml");
    let s = LoadedScenario::load(&fixture).unwrap();
    let options = ppl_engine::firmware_options::Options {
        runner: Some(runner.into()),
        firmware: Some(firmware.into()),
        init_data: Some(init.into()),
        os_rom: std::env::var_os("PRIME_TEST_OS_ROM").map(Into::into),
        ..Default::default()
    };
    let compared = compare(
        &s,
        vec![run_firmware(&s, &options, &Cancellation::default())],
    );
    if let Some(out) = std::env::var_os("PRIME_M16_EVIDENCE") {
        std::fs::write(out, serde_json::to_vec_pretty(&compared).unwrap()).unwrap();
    }
    assert!(
        compared.passed,
        "{}",
        format!(
            "{:?}; backend reasons: {:?}",
            compared.mismatches,
            compared.runs.iter().map(|r| &r.reason).collect::<Vec<_>>()
        )
    );
}
