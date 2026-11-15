mod common;
use ppl_math::{NumFormat, Real};
use ppl_vm::{AngleMode, HeadlessHost, Limits, PortableValue, SessionContext, Value};

fn home(session: &mut SessionContext, text: &str) -> Value {
    session
        .evaluate_home(text, Box::new(HeadlessHost::new()), Limits::default())
        .result
        .unwrap()
}
fn code(session: &mut SessionContext, name: &str) -> i64 {
    home(session, name).expect_i64("app setting test").unwrap()
}

#[test]
fn ppl_app_overrides_are_independent_and_never_change_home_settings() {
    let mut session = SessionContext::default();
    home(&mut session, "HAngle:=1");
    home(&mut session, "HFormat:=1");
    home(&mut session, "HDigits:=2");
    home(&mut session, "HComplex:=1");
    home(&mut session, "AAngle:=1");
    home(&mut session, "AFormat:=4");
    home(&mut session, "ADigits:=7");
    home(&mut session, "AComplex:=2");
    assert_eq!(session.effective_app_settings().angle, AngleMode::Radians);
    assert_eq!(
        session.effective_app_settings().number_format,
        NumFormat::Engineering(7)
    );
    assert!(!session.effective_app_settings().complex_results);
    assert_eq!(code(&mut session, "HAngle"), 1);
    assert_eq!(code(&mut session, "HDigits"), 2);
    assert_eq!(code(&mut session, "HComplex"), 1);
    assert_eq!(session.settings.number_format, NumFormat::Fixed(2));
    session.active_app = "Parametric".into();
    assert_eq!(code(&mut session, "AAngle"), 0);
    assert_eq!(code(&mut session, "AComplex"), 0);
    assert_eq!(code(&mut session, "AFormat"), 0);
    assert_eq!(session.effective_app_settings().angle, AngleMode::Degrees);
    assert_eq!(
        session.effective_app_settings().number_format,
        NumFormat::Fixed(2)
    );
    assert!(session.effective_app_settings().complex_results);
    home(&mut session, "AAngle:=3");
    home(&mut session, "AFormat:=3");
    home(&mut session, "ADigits:=5");
    assert_eq!(session.effective_app_settings().angle, AngleMode::Gradians);
    assert_eq!(
        session.effective_app_settings().number_format,
        NumFormat::Scientific(5)
    );
    session.active_app = "Function".into();
    assert_eq!(code(&mut session, "AAngle"), 1);
    assert_eq!(code(&mut session, "ADigits"), 7);
    assert_eq!(
        session.snapshot().effective_app_settings().number_format,
        NumFormat::Engineering(7)
    );
}

#[test]
fn exact_app_format_codes_and_system_inheritance() {
    let mut session = SessionContext::default();
    home(&mut session, "HDigits:=2");
    home(&mut session, "HFormat:=1");
    home(&mut session, "ADigits:=6");
    for (app_code, expected) in [
        (0, NumFormat::Fixed(2)),
        (1, NumFormat::Standard),
        (2, NumFormat::Fixed(6)),
        (3, NumFormat::Scientific(6)),
        (4, NumFormat::Engineering(6)),
        (5, NumFormat::Floating(6)),
        (6, NumFormat::Rounded(6)),
    ] {
        home(&mut session, &format!("AFormat:={app_code}"));
        assert_eq!(session.effective_app_settings().number_format, expected);
    }
    home(&mut session, "AAngle:=0");
    home(&mut session, "HAngle:=2");
    assert_eq!(session.effective_app_settings().angle, AngleMode::Gradians);
    for (app_code, expected) in [
        (1, AngleMode::Radians),
        (2, AngleMode::Degrees),
        (3, AngleMode::Gradians),
    ] {
        home(&mut session, &format!("AAngle:={app_code}"));
        assert_eq!(session.effective_app_settings().angle, expected);
    }
}

#[test]
fn invalid_ppl_catalog_and_persisted_settings_do_not_change_overrides() {
    let mut session = SessionContext::default();
    // Rejected writes do not even create a default per-app entry.
    assert!(
        session
            .set_variable("AAngle", PortableValue::Real(Real::from_i64(99)))
            .is_err()
    );
    assert!(session.app_settings.is_empty());
    home(&mut session, "AAngle:=2");
    for (name, invalid) in [
        ("AAngle", -1),
        ("AAngle", 4),
        ("AComplex", 3),
        ("AFormat", -1),
        ("AFormat", 7),
        ("ADigits", -1),
        ("ADigits", 12),
    ] {
        let before = session.app_settings();
        assert!(
            session
                .evaluate_home(
                    &format!("{name}:={invalid}"),
                    Box::new(HeadlessHost::new()),
                    Limits::default()
                )
                .result
                .is_err()
        );
        assert_eq!(session.app_settings(), before);
        assert!(
            session
                .set_variable(name, PortableValue::Real(Real::from_i64(invalid)))
                .is_err()
        );
        assert_eq!(session.app_settings(), before);
        assert!(!session.delete_variable(name));
    }
    let before = session.memory_json();
    let mut broken = before.clone();
    broken["app_settings"]["Function"]["angle"] = serde_json::json!(99);
    assert!(session.restore_memory(&broken).is_err());
    assert_eq!(session.memory_json(), before);
    let mut previous = before;
    previous.as_object_mut().unwrap().remove("app_settings");
    session.restore_memory(&previous).unwrap();
    assert!(session.app_settings.is_empty());
    assert_eq!(code(&mut session, "AAngle"), 0);
}

#[test]
fn app_settings_survive_a_fresh_process() {
    const KEY: &str = "PPL_APP_SETTINGS_RESTART_FIXTURE";
    if let Some(path) = std::env::var_os(KEY) {
        let data = std::fs::read(path).unwrap();
        let mut restored = SessionContext::default();
        restored
            .restore_memory(&serde_json::from_slice(&data).unwrap())
            .unwrap();
        assert_eq!(restored.active_app, "Parametric");
        assert_eq!(code(&mut restored, "AAngle"), 3);
        assert_eq!(
            restored.effective_app_settings().number_format,
            NumFormat::Scientific(5)
        );
        restored.active_app = "Function".into();
        assert_eq!(code(&mut restored, "AAngle"), 2);
        assert_eq!(code(&mut restored, "AComplex"), 1);
        assert_eq!(
            restored.effective_app_settings().number_format,
            NumFormat::Fixed(8)
        );
        assert_eq!(restored.settings.angle, AngleMode::Radians);
        return;
    }
    let mut session = SessionContext::default();
    for text in ["AAngle:=2", "AComplex:=1", "AFormat:=2", "ADigits:=8"] {
        home(&mut session, text);
    }
    session.active_app = "Parametric".into();
    for text in ["AAngle:=3", "AFormat:=3", "ADigits:=5"] {
        home(&mut session, text);
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "ppl-app-settings-{}-{nonce}.json",
        std::process::id()
    ));
    std::fs::write(&path, serde_json::to_vec(&session.memory_json()).unwrap()).unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "app_settings_survive_a_fresh_process",
            "--nocapture",
        ])
        .env(KEY, &path)
        .status()
        .unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(status.success());
}

#[test]
fn build15751_app_formats_and_digit_boundaries_match_original_programs() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-app-format-controls.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let mut run = common::Run::new(
            case["name"].as_str().unwrap(),
            case["native_transport_source"].as_str().unwrap(),
        );
        let result = run.call(case["entry"].as_str().unwrap(), &[]);
        if case["success"].as_bool().unwrap() {
            let actual = result.unwrap().display(Default::default());
            let expected = case["firmware_message"]
                .as_str()
                .unwrap()
                .strip_prefix("G1 firmware returned ")
                .unwrap()
                .replace(' ', "");
            assert_eq!(actual, expected, "{}", case["name"]);
        } else {
            assert!(result.is_err(), "{}", case["name"]);
        }
    }
}

#[test]
fn floating_rounded_and_boundary_digits_compute_and_restore_without_home_changes() {
    let mut session = SessionContext::default();
    home(&mut session, "HFormat:=1");
    home(&mut session, "HDigits:=2");
    for (format, digits, expected) in [
        (2, 0, NumFormat::Fixed(0)),
        (2, 11, NumFormat::Fixed(11)),
        (5, 0, NumFormat::Floating(0)),
        (5, 11, NumFormat::Floating(11)),
        (6, 0, NumFormat::Rounded(0)),
        (6, 11, NumFormat::Rounded(11)),
    ] {
        home(&mut session, &format!("AFormat:={format}"));
        home(&mut session, &format!("ADigits:={digits}"));
        assert_eq!(session.effective_app_settings().number_format, expected);
        let mut restored = SessionContext::default();
        restored.restore_memory(&session.memory_json()).unwrap();
        assert_eq!(restored.effective_app_settings().number_format, expected);
        assert_eq!(restored.settings.number_format, NumFormat::Fixed(2));
        let before = restored.memory_json();
        for (field, bad) in [
            ("format", -1),
            ("format", 7),
            ("digits", -1),
            ("digits", 12),
        ] {
            let mut malformed = before.clone();
            malformed["app_settings"]["Function"][field] = serde_json::json!(bad);
            assert!(restored.restore_memory(&malformed).is_err());
            assert_eq!(restored.memory_json(), before);
        }
    }
}
