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
fn original_grob_dimensions_follow_pixel_sizes_and_scoped_plot_units() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-grob-dimension-controls.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let source = case["source"].as_str().unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(source)),
            case["source_sha256"]
        );
        let name = case["name"].as_str().unwrap();
        let (program, diagnostics) = ppl_compiler::compile_source(
            &ppl_syntax::SourceFile::new(name, source),
            &Default::default(),
        );
        assert!(!diagnostics.has_errors(), "{name}: {diagnostics:?}");
        let mut session = current("Function");
        let result = session.run(
            SourceIdentity::new(name, source),
            Rc::new(program.unwrap()),
            Box::new(PlotHost(HeadlessHost::new())),
            Limits::default(),
            false,
            |machine| machine.call_function(case["entry"].as_str().unwrap(), vec![]),
        );
        let actual = result
            .result
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(
            actual
                .string_form(ppl_vm::Settings::default().format())
                .replace(' ', ""),
            case["return_display"].as_str().unwrap().replace(' ', ""),
            "{name}"
        );
    }
}

#[test]
fn original_geometry_scoped_settings_programs_match_without_geometry_presentation() {
    let controls: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-geometry-plot-controls.json"
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
            &ppl_syntax::SourceFile::new("geometry-plot-control", source),
            &Default::default(),
        );
        assert!(!diagnostics.has_errors(), "{case}: {diagnostics:?}");
        let result = session.run(
            SourceIdentity::new("geometry-plot-control", source),
            Rc::new(compiled.unwrap()),
            Box::new(PlotHost(HeadlessHost::new())),
            Limits::default(),
            false,
            |m| m.call_function(case["entry"].as_str().unwrap(), vec![]),
        );
        assert_eq!(
            result
                .result
                .unwrap()
                .expect_str("Geometry result")
                .unwrap()
                .as_ref(),
            case["expected"].as_str().unwrap(),
            "{}",
            case["name"]
        );
        for name in ppl_vm::GeometryPlotSettings::NAMES {
            assert!(
                !session.snapshot().globals.contains_key(name),
                "Geometry {name} leaked into Home: {}",
                case["name"]
            );
        }
    }
}
#[test]
fn geometry_scoped_state_roundtrips_and_invalid_restore_is_atomic() {
    let mut session = current("Function");
    home(
        &mut session,
        "Geometry.Xmin:=-7;Geometry.Ymin:=-3;Geometry.PixSize:=0.2",
    );
    assert_eq!(session.active_app, "Function");
    assert_eq!(
        home(&mut session, "Xmin")
            .expect_real("Function Xmin")
            .unwrap(),
        Real::parse("-15.9").unwrap()
    );
    let saved = session.memory_json();
    let mut restored = current("Function");
    restored.restore_memory(&saved).unwrap();
    assert!(
        restored.memory_json() == saved,
        "Restored settings differ from persisted settings"
    );
    let mut malformed = saved.clone();
    malformed["geometry_plot_settings"]["PixSize"] =
        ppl_vm::session::encode_value(&PortableValue::Real(Real::parse("0.001").unwrap()));
    assert!(restored.restore_memory(&malformed).is_err());
    assert!(
        restored.memory_json() == saved,
        "Restored settings differ from persisted settings"
    );
    let mut old = saved.clone();
    old["version"] = 5.into();
    old["active_app"] = "Geometry".into();
    old.as_object_mut()
        .unwrap()
        .remove("geometry_plot_settings");
    old["globals"]["Xmin"] = ppl_vm::session::encode_value(&PortableValue::Real(Real::from_i64(7)));
    restored.restore_memory(&old).unwrap();
    assert!(restored.geometry_plot_settings.is_none());
    assert!(
        restored.snapshot().globals.contains_key("Xmin"),
        "Legacy data must not be silently discarded"
    );
}
#[test]
fn geometry_bare_settings_preserve_scope_after_switching_to_a_non_plot_app() {
    let mut session = current("Geometry");
    home(&mut session, "Xmin:=7;STARTAPP(\"Spreadsheet\")");
    assert_eq!(session.geometry_plot_settings().xmin, Real::from_i64(7));
    assert!(!session.snapshot().globals.contains_key("Xmin"));
}
#[test]
fn original_geometry_conversion_programs_use_graphics_projection_not_canvas_pixels() {
    let fixtures: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-geometry-coordinate-controls.json"
    ))
    .unwrap();
    for case in fixtures["cases"].as_array().unwrap() {
        let source = case["source"].as_str().unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(source)),
            case["source_sha256"]
        );
        let mut session = current("Geometry");
        let (program, diagnostics) = ppl_compiler::compile_source(
            &ppl_syntax::SourceFile::new("geometry-coordinates", source),
            &Default::default(),
        );
        assert!(!diagnostics.has_errors(), "{diagnostics:?}");
        let result = session
            .run(
                SourceIdentity::new("geometry-coordinates", source),
                Rc::new(program.unwrap()),
                Box::new(PlotHost(HeadlessHost::new())),
                Limits::default(),
                false,
                |m| m.call_function(case["entry"].as_str().unwrap(), vec![]),
            )
            .result
            .unwrap();
        let pairs = result.expect_list("coordinate pairs").unwrap();
        assert_eq!(pairs.len(), case["expected"].as_array().unwrap().len());
        for (actual, expected) in pairs.iter().zip(case["expected"].as_array().unwrap()) {
            let pair = actual.expect_list("coordinate pair").unwrap();
            assert_eq!(pair.len(), 2);
            for (actual, expected) in pair.iter().zip(expected.as_array().unwrap()) {
                assert!(
                    (actual.expect_real("coordinate").unwrap().to_f64()
                        - expected.as_f64().unwrap())
                    .abs()
                        <= 1e-11,
                    "{}: actual{actual:?}, expected{expected}",
                    case["name"]
                );
            }
        }
    }
    let mut session = current("Geometry");
    home(
        &mut session,
        "Geometry.Xmin:=-7;Geometry.Ymin:=-3;Geometry.PixSize:=0.2;PIXON(0,0,RGB(255,0,0))",
    );
    assert_eq!(
        home(&mut session, "GETPIX_P(36,203)")
            .expect_i64("pixel")
            .unwrap()
            & 0xffffff,
        0xff0000
    );
    assert_eq!(
        home(&mut session, "GETPIX_P(35,204)")
            .expect_i64("pixel")
            .unwrap()
            & 0xffffff,
        0xffffff,
        "PIXON retains graphics projection while the actual Geometry canvas puts this point at35,204"
    );
    session.active_app = "Function".into();
    home(&mut session, "Xmin:=0;Xmax:=10;Ymin:=0;Ymax:=10");
    assert_eq!(
        home(&mut session, "STRING(C→PX({5,5}))")
            .expect_str("Function conversion")
            .unwrap()
            .as_ref(),
        "{160,109}"
    );
}
#[test]
fn original_coordinate_argument_shapes_flooring_and_failed_component_conversion_match() {
    let mut session = current("Geometry");
    for (source, expected) in [
        ("C→PX({1})", [0., 0.]),
        ("C→PX({1,2,3})", [0., 0.]),
        ("C→PX([1,2])", [0., 0.]),
        ("C→PX({(2,1),0})", [0., 0.]),
        ("C→PX({#2h,0})", [180., 109.]),
        ("C→PX({(2,0),0})", [180., 109.]),
        ("C→PX((1,2))", [170., 89.]),
        ("C→PX(1,2)", [170., 89.]),
        ("PX→C({160.5,109.5})", [-0.0499999999999; 2]),
        ("PX→C((160,109))", [-0.0499999999999; 2]),
        ("PX→C({-1.5,-1.5})", [-16.3009433962, 11.1009174312]),
        ("PX→C({1})", [-16.1003144654, 10.9]),
        ("PX→C({1E499,109})", [-16.1003144654, 10.9]),
        ("PX→C({109,1E499})", [-5.16603773585, 10.9]),
    ] {
        let value = home(&mut session, source);
        let pair = value.expect_list(source).unwrap();
        assert_eq!(pair.len(), 2);
        for (actual, expected) in pair.iter().zip(expected) {
            assert!(
                (actual.expect_real(source).unwrap().to_f64() - expected).abs() <= 5e-11,
                "{source}: {actual:?} expected{expected}"
            );
        }
    }
    for source in ["C→PX()", "C→PX(1,2,3)", "PX→C()", "PX→C(1,2,3)"] {
        assert!(
            session
                .evaluate_home(source, Box::new(HeadlessHost::new()), Limits::default())
                .result
                .is_err(),
            "{source} must reject unsupported arity"
        );
    }
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
fn statistics_signed_y_bounds_are_independent_typed_profile_values() {
    use ppl_cas::{CasValue, InfinitySign};

    let mut session = current("Statistics 1Var");
    let negative = PortableValue::Symbolic(CasValue::Infinity(InfinitySign::Negative));
    let positive = PortableValue::Symbolic(CasValue::Infinity(InfinitySign::Positive));
    session.set_variable("Ymin", negative.clone()).unwrap();
    session.set_variable("Ymax", positive.clone()).unwrap();
    let display = home(
        &mut session,
        "{STRING(Ymin),STRING(Ymax),TYPE(Ymin),TYPE(Ymax)}",
    );
    let values = display.expect_list("signed plot bounds").unwrap();
    assert_eq!(values[0].expect_str("Ymin").unwrap().as_ref(), "-Inf");
    assert_eq!(values[1].expect_str("Ymax").unwrap().as_ref(), "+Inf");
    assert_eq!(values[2].expect_i64("TYPE(Ymin)").unwrap(), 0);
    assert_eq!(values[3].expect_i64("TYPE(Ymax)").unwrap(), 0);
    let saved = session.memory_json();

    home(&mut session, "Ymin:=-3");
    let settings = session
        .snapshot()
        .plot_settings_for("Statistics 1Var")
        .unwrap();
    assert_eq!(settings.ymin, ppl_vm::PlotBound::Finite(Real::from_i64(-3)));
    assert_eq!(settings.ymax, ppl_vm::PlotBound::PositiveInfinity);
    let mut restored = current("Function");
    restored.restore_memory(&saved).unwrap();
    let restored_settings = restored
        .snapshot()
        .plot_settings_for("Statistics 1Var")
        .unwrap();
    assert_eq!(restored_settings.ymin, ppl_vm::PlotBound::NegativeInfinity);
    assert_eq!(restored_settings.ymax, ppl_vm::PlotBound::PositiveInfinity);

    let mut malformed = saved.clone();
    let unsigned = PortableValue::Symbolic(CasValue::Infinity(InfinitySign::Unsigned));
    malformed["app_plot_settings"]["Statistics 1Var"]["Ymax"] =
        ppl_vm::session::encode_value(&unsigned);
    let before = restored.memory_json();
    assert!(restored.restore_memory(&malformed).is_err());
    assert_eq!(restored.memory_json(), before);
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

#[test]
fn snapshots_keep_home_names_separate_from_scoped_plot_values() {
    let mut session = current("Function");
    let before = session.snapshot().globals;
    assert!(session.snapshot().plot_settings_for("Function").is_some());
    assert_eq!(session.snapshot().globals, before);
    home(
        &mut session,
        "Xmin:=7;Function.Xmax:=23;Statistics_1Var.Hwidth:=3",
    );
    let snapshot = session.snapshot();
    for field in ppl_vm::AppPlotSettings::NAMES {
        assert!(
            !snapshot.globals.contains_key(field),
            "invented Home name {field}"
        );
    }
    assert_eq!(
        snapshot.app_plot_settings["Function"].xmin,
        Real::from_i64(7)
    );
    assert_eq!(
        snapshot.app_plot_settings["Function"].xmax,
        Real::from_i64(23)
    );
    assert_eq!(
        snapshot.app_plot_settings["Statistics 1Var"].hwidth,
        Real::from_i64(3)
    );
    // A read-only consumer for another app can copy every genuine Home name.
    // Merely copying a snapshot must not request Spreadsheet/Geometry Plot Setup.
    for app in ["Spreadsheet", "Geometry"] {
        let mut isolated = current(app);
        isolated.app_plot_settings = snapshot.app_plot_settings.clone();
        for (name, value) in &snapshot.globals {
            isolated.set_variable(name, value.clone()).unwrap();
        }
        assert_eq!(
            home(&mut isolated, "6*7")
                .expect_real("calculation")
                .unwrap(),
            Real::from_i64(42)
        );
        assert_eq!(
            session.snapshot().app_plot_settings,
            snapshot.app_plot_settings
        );
    }
}

#[test]
fn invalid_qualified_column_edits_are_atomic_in_the_live_vm() {
    let mut session = current("Function");
    home(&mut session, "Statistics_1Var.D1:={2,4,6}");
    for source in [
        "Statistics_1Var.D1(2):=\"bad\"",
        "Statistics_1Var.D1:={1,{2}}",
    ] {
        let run = session.evaluate_home(source, Box::new(HeadlessHost::new()), Limits::default());
        assert!(run.result.is_err(), "{source}");
        assert_eq!(
            session.snapshot().globals["D1"],
            PortableValue::List(vec![
                PortableValue::Real(Real::from_i64(2)),
                PortableValue::Real(Real::from_i64(4)),
                PortableValue::Real(Real::from_i64(6)),
            ])
        );
    }
    assert_eq!(
        home(&mut session, "Statistics_1Var.D1(2)")
            .expect_real("cell")
            .unwrap(),
        Real::from_i64(4)
    );
}
