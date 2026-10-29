//! Real Native worker/session tests through the production egui keyboard route.
//! These tests never open USB, send programs, or inject calculator hardware keys.
use super::*;
use crate::ide::native::NativeView;
use ppl_vm::PortableValue;

fn event_key(key: egui::Key, pressed: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

fn frame(app: &mut IdeDesktopApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    let input = egui::RawInput {
        focused: true,
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1280.0, 900.0),
        )),
        events,
        ..Default::default()
    };
    let _ = ctx.run(input, |ctx| {
        app.menu_bar.handle_shortcuts(ctx, &mut app.state);
        for action in app.state.take_actions() {
            app.handle_action(action, ctx);
        }
        app.forward_calc_keys(ctx);
        app.poll_execution();
        app.dialogs.render(ctx, &mut app.state, &mut app.editor);
    });
}

fn press(app: &mut IdeDesktopApp, ctx: &egui::Context, key: egui::Key) {
    frame(app, ctx, vec![event_key(key, true), event_key(key, false)]);
}

fn type_text(app: &mut IdeDesktopApp, ctx: &egui::Context, text: &str) {
    frame(app, ctx, vec![egui::Event::Text(text.into())]);
}

fn wait(app: &mut IdeDesktopApp, ctx: &egui::Context, predicate: impl Fn(&IdeDesktopApp) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        frame(app, ctx, Vec::new());
        if predicate(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "session did not reach checkpoint: {}",
            app.native_input_snapshot()
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn contains_value_at_key(value: &serde_json::Value, key: &str, expected: &str) -> bool {
    match value {
        serde_json::Value::Object(values) => {
            values.get(key).and_then(|v| v.as_str()) == Some(expected)
                || values
                    .values()
                    .any(|v| contains_value_at_key(v, key, expected))
        }
        serde_json::Value::Array(values) => values
            .iter()
            .any(|v| contains_value_at_key(v, key, expected)),
        _ => false,
    }
}

fn dialog(app: &IdeDesktopApp, title: &str) -> bool {
    contains_value_at_key(&app.native_input_snapshot(), "title", title)
}

fn fixture(source: &str) -> (IdeDesktopApp, egui::Context) {
    let mut app = IdeDesktopApp::default();
    app.editor.active_tab_mut().unwrap().content = source.into();
    app.state.layout = crate::ide::layout::WorkspaceLayout::Calculator;
    app.state.calc_input_focus = true;
    let ctx = egui::Context::default();
    // Match the measured G1 setting through the same Home input route as a
    // user; do not modify the program body or the engine's global defaults.
    app.handle_action(IdeAction::NativeHome, &ctx);
    submit_home(&mut app, &ctx, "HDigits:=0");
    assert_eq!(app.controller.session_snapshot().settings.hdigits, 0);
    app.handle_action(IdeAction::NativeCatalog, &ctx);
    (app, ctx)
}

fn run(app: &mut IdeDesktopApp, ctx: &egui::Context) {
    app.handle_action(IdeAction::RunProgram, ctx);
    wait(app, ctx, |a| a.controller.operation_active());
}

fn complete(app: &mut IdeDesktopApp, ctx: &egui::Context) {
    wait(app, ctx, |a| {
        !a.controller.operation_active()
            && a.native_compile.is_none()
            && !a.native_stopping
            && a.state.native.operation.is_none()
    });
}

fn returned(app: &mut IdeDesktopApp) -> String {
    match app.controller.outcome() {
        Some(WorkerOutcome::Success { value_str, .. }) => value_str.clone(),
        other => panic!("expected successful program return: {other:?}"),
    }
}

fn submit_home(app: &mut IdeDesktopApp, ctx: &egui::Context, text: &str) {
    type_text(app, ctx, text);
    press(app, ctx, egui::Key::Enter);
    complete(app, ctx);
}

fn baseconv_source() -> String {
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/baseconv/Baseconv_fixed.hpprgm"
    ))
    .replace("EXPORT Baseconv()", "EXPORT ZZT_BC24723()")
}

fn baseconv_destination(app: &mut IdeDesktopApp, ctx: &egui::Context) {
    wait(app, ctx, |a| dialog(a, "Number to convert"));
    type_text(app, ctx, "FF");
    press(app, ctx, egui::Key::Enter);
    wait(app, ctx, |a| dialog(a, "Source base"));
    type_text(app, ctx, "16");
    press(app, ctx, egui::Key::Enter);
    wait(app, ctx, |a| dialog(a, "Destination base"));
}

#[test]
fn session_baseconv_destination_escape_then_home_caller() {
    let (mut app, ctx) = fixture(&baseconv_source());
    app.handle_action(IdeAction::NativeHome, &ctx);
    app.refresh_native_catalog();
    type_text(&mut app, &ctx, "ZZT_BC24723()");
    press(&mut app, &ctx, egui::Key::Enter);
    baseconv_destination(&mut app, &ctx);
    press(&mut app, &ctx, egui::Key::Escape);
    complete(&mut app, &ctx);
    assert_eq!(returned(&mut app), "0");
    assert_eq!(app.state.native.view, NativeView::ProgramOutput);
    press(&mut app, &ctx, egui::Key::Escape);
    assert_eq!(app.state.native.view, NativeView::Home);
    assert!(app.controller.session_alive());
    submit_home(&mut app, &ctx, "2+3");
    assert_eq!(
        app.controller.session_snapshot().globals.get("Ans"),
        Some(&PortableValue::Real(ppl_math::Real::from_i64(5)))
    );
}

#[test]
fn session_baseconv_valid_after_cancel_uses_current_inputs() {
    let (mut app, ctx) = fixture(&baseconv_source());
    run(&mut app, &ctx);
    baseconv_destination(&mut app, &ctx);
    press(&mut app, &ctx, egui::Key::Escape);
    complete(&mut app, &ctx);
    press(&mut app, &ctx, egui::Key::Escape);
    assert_eq!(app.state.native.view, NativeView::Catalog);
    press(&mut app, &ctx, egui::Key::Enter);
    baseconv_destination(&mut app, &ctx);
    type_text(&mut app, &ctx, "10");
    press(&mut app, &ctx, egui::Key::Enter);
    wait(&mut app, &ctx, |a| dialog(a, "Fractional precision"));
    type_text(&mut app, &ctx, "10");
    press(&mut app, &ctx, egui::Key::Enter);
    complete(&mut app, &ctx);
    assert_eq!(
        app.controller.session_snapshot().last_value,
        Some(PortableValue::String("255".into()))
    );
    assert!(
        app.controller
            .shared
            .terminal_lines
            .lock()
            .unwrap()
            .iter()
            .any(|s| s == "Final Result: 255")
    );
}

#[test]
fn session_getkey_handles_one_escape_without_leaking_into_following_wait() {
    let source = "EXPORT TEST() BEGIN LOCAL k:=-1,w; REPEAT k:=GETKEY; UNTIL k<>-1; PRINT(\"after key\"); w:=WAIT(0); RETURN {k,w}; END;";
    let (mut app, ctx) = fixture(source);
    run(&mut app, &ctx);
    press(&mut app, &ctx, egui::Key::Escape);
    wait(&mut app, &ctx, |a| {
        a.controller
            .shared
            .terminal_lines
            .lock()
            .unwrap()
            .iter()
            .any(|s| s == "after key")
    });
    for _ in 0..5 {
        frame(&mut app, &ctx, Vec::new());
    }
    assert!(
        app.controller.operation_active(),
        "duplicate Esc escaped the following WAIT"
    );
    press(&mut app, &ctx, egui::Key::Enter);
    complete(&mut app, &ctx);
    assert_eq!(returned(&mut app), "{4,30}");
}

#[test]
fn session_wait_zero_receives_escape_as_program_input() {
    let (mut app, ctx) = fixture("EXPORT TEST() BEGIN RETURN WAIT(0); END;");
    run(&mut app, &ctx);
    press(&mut app, &ctx, egui::Key::Escape);
    complete(&mut app, &ctx);
    assert_eq!(returned(&mut app), "4");
    assert_eq!(app.state.native.view, NativeView::ProgramOutput);
}

#[test]
fn session_choose_cancellation_does_not_also_close_next_message() {
    let source = "EXPORT TEST() BEGIN LOCAL c:=1,ok,m; ok:=CHOOSE(c,\"Session choice\",{\"A\",\"B\"}); m:=MSGBOX(\"Second independent modal\"); RETURN {ok,c,m}; END;";
    let (mut app, ctx) = fixture(source);
    run(&mut app, &ctx);
    wait(&mut app, &ctx, |a| dialog(a, "Session choice"));
    press(&mut app, &ctx, egui::Key::Escape);
    wait(&mut app, &ctx, |a| {
        contains_value_at_key(
            &a.native_input_snapshot(),
            "text",
            "Second independent modal",
        )
    });
    assert!(app.controller.operation_active());
    press(&mut app, &ctx, egui::Key::Enter);
    complete(&mut app, &ctx);
    assert_eq!(returned(&mut app), "{0,1,1}");
}

#[test]
fn session_explicit_stop_interrupts_busy_operation_then_home_still_evaluates() {
    let (mut app, ctx) = fixture("EXPORT TEST() BEGIN LOCAL x:=0; WHILE 1 DO x:=x+1; END; END;");
    run(&mut app, &ctx);
    // Local Native policy: Esc is program input while computing. This does
    // not establish how the physical G1 interrupts compute-bound programs.
    press(&mut app, &ctx, egui::Key::Escape);
    for _ in 0..5 {
        frame(&mut app, &ctx, Vec::new());
    }
    assert!(
        app.controller.operation_active(),
        "Esc unexpectedly became unconditional Stop"
    );
    frame(&mut app, &ctx, vec![event_key(egui::Key::ArrowUp, true)]);
    assert!(!app.controller.shared.held_keys.lock().unwrap().is_empty());
    app.handle_action(IdeAction::StopProgram, &ctx);
    // Worker completion and UI lifecycle completion are separate. Wait for
    // the app to consume the outcome, not just the worker's atomic flag.
    complete(&mut app, &ctx);
    assert!(
        app.state.native.operation.is_none(),
        "Stop retained a pending Native operation"
    );
    assert_eq!(
        app.state
            .native
            .last_outcome
            .as_ref()
            .and_then(|v| v.get("status"))
            .and_then(|v| v.as_str()),
        Some("interrupted")
    );
    assert!(app.controller.shared.held_keys.lock().unwrap().is_empty());
    assert!(app.controller.shared.key_queue.lock().unwrap().is_empty());
    assert!(app.controller.session_alive());
    app.handle_action(IdeAction::NativeHome, &ctx);
    submit_home(&mut app, &ctx, "2+3");
    assert_eq!(
        app.controller.session_snapshot().last_value,
        Some(PortableValue::Real(ppl_math::Real::from_i64(5)))
    );
}

#[test]
fn session_home_ans_and_catalog_rerun_keep_session_and_refresh_source() {
    let (mut app, ctx) = fixture("EXPORT TEST() BEGIN RETURN 42; END;");
    app.handle_action(IdeAction::NativeHome, &ctx);
    submit_home(&mut app, &ctx, "2+3");
    submit_home(&mut app, &ctx, "Ans*2");
    assert_eq!(
        app.controller.session_snapshot().last_value,
        Some(PortableValue::Real(ppl_math::Real::from_i64(10)))
    );
    assert_eq!(
        app.state
            .native
            .history
            .iter()
            .rev()
            .take(2)
            .map(|h| h.expression.as_str())
            .collect::<Vec<_>>(),
        vec!["Ans*2", "2+3"]
    );
    app.handle_action(IdeAction::NativeCatalog, &ctx);
    press(&mut app, &ctx, egui::Key::Enter);
    complete(&mut app, &ctx);
    assert_eq!(returned(&mut app), "42");
    press(&mut app, &ctx, egui::Key::Escape);
    assert_eq!(app.state.native.view, NativeView::Catalog);
    app.editor.active_tab_mut().unwrap().content = "EXPORT TEST() BEGIN RETURN 84; END;".into();
    app.refresh_native_catalog();
    press(&mut app, &ctx, egui::Key::Enter);
    complete(&mut app, &ctx);
    assert_eq!(returned(&mut app), "84");
    let snapshot = app.controller.session_snapshot();
    assert_eq!(
        snapshot.last_value,
        Some(PortableValue::Real(ppl_math::Real::from_i64(84)))
    );
    assert_eq!(
        snapshot.globals.get("Ans"),
        Some(&PortableValue::Real(ppl_math::Real::from_i64(10)))
    );
}

#[test]
fn session_ide_modal_owns_escape_and_stale_program_text_does_not_reach_home() {
    let (mut app, ctx) = fixture(
        "EXPORT TEST() BEGIN LOCAL n:=7,ok; ok:=INPUT(n,\"Epoch input\",\"N\"); RETURN {ok,n}; END;",
    );
    run(&mut app, &ctx);
    wait(&mut app, &ctx, |a| dialog(a, "Epoch input"));
    app.dialogs.show_settings = true;
    press(&mut app, &ctx, egui::Key::Escape);
    assert!(app.controller.operation_active());
    assert!(dialog(&app, "Epoch input"));
    let epoch = app.state.native.input_epoch;
    frame(
        &mut app,
        &ctx,
        vec![
            event_key(egui::Key::Escape, true),
            egui::Event::Text("9".into()),
            event_key(egui::Key::Escape, false),
        ],
    );
    complete(&mut app, &ctx);
    assert_eq!(returned(&mut app), "{0,7}");
    press(&mut app, &ctx, egui::Key::Escape);
    app.handle_action(IdeAction::NativeHome, &ctx);
    assert!(app.state.native.input_epoch > epoch);
    assert!(app.controller.shared.held_keys.lock().unwrap().is_empty());
    assert!(app.controller.shared.key_queue.lock().unwrap().is_empty());
    // A normal printable PC key yields both Key and Text in one frame.
    // The central route must not insert the same digit twice.
    frame(
        &mut app,
        &ctx,
        vec![
            event_key(egui::Key::Num2, true),
            egui::Event::Text("2".into()),
            event_key(egui::Key::Num2, false),
        ],
    );
    type_text(&mut app, &ctx, "+3");
    press(&mut app, &ctx, egui::Key::Enter);
    complete(&mut app, &ctx);
    assert_eq!(
        app.controller.session_snapshot().last_value,
        Some(PortableValue::Real(ppl_math::Real::from_i64(5)))
    );
}

#[test]
fn session_explicit_editor_focus_keeps_typing_and_escape_out_of_input_dialog() {
    let (mut app, ctx) = fixture(
        "EXPORT TEST() BEGIN LOCAL n:=7,ok; ok:=INPUT(n,\"Editor isolation\",\"N\"); RETURN {ok,n}; END;",
    );
    run(&mut app, &ctx);
    wait(&mut app, &ctx, |a| dialog(a, "Editor isolation"));
    app.handle_action(
        IdeAction::SetLayout(crate::ide::layout::WorkspaceLayout::Split),
        &ctx,
    );
    app.state.calc_input_focus = false;
    let editor_id = crate::ide::editor::view::edit_id(app.editor.active_tab().unwrap().id);
    ctx.memory_mut(|m| m.request_focus(editor_id));
    type_text(&mut app, &ctx, "99");
    press(&mut app, &ctx, egui::Key::Escape);
    assert!(app.controller.operation_active());
    assert!(dialog(&app, "Editor isolation"));
    assert!(app.controller.shared.key_queue.lock().unwrap().is_empty());
    app.focus_native_calculator(&ctx);
    press(&mut app, &ctx, egui::Key::Escape);
    complete(&mut app, &ctx);
    assert_eq!(returned(&mut app), "{0,7}");
}

#[test]
fn session_focus_loss_releases_held_keys_without_deleting_accepted_input() {
    let (mut app, ctx) = fixture("EXPORT TEST() BEGIN LOCAL x:=0; WHILE 1 DO x:=x+1; END; END;");
    run(&mut app, &ctx);
    // A computing program cannot consume this queued key, which makes the
    // focus-loss retention assertion independent of worker scheduling.
    frame(&mut app, &ctx, vec![event_key(egui::Key::ArrowUp, true)]);
    let queued = app.controller.shared.key_queue.lock().unwrap().len();
    assert!(queued > 0);
    assert!(!app.controller.shared.held_keys.lock().unwrap().is_empty());
    let epoch = app
        .controller
        .shared
        .input_epoch
        .load(std::sync::atomic::Ordering::Acquire);
    let _ = ctx.run(
        egui::RawInput {
            focused: false,
            ..Default::default()
        },
        |ctx| {
            app.forward_calc_keys(ctx);
        },
    );
    assert!(app.controller.shared.held_keys.lock().unwrap().is_empty());
    assert_eq!(
        app.controller.shared.key_queue.lock().unwrap().len(),
        queued
    );
    assert_eq!(
        app.controller
            .shared
            .input_epoch
            .load(std::sync::atomic::Ordering::Acquire),
        epoch
    );
    app.handle_action(IdeAction::StopProgram, &ctx);
    complete(&mut app, &ctx);
}

#[test]
fn session_late_stop_preserves_a_real_completed_success() {
    let (mut app, ctx) = fixture("EXPORT TEST() BEGIN RETURN 42; END;");
    app.handle_action(IdeAction::RunProgram, &ctx);
    let deadline = Instant::now() + Duration::from_secs(5);
    // Deliberately settle compilation and the worker without polling the app
    // completion handler, reproducing Stop racing a finished operation.
    while app.native_compile.is_some() {
        app.poll_native_compile();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    while app.controller.operation_active() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    app.handle_action(IdeAction::StopProgram, &ctx);
    complete(&mut app, &ctx);
    assert_eq!(returned(&mut app), "42");
    assert_eq!(app.state.status_message, "Program finished");
    assert_eq!(app.state.native.view, NativeView::ProgramOutput);
    assert!(app.state.native.error.is_none());
}

#[test]
fn session_compile_rejection_retires_ownership_and_allows_home_then_corrected_run() {
    let (mut app, ctx) = fixture("EXPORT TEST() BEGIN LOCAL x; x:=; END;");
    app.handle_action(IdeAction::RunProgram, &ctx);
    wait(&mut app, &ctx, |a| a.native_compile.is_none());
    assert!(app.state.native.operation.is_none());
    assert!(!app.controller.operation_active());
    assert_eq!(app.state.native.view, NativeView::Catalog);
    assert_eq!(
        app.state.native.last_outcome.as_ref().unwrap()["status"],
        "blocked"
    );
    app.handle_action(IdeAction::NativeHome, &ctx);
    submit_home(&mut app, &ctx, "2+3");
    assert_eq!(
        app.controller.session_snapshot().last_value,
        Some(PortableValue::Real(ppl_math::Real::from_i64(5)))
    );
    app.editor.active_tab_mut().unwrap().content = "EXPORT TEST() BEGIN RETURN 42; END;".into();
    app.handle_action(IdeAction::NativeCatalog, &ctx);
    press(&mut app, &ctx, egui::Key::Enter);
    complete(&mut app, &ctx);
    assert_eq!(returned(&mut app), "42");
}

#[test]
fn session_stop_after_handled_error_does_not_replay_completion_or_lose_the_view() {
    let (mut app, ctx) = fixture("EXPORT TEST() BEGIN RETURN 1/0; END;");
    run(&mut app, &ctx);
    complete(&mut app, &ctx);
    assert_eq!(app.state.native.view, NativeView::Error);
    let history = app.console.full_text();
    let outcome = app.state.native.last_outcome.clone();
    app.handle_action(IdeAction::StopProgram, &ctx);
    frame(&mut app, &ctx, Vec::new());
    assert_eq!(app.console.full_text(), history);
    assert_eq!(app.state.native.last_outcome, outcome);
    assert_eq!(app.state.native.view, NativeView::Error);
    press(&mut app, &ctx, egui::Key::Escape);
    assert_eq!(app.state.native.view, NativeView::Catalog);
}

#[test]
fn session_same_basename_and_source_have_distinct_module_state_per_file() {
    let source = "LOCAL count:=0; EXPORT TEST() BEGIN count:=count+1; RETURN count; END;";
    let (mut app, ctx) = fixture(source);
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let folder = std::env::temp_dir().join(format!(
        "ppl-session-module-files-{}-{unique}",
        std::process::id()
    ));
    let first = folder.join("a/same.hpprgm");
    let second = folder.join("b/same.hpprgm");
    for path in [&first, &second] {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
    app.open_file(first.clone());
    app.handle_action(IdeAction::RunProgram, &ctx);
    complete(&mut app, &ctx);
    assert_eq!(returned(&mut app), "1");
    app.handle_action(IdeAction::RunProgram, &ctx);
    complete(&mut app, &ctx);
    assert_eq!(returned(&mut app), "2");
    app.open_file(second);
    app.handle_action(IdeAction::RunProgram, &ctx);
    complete(&mut app, &ctx);
    assert_eq!(returned(&mut app), "1");
    app.open_file(first);
    app.handle_action(IdeAction::RunProgram, &ctx);
    complete(&mut app, &ctx);
    assert_eq!(returned(&mut app), "3");
    std::fs::remove_dir_all(folder).unwrap();
}
