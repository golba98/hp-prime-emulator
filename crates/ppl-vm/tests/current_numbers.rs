use ppl_math::{NumericVersion, Real, RealOrigin};
use ppl_vm::{HeadlessHost, Limits, PortableValue, SessionContext, Settings, Value};

fn session() -> SessionContext {
    SessionContext::new(
        Settings {
            numeric_version: NumericVersion::Current15751,
            ..Default::default()
        },
        1,
    )
}
fn home(context: &mut SessionContext, source: &str) -> Value {
    context
        .evaluate_home(source, Box::new(HeadlessHost::default()), Limits::default())
        .result
        .unwrap()
}
fn real(value: Value) -> Real {
    let Value::Real(value) = value else {
        panic!("expected native real")
    };
    value
}

#[test]
fn app_arithmetic_updates_live_without_rewriting_home_settings_or_other_apps() {
    let mut context = session();
    context
        .store_program(
            "DYNAMIC",
            "EXPORT DYNAMIC() BEGIN RETURN EXPR(\"SIN(90)\"); END;".into(),
        )
        .unwrap();
    home(&mut context, "HAngle:=0");
    home(&mut context, "AAngle:=2");
    assert_eq!(real(home(&mut context, "SIN(90)")), Real::ONE);
    assert_eq!(real(home(&mut context, "DYNAMIC()")), Real::ONE);
    assert_eq!(real(home(&mut context, "HAngle")), Real::ZERO);
    home(&mut context, "HComplex:=1");
    home(&mut context, "AComplex:=1");
    assert!(
        context
            .evaluate_home(
                "SQRT(-1)",
                Box::new(HeadlessHost::default()),
                Limits::default()
            )
            .result
            .is_err()
    );
    assert!(matches!(
        home(&mut context, "SQRT((-1,0))"),
        Value::Complex(_)
    ));
    home(&mut context, "AComplex:=2");
    assert!(matches!(home(&mut context, "SQRT(-1)"), Value::Complex(_)));
    assert_eq!(real(home(&mut context, "HComplex")), Real::ONE);
    context.active_app = "Parametric".into();
    assert_eq!(real(home(&mut context, "AAngle")), Real::ZERO);
    assert_ne!(real(home(&mut context, "SIN(90)")), Real::ONE);
    assert!(matches!(home(&mut context, "SQRT(-1)"), Value::Complex(_)));
}

#[test]
fn source_digits_survive_home_expr_and_late_linked_catalog_execution() {
    let mut context = session();
    context
        .store_program(
            "DIGITS",
            "EXPORT DIGITS(x) BEGIN RETURN {1234567890123456789+x,EXPR(\"1/3\")*3}; END;".into(),
        )
        .unwrap();
    let value = home(&mut context, "DIGITS(1)");
    let list = value.expect_list("digits").unwrap();
    assert_eq!(
        list[0].as_real().unwrap().to_i128(),
        Some(1_234_567_890_123_456_790)
    );
    assert_eq!(
        list[1].as_real().unwrap().origin(),
        RealOrigin::Rational {
            numerator: 1,
            denominator: 1
        }
    );
    assert_eq!(
        real(home(&mut context, "EXPR(\"1234567890123456789\")")).to_i128(),
        Some(1_234_567_890_123_456_789)
    );
}
#[test]
fn recall_source_retains_rationals_pi_and_approximate_values_without_giac() {
    let mut context = session();
    for source in [
        "1234567890123456789",
        "1/3",
        "-π/3",
        "SQRT(2)",
        "1E400",
        "1E-400",
        "approx(1/3)",
    ] {
        let value = real(home(&mut context, source));
        let recalled = real(home(&mut context, &value.ppl_source()));
        assert_eq!(value, recalled, "{source}");
        assert_eq!(
            value.origin(),
            recalled.origin(),
            "{source}: {}",
            value.ppl_source()
        );
    }
}
#[test]
fn exact_scalar_collection_ans_and_app_numeric_memory_roundtrip_transactionally() {
    let mut context = session();
    home(&mut context, "A:=1234567890123456789");
    home(&mut context, "L1:={1/3,π/2,SQRT(2),A}");
    home(&mut context, "NumStart:=A");
    home(&mut context, "NumStep:=1/3");
    home(&mut context, "NumIndep:={A,1/3,π/2}");
    let image = context.memory_json();
    let mut restored = SessionContext::default();
    restored.restore_memory(&image).unwrap();
    assert_eq!(
        restored.settings.numeric_version,
        NumericVersion::Current15751
    );
    assert_eq!(
        real(home(&mut restored, "A")).to_i128(),
        Some(1_234_567_890_123_456_789)
    );
    assert_eq!(
        real(home(&mut restored, "L1(1)*3")).origin(),
        RealOrigin::Rational {
            numerator: 1,
            denominator: 1
        }
    );
    assert_eq!(
        restored.app_numeric_settings(),
        context.app_numeric_settings()
    );
    for source in ["A", "L1", "NumStart", "NumStep", "NumIndep"] {
        assert_eq!(
            PortableValue::from(&home(&mut restored, source)),
            PortableValue::from(&home(&mut context, source))
        );
    }
    let before = restored.memory_json();
    for (field, value) in [
        ("denominator", serde_json::json!("0")),
        (
            "numerator",
            serde_json::json!("999999999999999999999999999"),
        ),
        ("negative", serde_json::json!(true)),
        ("kind", serde_json::json!("future")),
    ] {
        let mut bad = image.clone();
        bad["globals"]["A"]["origin"][field] = value;
        assert!(restored.restore_memory(&bad).is_err(), "{field}");
        assert_eq!(restored.memory_json(), before);
    }
}
#[test]
fn exact_memory_survives_an_independent_process() {
    let mut context = session();
    home(&mut context, "A:=1234567890123456789");
    home(&mut context, "L1:={1/3,π/2,SQRT(2)}");
    home(&mut context, "NumStep:=1/3");
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("memory.json"),
        serde_json::to_vec(&context.memory_json()).unwrap(),
    )
    .unwrap();
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "current_memory_child", "--nocapture"])
        .env("PPL_CURRENT_MEMORY_CHILD", root.path())
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "{} {}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );
    assert!(String::from_utf8_lossy(&child.stdout).contains("current-memory-resumed"));
}
#[test]
fn current_memory_child() {
    let Some(root) = std::env::var_os("PPL_CURRENT_MEMORY_CHILD") else {
        return;
    };
    let mut context = SessionContext::default();
    context
        .restore_memory(
            &serde_json::from_slice(
                &std::fs::read(std::path::Path::new(&root).join("memory.json")).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(
        context.settings.numeric_version,
        NumericVersion::Current15751
    );
    assert_eq!(
        real(home(&mut context, "A+1")).to_i128(),
        Some(1_234_567_890_123_456_790)
    );
    assert_eq!(
        real(home(&mut context, "L1(1)*3")).origin(),
        RealOrigin::Rational {
            numerator: 1,
            denominator: 1
        }
    );
    assert_eq!(
        real(home(&mut context, "L1(2)")).origin(),
        RealOrigin::PiMultiple {
            numerator: 1,
            denominator: 2
        }
    );
    assert_eq!(
        real(home(&mut context, "L1(3)")).origin(),
        RealOrigin::Decimal
    );
    assert_eq!(
        real(home(&mut context, "NumStep")).origin(),
        RealOrigin::Rational {
            numerator: 1,
            denominator: 3
        }
    );
    println!("current-memory-resumed");
}

#[cfg(feature = "giac-integration")]
#[test]
fn genuine_giac_keeps_exact_home_inputs_and_shared_integer_updates() {
    let mut context = session();
    home(&mut context, "A:=1234567890123456789");
    assert_eq!(
        real(home(&mut context, "CAS(\"A+1\")")).to_i128(),
        Some(1_234_567_890_123_456_790)
    );
    home(&mut context, "A:=1/3");
    let rational = home(&mut context, "CAS(\"A\")");
    assert_eq!(rational.type_code(), 8);
    assert_eq!(
        ppl_vm::cas::to_cas(&rational),
        Some(ppl_cas::CasValue::Rational {
            numerator: "1".into(),
            denominator: "3".into()
        })
    );
    home(&mut context, "CAS(\"A:=1234567890123456789\")");
    assert_eq!(
        real(home(&mut context, "A+1")).to_i128(),
        Some(1_234_567_890_123_456_790)
    );
    let decimal = real(home(&mut context, "approx(CAS(\"1/3\"))"));
    assert_eq!(decimal.origin(), RealOrigin::Decimal);
    assert_eq!(
        decimal
            .mul(Real::parse_versioned("3", NumericVersion::Current15751).unwrap())
            .unwrap()
            .to_string(),
        "0.999999999999"
    );
}
