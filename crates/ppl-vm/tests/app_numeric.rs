//! Real VM Numeric Setup execution. AppHost acknowledges presentation only;
//! these tests do not stand in for actual calculator GUI validation.
mod common;
use ppl_math::Real;
use ppl_vm::host::{CalculatorView, Dialog, DialogResult, Touch};
use ppl_vm::{
    HeadlessHost, Host, Limits, PortableValue, RuntimeError, SessionContext, SourceIdentity, Value,
};
use std::rc::Rc;

struct AppHost(HeadlessHost);
impl Host for AppHost {
    fn calculator_view(&mut self, request: CalculatorView) -> Result<bool, RuntimeError> {
        Ok(
            matches!(request, CalculatorView::StartApp(ref name) if ["Function", "Parametric", "Polar", "Sequence"].contains(&name.as_str())),
        )
    }
    fn print(&mut self, text: &str) {
        self.0.print(text)
    }
    fn poll_key(&mut self) -> Result<Option<i64>, RuntimeError> {
        self.0.poll_key()
    }
    fn is_key_down(&mut self, key: i64) -> Result<bool, RuntimeError> {
        self.0.is_key_down(key)
    }
    fn wait_key(&mut self, timeout: Option<f64>) -> Result<Option<i64>, RuntimeError> {
        self.0.wait_key(timeout)
    }
    fn sleep(&mut self, seconds: f64) -> Result<(), RuntimeError> {
        self.0.sleep(seconds)
    }
    fn touch(&mut self) -> Result<Option<Touch>, RuntimeError> {
        self.0.touch()
    }
    fn dialog(
        &mut self,
        dialog: &Dialog,
        screen: &ppl_graphics::Grob,
    ) -> Result<DialogResult, RuntimeError> {
        self.0.dialog(dialog, screen)
    }
    fn now_ms(&mut self) -> f64 {
        self.0.now_ms()
    }
    fn local_time(&mut self) -> (i64, i64, i64, i64, i64, f64) {
        self.0.local_time()
    }
}
fn home(context: &mut SessionContext, source: &str) -> Value {
    context
        .evaluate_home(source, Box::new(HeadlessHost::new()), Limits::default())
        .result
        .unwrap()
}
fn assert_value(actual: Value, expected: Value) {
    assert!(actual.equals(&expected), "{actual:?} != {expected:?}");
}
fn canonical(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| if c == '−' { '-' } else { c })
        .collect()
}
#[test]
fn original_firmware_defaults_admission_and_app_switches_execute_in_the_vm() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-app-numeric-controls.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let source = case["source"].as_str().unwrap();
        let file = ppl_syntax::SourceFile::new("numeric-controls", source);
        let (compiled, diagnostics) = ppl_compiler::compile_source(&file, &Default::default());
        assert!(!diagnostics.has_errors(), "{case}: {diagnostics:?}");
        let result = SessionContext::default()
            .run(
                SourceIdentity::new("numeric-controls", source),
                Rc::new(compiled.unwrap()),
                Box::new(AppHost(HeadlessHost::new())),
                Limits::default(),
                false,
                |m| m.call_function("NUMSET", vec![]),
            )
            .result
            .unwrap();
        assert_eq!(
            canonical(result.expect_str("original control").unwrap().as_ref()),
            canonical(case["expected"].as_str().unwrap()),
            "{}",
            case["case"]
        );
    }
}
#[test]
fn catalog_vm_snapshot_and_decimal_range_share_one_numeric_state() {
    let mut session = SessionContext::default();
    session
        .set_variable(
            "NumStart",
            PortableValue::Real(Real::parse("1E400").unwrap()),
        )
        .unwrap();
    home(&mut session, "NumStep:=1E-400");
    home(&mut session, "NumZoom:=1E400");
    home(&mut session, "NumType:=1");
    home(&mut session, "NumIndep:={2,4,6,1E-400}");
    let snapshot = session.snapshot();
    assert_eq!(
        snapshot.app_numeric_settings(),
        session.app_numeric_settings()
    );
    assert_eq!(
        snapshot.app_numeric_settings().step,
        Real::parse("1E-400").unwrap()
    );
    assert_value(
        home(&mut session, "NumStart"),
        Value::Real(Real::parse("1E400").unwrap()),
    );
    assert!(
        !session.memory_json()["globals"]
            .as_object()
            .unwrap()
            .contains_key("NumStart")
    );
    assert!(!session.delete_variable("NumStart"));
    session.active_app = "Parametric".into();
    assert_value(home(&mut session, "NumStart"), Value::Real(Real::ZERO));
    assert_value(
        home(&mut session, "NumStep"),
        Value::Real(Real::parse("0.1").unwrap()),
    );
    session.active_app = "Sequence".into();
    assert_value(home(&mut session, "NumStart"), Value::Real(Real::ONE));
    assert_value(home(&mut session, "NumStep"), Value::Real(Real::ONE));
    assert_value(home(&mut session, "NumIndep"), Value::list(vec![]));
    session.active_app = "Function".into();
    assert_eq!(
        session.app_numeric_settings(),
        snapshot.app_numeric_settings()
    );
}
#[test]
fn numeric_memory_validates_transactionally_and_migrates_real_legacy_globals() {
    let mut session = SessionContext::default();
    home(&mut session, "NumStart:=5");
    home(&mut session, "NumStep:=2");
    home(&mut session, "NumIndep:={2,4,6}");
    let image = session.memory_json();
    let mut restored = SessionContext::default();
    restored.restore_memory(&image).unwrap();
    assert_value(home(&mut restored, "NumStart+NumStep"), Value::from_i64(7));
    for (key, value) in [
        ("step", serde_json::json!("0")),
        ("zoom", serde_json::json!("1")),
        ("kind", serde_json::json!(0.5)),
        ("start", serde_json::json!("bad")),
        ("independent", serde_json::json!(["2", "not a number"])),
    ] {
        let before = restored.memory_json();
        let mut bad = image.clone();
        bad["app_numeric_settings"]["Function"][key] = value;
        assert!(restored.restore_memory(&bad).is_err());
        assert_eq!(restored.memory_json(), before);
    }
    let mut old = image.clone();
    old["version"] = 3.into();
    old.as_object_mut().unwrap().remove("app_numeric_settings");
    old["globals"]["NumStart"] =
        ppl_vm::session::encode_value(&PortableValue::Real(Real::from_i64(9)));
    old["globals"]["NumStep"] =
        ppl_vm::session::encode_value(&PortableValue::Real(Real::from_i64(3)));
    restored.restore_memory(&old).unwrap();
    assert_value(home(&mut restored, "NumStart+NumStep"), Value::from_i64(12));
    assert!(restored.memory_json()["globals"].get("NumStart").is_none());
}
#[test]
fn numeric_settings_survive_a_separate_process() {
    let mut session = SessionContext::default();
    home(&mut session, "NumStart:=1E400");
    home(&mut session, "NumStep:=1E-400");
    home(&mut session, "NumIndep:={2,4,6,1E-400}");
    session.active_app = "Polar".into();
    home(&mut session, "NumStart:=-7");
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("memory.json"),
        serde_json::to_vec(&session.memory_json()).unwrap(),
    )
    .unwrap();
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "numeric_memory_child", "--nocapture"])
        .env("PPL_NUMERIC_MEMORY_CHILD", root.path())
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "{} {}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );
    assert!(String::from_utf8_lossy(&child.stdout).contains("numeric-memory-resumed"));
}
#[test]
fn numeric_memory_child() {
    let Some(root) = std::env::var_os("PPL_NUMERIC_MEMORY_CHILD") else {
        return;
    };
    let mut session = SessionContext::default();
    session
        .restore_memory(
            &serde_json::from_slice(
                &std::fs::read(std::path::Path::new(&root).join("memory.json")).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    assert_value(home(&mut session, "NumStart"), Value::from_i64(-7));
    session.active_app = "Function".into();
    assert_value(
        home(&mut session, "NumStart"),
        Value::Real(Real::parse("1E400").unwrap()),
    );
    assert_value(
        home(&mut session, "NumStep"),
        Value::Real(Real::parse("1E-400").unwrap()),
    );
    assert_value(
        home(&mut session, "NumIndep"),
        Value::list(vec![
            Value::from_i64(2),
            Value::from_i64(4),
            Value::from_i64(6),
            Value::Real(Real::parse("1E-400").unwrap()),
        ]),
    );
    println!("numeric-memory-resumed");
}
