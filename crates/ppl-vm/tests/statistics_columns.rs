//! Genuine column semantics plus the session boundary used by desktop tables.
use ppl_vm::{HeadlessHost, Host, Limits, PortableValue, SessionContext, SourceIdentity, Value};
use std::rc::Rc;

fn json(value: &Value) -> serde_json::Value {
    match value {
        Value::List(values) => values.iter().map(json).collect(),
        Value::Str(value) => serde_json::json!(value.as_ref()),
        Value::Real(value) => serde_json::from_str(&value.to_string()).unwrap(),
        other => panic!("unexpected column control: {other:?}"),
    }
}
// Only app acknowledgement is scripted; all column storage and calculations
// execute on the production VM. Actual desktop bindings have separate GUI tests.
struct ColumnHost(HeadlessHost);
impl Host for ColumnHost {
    fn calculator_view(
        &mut self,
        request: ppl_vm::host::CalculatorView,
    ) -> Result<bool, ppl_vm::RuntimeError> {
        assert!(matches!(request, ppl_vm::host::CalculatorView::StartApp(_)));
        Ok(true)
    }
    fn print(&mut self, text: &str) {
        self.0.print(text)
    }
    fn poll_key(&mut self) -> Result<Option<i64>, ppl_vm::RuntimeError> {
        self.0.poll_key()
    }
    fn is_key_down(&mut self, key: i64) -> Result<bool, ppl_vm::RuntimeError> {
        self.0.is_key_down(key)
    }
    fn wait_key(&mut self, timeout: Option<f64>) -> Result<Option<i64>, ppl_vm::RuntimeError> {
        self.0.wait_key(timeout)
    }
    fn sleep(&mut self, seconds: f64) -> Result<(), ppl_vm::RuntimeError> {
        self.0.sleep(seconds)
    }
    fn touch(&mut self) -> Result<Option<ppl_vm::host::Touch>, ppl_vm::RuntimeError> {
        self.0.touch()
    }
    fn dialog(
        &mut self,
        request: &ppl_vm::host::Dialog,
        screen: &ppl_graphics::Grob,
    ) -> Result<ppl_vm::host::DialogResult, ppl_vm::RuntimeError> {
        self.0.dialog(request, screen)
    }
    fn now_ms(&mut self) -> f64 {
        self.0.now_ms()
    }
    fn local_time(&mut self) -> (i64, i64, i64, i64, i64, f64) {
        self.0.local_time()
    }
}
fn execute(session: &mut SessionContext, name: &str, source: &str, entry: &str) -> Value {
    let (program, diagnostics) = ppl_compiler::compile_source(
        &ppl_syntax::SourceFile::new(name, source),
        &Default::default(),
    );
    assert!(
        !diagnostics.has_errors(),
        "{:?}",
        diagnostics.errors().collect::<Vec<_>>()
    );
    session
        .run(
            SourceIdentity::new(name, source),
            Rc::new(program.unwrap()),
            Box::new(ColumnHost(HeadlessHost::new())),
            Limits::default(),
            false,
            |machine| machine.call_function(entry, vec![]),
        )
        .result
        .unwrap()
}
#[test]
fn original_g1_column_programs_match_across_runs_apps_and_failed_writes() {
    use sha2::{Digest, Sha256};
    let controls: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-statistics-column-controls.json"
    ))
    .unwrap();
    for suite in controls["suites"].as_array().unwrap() {
        let mut session = SessionContext::default();
        for case in suite["cases"].as_array().unwrap() {
            let source = case["source"].as_str().unwrap();
            assert_eq!(
                format!("{:x}", Sha256::digest(source.as_bytes())),
                case["reference_source_sha256"]
            );
            let result = execute(
                &mut session,
                case["name"].as_str().unwrap(),
                source,
                case["entry"].as_str().unwrap(),
            );
            assert_eq!(
                json(&result),
                case["expected"],
                "{} {}",
                suite["name"],
                case["name"]
            );
        }
    }
}
#[test]
fn gui_column_edits_are_seen_by_ppl_and_survive_memory_restore_without_eager_defaults() {
    let mut session = SessionContext::default();
    for prefix in ['C', 'D'] {
        for index in 0..=9 {
            let name = format!("{prefix}{index}");
            assert!(!session.snapshot().globals.contains_key(&name));
            session
                .set_variable(
                    &name,
                    PortableValue::List(vec![PortableValue::Real(ppl_math::Real::from_i64(index))]),
                )
                .unwrap();
        }
    }
    assert_eq!(session.snapshot().statistics2_columns_revision, 10);
    let value = execute(
        &mut session,
        "column write",
        "EXPORT COL() BEGIN C9(1):=C9(1)+D0(1)+10; RETURN {C0,C9,D9}; END;",
        "COL",
    );
    assert_eq!(json(&value), serde_json::json!([[0], [19], [9]]));
    assert_eq!(session.snapshot().statistics2_columns_revision, 11);
    let saved = session.memory_json();
    let mut restored = SessionContext::default();
    restored.restore_memory(&saved).unwrap();
    let value = restored
        .evaluate_home(
            "C9(1)+D9(1)",
            Box::new(HeadlessHost::new()),
            Limits::default(),
        )
        .result
        .unwrap();
    assert_eq!(json(&value), serde_json::json!(28));
    restored
        .set_variable("C1", PortableValue::List(vec![]))
        .unwrap();
    assert_eq!(
        restored.snapshot().globals["C1"],
        PortableValue::List(vec![])
    );
    assert_eq!(restored.snapshot().statistics2_columns_revision, 1);
    restored
        .set_variable("C1", PortableValue::List(vec![]))
        .unwrap();
    assert_eq!(restored.snapshot().statistics2_columns_revision, 2);
    let before = restored.memory_json();
    assert!(
        restored
            .set_variable("C1", PortableValue::String("bad".into()))
            .is_err()
    );
    assert_eq!(restored.snapshot().statistics2_columns_revision, 2);
    assert_eq!(restored.memory_json(), before);
    let mut broken = before.clone();
    broken["globals"]["C1"] =
        serde_json::json!({"kind":"list","value":[{"kind":"string","value":"bad"}]});
    assert!(restored.restore_memory(&broken).is_err());
    assert_eq!(restored.memory_json(), before);
}
#[test]
fn locals_named_like_columns_retain_ordinary_value_semantics() {
    let mut session = SessionContext::default();
    let value = execute(
        &mut session,
        "column shadow",
        "EXPORT SHADOW(C1) BEGIN C1:=\"private\"; RETURN C1; END; EXPORT TEST() BEGIN RETURN SHADOW(1); END;",
        "TEST",
    );
    assert_eq!(json(&value), serde_json::json!("private"));
    assert!(!session.snapshot().globals.contains_key("C1"));
}
