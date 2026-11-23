//! Original firmware Plot Setup and qualified-column contracts. The host only
//! acknowledges STARTAPP; all variables, validation and arithmetic use the VM.
use ppl_math::{NumericVersion, Real};
use ppl_vm::{HeadlessHost, Host, Limits, PortableValue, SessionContext, SourceIdentity, Value};
use sha2::{Digest, Sha256};
use std::rc::Rc;
struct PlotHost(HeadlessHost);
impl Host for PlotHost {
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

fn current(app: &str) -> SessionContext {
    let mut session = SessionContext::new(
        ppl_vm::Settings {
            numeric_version: NumericVersion::Current15751,
            ..Default::default()
        },
        1,
    );
    session.active_app = app.into();
    session
}
fn home(session: &mut SessionContext, source: &str) -> Value {
    session
        .evaluate_home(
            source,
            Box::new(PlotHost(HeadlessHost::new())),
            Limits::default(),
        )
        .result
        .unwrap()
}
#[test]
fn unchanged_original_plot_and_qualified_column_programs_match() {
    let controls: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-plot-settings-controls.json"
    ))
    .unwrap();
    for case in controls["cases"].as_array().unwrap() {
        let source = case["source"].as_str().unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(source)),
            case["source_sha256"]
        );
        let mut session = current(case["active_app"].as_str().unwrap());
        let (compiled, diagnostics) = ppl_compiler::compile_source(
            &ppl_syntax::SourceFile::new("plot-controls", source),
            &Default::default(),
        );
        assert!(!diagnostics.has_errors(), "{case}: {diagnostics:?}");
        let run = session.run(
            SourceIdentity::new("plot-controls", source),
            Rc::new(compiled.unwrap()),
            Box::new(PlotHost(HeadlessHost::new())),
            Limits::default(),
            false,
            |m| m.call_function(case["entry"].as_str().unwrap(), vec![]),
        );
        assert_eq!(
            run.result
                .unwrap()
                .expect_str("original plot result")
                .unwrap()
                .as_ref(),
            case["expected"].as_str().unwrap(),
            "{}",
            case["name"]
        );
        assert!(
            !session.memory_json()["globals"]
                .as_object()
                .unwrap()
                .contains_key("Xmin"),
            "Plot settings leaked to Home"
        );
    }
}
#[test]
fn scoped_plot_memory_restores_and_rejects_malformed_state_atomically() {
    let mut session = current("Statistics 1Var");
    home(&mut session, "Xmin:=7;Hwidth:=3;Xtick:=0.25;Axes:=2");
    home(&mut session, "STARTAPP(\"Function\");Xmin:=4;Xmax:=42");
    let saved = session.memory_json();
    let mut restored = current("Function");
    restored.restore_memory(&saved).unwrap();
    assert_eq!(
        restored.snapshot().app_plot_settings,
        session.snapshot().app_plot_settings
    );
    assert_eq!(
        home(&mut restored, "Xmin").expect_real("range").unwrap(),
        Real::from_i64(4)
    );
    assert_eq!(
        home(&mut restored, "Statistics_1Var.Xmin")
            .expect_real("range")
            .unwrap(),
        Real::from_i64(7)
    );
    for (field, value) in [("Xtick", "0"), ("Hwidth", "-1"), ("Axes", "2")] {
        let mut bad = saved.clone();
        bad["app_plot_settings"]["Statistics 1Var"][field] =
            ppl_vm::session::encode_value(&PortableValue::Real(Real::parse(value).unwrap()));
        let before = restored.memory_json();
        assert!(restored.restore_memory(&bad).is_err(), "{field}");
        assert_eq!(
            restored.memory_json(),
            before,
            "Rejected state mutated memory"
        );
    }
    let mut old = saved.clone();
    old.as_object_mut().unwrap().remove("app_plot_settings");
    old["globals"]["Xmin"] =
        ppl_vm::session::encode_value(&PortableValue::Real(Real::from_i64(19)));
    restored.restore_memory(&old).unwrap();
    assert_eq!(
        home(&mut restored, "Xmin").expect_real("migrated").unwrap(),
        Real::from_i64(19)
    );
    assert!(
        !restored.memory_json()["globals"]
            .as_object()
            .unwrap()
            .contains_key("Xmin")
    );
}
#[test]
fn declared_and_dynamic_locals_do_not_alias_app_settings_or_qualified_columns() {
    let mut session = current("Function");
    session.store_program("SCOPE", "EXPORT SCOPE() BEGIN LOCAL D1:={77},Xmin:=4; Statistics_1Var.D1:={9}; EXPR(\"Xmin:=5\"); RETURN {Xmin,D1,Statistics_1Var.D1,Function.Xmin}; END;".into()).unwrap();
    let value = home(&mut session, "SCOPE()");
    assert_eq!(
        PortableValue::from(&value),
        PortableValue::List(vec![
            PortableValue::Real(Real::from_i64(5)),
            PortableValue::List(vec![PortableValue::Real(Real::from_i64(77))]),
            PortableValue::List(vec![PortableValue::Real(Real::from_i64(9))]),
            PortableValue::Real(Real::parse("-15.9").unwrap()),
        ])
    );
    assert_eq!(
        session.snapshot().globals["D1"],
        PortableValue::List(vec![PortableValue::Real(Real::from_i64(9))])
    );
}
#[test]
fn error_exit_retains_actual_scoped_writes_without_phantom_home_values() {
    let mut session = current("Statistics 1Var");
    let run = session.evaluate_home(
        "Xmin:=7;Hwidth:=3;1/0",
        Box::new(HeadlessHost::new()),
        Limits::default(),
    );
    assert!(run.result.is_err());
    let plot = session.snapshot().app_plot_settings().unwrap();
    assert_eq!(plot.xmin, Real::from_i64(7));
    assert_eq!(plot.hwidth, Real::from_i64(3));
    assert!(
        !session.memory_json()["globals"]
            .as_object()
            .unwrap()
            .contains_key("Hwidth")
    );
    let saved = session.memory_json();
    let mut restored = current("Function");
    restored.restore_memory(&saved).unwrap();
    assert_eq!(restored.snapshot().app_plot_settings().unwrap(), plot);
}
