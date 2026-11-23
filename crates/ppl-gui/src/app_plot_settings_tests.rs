//! Plot Setup regressions through the production GUI request and VM workers.
use super::*;

fn run(app: &mut IdeDesktopApp, ctx: &egui::Context, name: &str, source: &str) {
    app.handle_native_tool_request(Request::SaveProgram(name.into(), source.into()), ctx);
    app.handle_native_tool_request(Request::RunProgram(name.into(), source.into()), ctx);
    wait(app, ctx, |app| {
        app.native_compile.is_none()
            && !app.controller.worker_alive()
            && matches!(
                app.state.vm_state,
                crate::ide::VmExecutionState::Terminated | crate::ide::VmExecutionState::Error(_)
            )
    });
    assert!(
        matches!(app.state.vm_state, crate::ide::VmExecutionState::Terminated),
        "{:?}",
        app.state.native.last_outcome
    );
}

#[test]
fn fresh_function_library_selection_keeps_firmware_plot_defaults() {
    let (mut app, ctx) = fixture(Page::Programs);
    app.handle_native_tool_request(Request::SelectApp("Function".into()), &ctx);
    frame(&mut app, &ctx, vec![]);
    let snapshot = app.controller.session_snapshot();
    assert_eq!(
        snapshot.app_plot_settings["Function"],
        ppl_vm::AppPlotSettings::for_app("Function").unwrap()
    );
}

#[test]
fn function_library_selection_keeps_an_existing_ui_only_window() {
    let (mut app, ctx) = fixture(Page::Programs);
    app.native_tools
        .memory
        .graphs
        .get_mut("Function")
        .unwrap()
        .window = crate::ide::native_apps::PlotWindow::default();
    app.handle_native_tool_request(Request::SelectApp("Function".into()), &ctx);
    frame(&mut app, &ctx, vec![]);
    let snapshot = app.controller.session_snapshot();
    let window = &snapshot.app_plot_settings["Function"];
    assert_eq!(
        [
            window.xmin.to_f64(),
            window.xmax.to_f64(),
            window.ymin.to_f64(),
            window.ymax.to_f64()
        ],
        [-10., 10., -10., 10.]
    );
    assert!(window.axes);
}

#[test]
fn qualified_inactive_windows_survive_startapp_and_remain_independent() {
    let (mut app, ctx) = fixture(Page::Programs);
    run(
        &mut app,
        &ctx,
        "SCOPEDPLOT",
        "EXPORT SCOPEDPLOT() BEGIN Function.Xmin:=2;Function.Xmax:=8;Parametric.Xmin:=-3;Parametric.Xmax:=9;STARTAPP(\"Parametric\");STARTAPP(\"Function\");RETURN {Xmin,Xmax,Parametric.Xmin,Parametric.Xmax};END;",
    );
    let snapshot = app.controller.session_snapshot();
    assert_eq!(
        snapshot
            .last_value
            .as_ref()
            .unwrap()
            .display(&snapshot.settings),
        "{2,8,-3,9}"
    );
    assert_eq!(app.native_tools.memory.graphs["Function"].window.xmin, 2.);
    assert_eq!(app.native_tools.memory.graphs["Function"].window.xmax, 8.);
    assert_eq!(
        app.native_tools.memory.graphs["Parametric"].window.xmin,
        -3.
    );
    assert_eq!(app.native_tools.memory.graphs["Parametric"].window.xmax, 9.);
    assert!(!snapshot.globals.contains_key("Xmin"));
    assert!(!snapshot.globals.contains_key("Xmax"));
}

#[test]
fn visible_plot_setup_apply_and_axes_checkbox_change_real_ppl_settings() {
    let (mut app, ctx) = fixture(Page::PlotSetup);
    {
        let graph = app.native_tools.memory.graphs.get_mut("Function").unwrap();
        graph.curves[0].expression = "X^2".into();
        graph.window.xmin = 2.;
        graph.window.xmax = 8.;
        graph.window.ymin = -3.;
        graph.window.ymax = 5.;
        graph.window.axes = true;
    }
    click(&mut app, &ctx, "Axes");
    assert!(!app.native_tools.memory.graphs["Function"].window.axes);
    click(&mut app, &ctx, "Apply");
    assert_eq!(app.native_tools.page, Some(Page::Plot));
    wait(&mut app, &ctx, |app| !app.native_tools.calculating());
    run(
        &mut app,
        &ctx,
        "READPLOT",
        "EXPORT READPLOT() BEGIN RETURN {Xmin,Xmax,Ymin,Ymax,Axes};END;",
    );
    let snapshot = app.controller.session_snapshot();
    assert_eq!(
        snapshot
            .last_value
            .as_ref()
            .unwrap()
            .display(&snapshot.settings),
        "{2,8,-3,5,0}"
    );
    assert!(!snapshot.globals.contains_key("Axes"));
    assert!(!snapshot.globals.contains_key("Ymin"));
    assert!(
        app.native_tools.error.is_none(),
        "{:?}",
        app.native_tools.error
    );
}

#[test]
fn statistics_autoscale_worker_returns_all_checked_bounds_and_histogram_limits_to_live_ppl() {
    use crate::ide::native_statistics::one::Chart;
    let (mut app, ctx) = fixture(Page::Programs);
    app.native_tools
        .memory
        .models
        .ensure_statistics_book(false)
        .unwrap();
    {
        let book = app
            .native_tools
            .memory
            .models
            .statistics1_book
            .as_mut()
            .unwrap();
        book.analyses[0].chart = Chart::Bar;
        book.analyses[1].enabled = true;
        book.analyses[1].data = Some(1);
        book.analyses[1].chart = Chart::Bar;
    }
    app.controller
        .mutate_memory(|memory| {
            memory.settings.numeric_version = ppl_math::NumericVersion::Current15751;
            for (name, data) in [("D1", vec![1, 2, 4]), ("D2", vec![100, 200, 400])] {
                memory
                    .set_variable(
                        name,
                        ppl_vm::PortableValue::List(
                            data.into_iter()
                                .map(|n| ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(n)))
                                .collect(),
                        ),
                    )
                    .map_err(|e| e.to_string())?;
            }
            Ok(())
        })
        .unwrap();
    run(
        &mut app,
        &ctx,
        "STATSAUTO",
        "EXPORT STATSAUTO() BEGIN STARTAPP(\"Statistics 1Var\");STARTVIEW(9,1);RETURN {Xmin,Xmax,Ymin,Ymax};END;",
    );
    let snapshot = app.controller.session_snapshot();
    let ppl_vm::PortableValue::List(bounds) = snapshot.last_value.as_ref().unwrap() else {
        panic!("missing live bounds")
    };
    // Actual two-analysis Bar control from native-statistics-one-autoscale-controls.json.
    for (value, expected) in bounds.iter().zip([-0.303620689655, 3.315, -88., 488.]) {
        assert!((value.to_value().expect_real("bound").unwrap().to_f64() - expected).abs() < 1E-9);
    }
    {
        let book = app
            .native_tools
            .memory
            .models
            .statistics1_book
            .as_mut()
            .unwrap();
        book.analyses[1].enabled = false;
        book.analyses[0].chart = Chart::Histogram;
    }
    run(
        &mut app,
        &ctx,
        "STATSHIST",
        "EXPORT STATSHIST() BEGIN STARTAPP(\"Statistics 1Var\");Hwidth:=1;STARTVIEW(9,1);RETURN {Hmin,Hmax,Hwidth};END;",
    );
    assert_eq!(
        app.controller
            .session_snapshot()
            .last_value
            .unwrap()
            .display(&snapshot.settings),
        "{1,5,1}"
    );
    let data = &app.native_tools.memory.models.results["Statistics 1Var:plot"];
    assert_eq!(data["scale"]["histogram"]["minimum"], 1.);
    assert_eq!(data["scale"]["histogram"]["maximum"], 5.);
}

#[test]
fn statistics_goto_editor_keeps_committed_and_cancelled_selection_but_home_resets_it() {
    let (mut app, ctx) = fixture(Page::Programs);
    run(
        &mut app,
        &ctx,
        "STATGOTO",
        r#"EXPORT STATGOTO() BEGIN
        STARTAPP("Statistics 1Var");D1:={3,1,3};SetFreq(H1,10001);
        H1(3):=8;STARTVIEW(9,1);RETURN 1;
    END;"#,
    );
    wait(&mut app, &ctx, |app| !app.native_tools.calculating());
    let point = |app: &IdeDesktopApp| {
        app.native_tools
            .trace
            .current(&app.native_tools.trace_samples())
            .unwrap()
            .point
    };
    for (source, commit, expected) in [
        ("3", true, [3., 20002.]),
        ("1", true, [1., 10001.]),
        ("3", false, [1., 10001.]),
        ("3", true, [3., 20002.]),
    ] {
        app.open_math_entry(crate::ide::native_tools::MathTarget::StatisticsPlotX, &ctx);
        app.state.native.edit(|editor| editor.set_text(source));
        assert!(app.finish_math_entry(commit, &ctx));
        assert_eq!(app.native_tools.page, Some(Page::Plot));
        assert_eq!(point(&app), expected);
    }
    app.native_tools.close();
    app.native_tools.open(Page::Plot);
    assert_eq!(point(&app), [1., 10001.]);
}

#[test]
fn fresh_sequence_library_launch_keeps_firmware_plot_defaults() {
    let (mut app, ctx) = fixture(Page::Programs);
    app.handle_native_tool_request(Request::StartApp("Sequence".into()), &ctx);
    frame(&mut app, &ctx, vec![]);
    let snapshot = app.controller.session_snapshot();
    assert_eq!(snapshot.active_app, "Sequence");
    assert_eq!(app.native_tools.page, Some(Page::Symbolic));
    assert_eq!(
        snapshot.app_plot_settings["Sequence"],
        ppl_vm::AppPlotSettings::for_app("Sequence").unwrap()
    );
}

#[test]
fn sequence_library_reset_restores_only_its_definitions_and_setup() {
    use crate::ide::native_apps::SequenceDefinition;
    use crate::ide::native_tools::ResetApp;

    let (mut app, ctx) = fixture(Page::Programs);
    run(
        &mut app,
        &ctx,
        "SEQRESETSEED",
        "EXPORT SEQRESETSEED() BEGIN A:=46467; HAngle:=1; STARTAPP(\"Function\"); Function.Xmin:=-12.25; Function.F1:=\"X+47\"; STARTAPP(\"Sequence\"); Xmin:=-4.5; Xmax:=22.125; Ymin:=-7.75; Ymax:=31.5; Xtick:=2; Ytick:=3; Axes:=1; GridDots:=1; GridLines:=1; Labels:=1; NumStart:=5; NumStep:=2; NumZoom:=4; NumIndep:={5,9}; NumType:=1; AAngle:=2; AComplex:=2; AFormat:=2; ADigits:=7; END;",
    );
    let graph = app.native_tools.memory.graphs.get_mut("Sequence").unwrap();
    graph.curves[0].expression = "U1(N-1)+2".into();
    graph.curves[0].enabled = true;
    graph.curves[0].sequence = Some(SequenceDefinition {
        start: 1,
        initial: vec!["3".into(), "5".into()],
        offset: 0,
    });

    app.native_tools.open(Page::Apps);
    app.handle_native_tool_request(Request::ResetApp(ResetApp::Sequence), &ctx);
    assert_eq!(app.native_tools.page, Some(Page::Apps));
    let curves = &app.native_tools.memory.graphs["Sequence"].curves;
    assert_eq!(curves.len(), 1);
    assert!(curves[0].expression.is_empty());
    assert!(!curves[0].enabled);
    assert!(curves[0].sequence.is_none());

    run(
        &mut app,
        &ctx,
        "SEQRESETREAD",
        "EXPORT SEQRESETREAD() BEGIN RETURN {Xmin,Xmax,Ymin,Ymax,Xtick,Ytick,Axes,GridDots,GridLines,Labels,NumStart,NumStep,NumZoom,NumType,TYPE(NumIndep),NumIndep,AAngle,AComplex,AFormat,ADigits,HAngle,A,Function.Xmin,Function.F1(2)}; END;",
    );
    let snapshot = app.controller.session_snapshot();
    assert_eq!(snapshot.active_app, "Sequence");
    assert_eq!(
        snapshot.last_value.unwrap().display(&snapshot.settings),
        "{-1.8,30,-1.8,20,1,1,0,0,0,0,1,1,2,0,6,{},0,0,0,4,1,46467,-12.25,49}"
    );
}

#[test]
fn sequence_library_selection_keeps_an_existing_ui_only_window() {
    let (mut app, ctx) = fixture(Page::Programs);
    app.native_tools
        .memory
        .graphs
        .get_mut("Sequence")
        .unwrap()
        .window = crate::ide::native_apps::PlotWindow::default();
    app.handle_native_tool_request(Request::SelectApp("Sequence".into()), &ctx);
    frame(&mut app, &ctx, vec![]);
    let snapshot = app.controller.session_snapshot();
    let window = &snapshot.app_plot_settings["Sequence"];
    assert_eq!(
        [
            window.xmin.to_f64(),
            window.xmax.to_f64(),
            window.ymin.to_f64(),
            window.ymax.to_f64()
        ],
        [-10., 10., -10., 10.]
    );
    assert!(window.axes);
}

#[test]
fn statistics_app_selection_uses_measured_defaults_and_preserves_legacy_or_scoped_windows() {
    use crate::ide::native_apps::PlotWindow;
    for name in ["Statistics 1Var", "Statistics 2Var"] {
        let (mut app, ctx) = fixture(Page::Programs);
        app.handle_native_tool_request(Request::SelectApp(name.into()), &ctx);
        frame(&mut app, &ctx, vec![]);
        let settings = app.controller.session_snapshot().app_plot_settings[name].clone();
        assert_eq!(settings, ppl_vm::AppPlotSettings::for_app(name).unwrap());
        let window = &app.native_tools.memory.model_windows[name];
        assert_eq!(
            [window.xmin, window.xmax, window.ymin, window.ymax],
            [-1.8, 30., -1.8, 20.]
        );
        assert!(
            !app.controller
                .session_snapshot()
                .globals
                .contains_key("Xmin")
        );
    }
    let (mut app, ctx) = fixture(Page::Programs);
    app.native_tools.memory.model_windows.insert(
        "Statistics 1Var".into(),
        PlotWindow {
            xmin: 2.,
            xmax: 8.,
            ymin: -3.,
            ymax: 5.,
            axes: true,
            ..Default::default()
        },
    );
    app.native_tools.memory.models.histogram =
        Some(crate::ide::native_model_plots::HistogramSpec {
            minimum: 1.,
            maximum: 6.,
            width: 0.5,
        });
    app.handle_native_tool_request(Request::SelectApp("Statistics 1Var".into()), &ctx);
    frame(&mut app, &ctx, vec![]);
    let settings = app.controller.session_snapshot().app_plot_settings["Statistics 1Var"].clone();
    assert_eq!(
        [
            settings.xmin.to_f64(),
            settings.xmax.to_f64(),
            settings.ymin.to_f64(),
            settings.ymax.to_f64()
        ],
        [2., 8., -3., 5.]
    );
    assert!(settings.axes);
    assert_eq!(
        [
            settings.hmin.to_f64(),
            settings.hmax.to_f64(),
            settings.hwidth.to_f64()
        ],
        [1., 6., 0.5]
    );
    let mut explicit = ppl_vm::AppPlotSettings::for_app("Statistics 2Var").unwrap();
    explicit.xmin = ppl_math::Real::from_i64(4);
    explicit.xmax = ppl_math::Real::from_i64(14);
    app.controller
        .mutate_memory(|memory| {
            memory
                .set_app_plot_settings("Statistics 2Var", explicit.clone())
                .map_err(|e| e.to_string())
        })
        .unwrap();
    app.native_tools
        .memory
        .model_windows
        .insert("Statistics 2Var".into(), PlotWindow::default());
    app.handle_native_tool_request(Request::SelectApp("Statistics 2Var".into()), &ctx);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(
        app.controller.session_snapshot().app_plot_settings["Statistics 2Var"],
        explicit
    );
    assert_eq!(
        app.native_tools.memory.model_windows["Statistics 2Var"].xmin,
        4.
    );
}

#[test]
fn geometry_setup_expression_rejects_invalid_value_and_commits_valid_correction() {
    use crate::ide::native_tools::MathTarget;
    let (mut app, ctx) = fixture(Page::Programs);
    app.handle_native_tool_request(Request::SelectApp("Geometry".into()), &ctx);
    app.native_tools.open(Page::PlotSetup);
    app.open_math_entry(MathTarget::GeometryPlotField("PixSize".into()), &ctx);
    app.state.native.edit(|editor| editor.set_text("1/1000"));
    assert!(app.finish_math_entry(true, &ctx));
    wait(&mut app, &ctx, |app| app.model_task.is_none());
    assert!(app.geometry_plot_edit_matches("PixSize", "1/1000"));
    assert_eq!(
        app.controller
            .session_snapshot()
            .geometry_plot_settings()
            .pix_size
            .to_f64(),
        0.1
    );

    app.state.native.edit(|editor| editor.set_text("1/5"));
    assert!(app.finish_math_entry(true, &ctx));
    wait(&mut app, &ctx, |app| app.model_task.is_none());
    assert_eq!(app.native_tools.page, Some(Page::PlotSetup));
    assert_eq!(
        app.controller
            .session_snapshot()
            .geometry_plot_settings()
            .pix_size
            .to_f64(),
        0.2
    );
    assert_eq!(app.native_tools.memory.model_windows["Geometry"].xmax, 47.8);
    run(
        &mut app,
        &ctx,
        "READGEOSET",
        "EXPORT READGEOSET() BEGIN RETURN {Geometry.Xmin,Geometry.Ymin,Geometry.PixSize};END;",
    );
    let snapshot = app.controller.session_snapshot();
    let values = snapshot.last_value.unwrap().to_value();
    let ppl_vm::Value::List(values) = values else {
        panic!("Expected scoped settings")
    };
    let values = values
        .iter()
        .map(|v| v.as_real().unwrap().to_f64())
        .collect::<Vec<_>>();
    assert_eq!(values, [-16., -11., 0.2]);
}

#[test]
fn geometry_legacy_camera_is_retained_when_the_scoped_viewport_is_created() {
    let (mut app, ctx) = fixture(Page::Programs);
    let legacy = crate::ide::native_apps::PlotWindow {
        xmin: -10.,
        xmax: 21.8,
        ymin: -12.,
        ymax: 9.8,
        axes: false,
        ..Default::default()
    };
    app.native_tools
        .memory
        .model_windows
        .insert("Geometry".into(), legacy.clone());
    app.handle_native_tool_request(Request::SelectApp("Geometry".into()), &ctx);
    let expected = serde_json::to_value(&legacy).unwrap();
    assert_eq!(
        serde_json::to_value(&app.native_tools.memory.geometry_legacy_window).unwrap(),
        expected
    );
    let settings = app.controller.session_snapshot().geometry_plot_settings();
    assert!(settings.axes);
    assert_eq!(
        [
            settings.xmin.to_f64(),
            settings.ymin.to_f64(),
            settings.pix_size.to_f64()
        ],
        [-10.1, -12.1, 0.1]
    );

    app.controller
        .mutate_memory(|memory| {
            memory
                .set_variable(
                    "PixSize",
                    ppl_vm::PortableValue::Real(ppl_math::Real::parse("0.25").unwrap()),
                )
                .map_err(|error| error.to_string())
        })
        .unwrap();
    app.handle_native_tool_request(Request::SelectApp("Geometry".into()), &ctx);
    assert_eq!(
        app.controller
            .session_snapshot()
            .geometry_plot_settings()
            .pix_size
            .to_f64(),
        0.25
    );
    assert_eq!(
        serde_json::to_value(&app.native_tools.memory.geometry_legacy_window).unwrap(),
        expected
    );
}
