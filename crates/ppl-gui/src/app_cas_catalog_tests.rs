//! Real-Giac CAS catalogs through production egui controls and async app workers.
//! Enabling giac-integration requires the actual separately installed helper;
//! unavailable helpers fail these tests rather than silently skipping them.
#![cfg(feature = "giac-integration")]
use super::*;
use crate::ide::native_tools::Page;
use std::sync::atomic::AtomicBool;

fn frame(
    app: &mut IdeDesktopApp,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    ctx.run(
        egui::RawInput {
            focused: true,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280., 900.),
            )),
            events,
            ..Default::default()
        },
        |ctx| {
            app.poll_execution();
            app.poll_native_tools(ctx);
            app.forward_calc_keys(ctx);
            app.consume_native_tool(ctx);
            app.render_native_tools(ctx);
        },
    )
}
fn text_rect(shape: &egui::epaint::Shape, predicate: &impl Fn(&str) -> bool) -> Option<egui::Rect> {
    match shape {
        egui::epaint::Shape::Text(t) if predicate(t.galley.text()) => {
            Some(t.galley.rect.translate(t.pos.to_vec2()))
        }
        egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(|s| text_rect(s, predicate)),
        _ => None,
    }
}
fn click_matching(app: &mut IdeDesktopApp, ctx: &egui::Context, predicate: impl Fn(&str) -> bool) {
    let mut output = frame(app, ctx, vec![]);
    let mut rect = None;
    for _ in 0..3 {
        rect = output.shapes.iter().find_map(|s| {
            text_rect(&s.shape, &predicate).filter(|r| s.clip_rect.contains(r.center()))
        });
        if rect.is_some() {
            break;
        }
        output = frame(app, ctx, vec![]);
    }
    let pos = rect.expect("visible catalog control").center();
    frame(
        app,
        ctx,
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    frame(
        app,
        ctx,
        vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
}
fn click(app: &mut IdeDesktopApp, ctx: &egui::Context, label: &str) {
    click_matching(app, ctx, |text| text == label);
}
fn type_text(app: &mut IdeDesktopApp, ctx: &egui::Context, text: &str) {
    frame(app, ctx, vec![egui::Event::Text(text.into())]);
}
fn press_with(
    app: &mut IdeDesktopApp,
    ctx: &egui::Context,
    key: egui::Key,
    modifiers: egui::Modifiers,
) {
    frame(
        app,
        ctx,
        vec![
            egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            },
            egui::Event::Key {
                key,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers,
            },
        ],
    );
}
fn press(app: &mut IdeDesktopApp, ctx: &egui::Context, key: egui::Key) {
    press_with(app, ctx, key, egui::Modifiers::NONE);
}
fn wait(app: &mut IdeDesktopApp, ctx: &egui::Context, predicate: impl Fn(&IdeDesktopApp) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        frame(app, ctx, vec![]);
        if predicate(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "CAS catalog did not settle: {} / {:?}",
            app.state.status_message,
            app.native_tools.error
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn settled(app: &mut IdeDesktopApp, ctx: &egui::Context) {
    wait(app, ctx, |a| {
        a.cas_task.is_none() && !a.controller.running() && a.state.native.operation.is_none()
    });
    assert!(
        app.native_tools.error.is_none(),
        "real Giac operation failed: {:?}",
        app.native_tools.error
    );
}
fn base() -> (IdeDesktopApp, egui::Context) {
    let mut app = IdeDesktopApp::default();
    app.state.layout = crate::ide::layout::WorkspaceLayout::Calculator;
    app.state.calc_input_focus = true;
    app.display.last_display_rect =
        egui::Rect::from_min_size(egui::pos2(40., 40.), egui::vec2(640., 480.));
    (app, egui::Context::default())
}
fn seed(app: &mut IdeDesktopApp, ctx: &egui::Context) {
    app.handle_action(IdeAction::NativeHome, ctx);
    for expression in ["A:=7", "77"] {
        type_text(app, ctx, expression);
        press(app, ctx, egui::Key::Enter);
        settled(app, ctx);
    }
    app.route_native_press(KeyPress::code(crate::keymap::KEY_CAS), "keypad", ctx);
    app.consume_native_tool(ctx);
    assert!(app.state.native.cas_mode);
    type_text(app, ctx, "2+3");
    press(app, ctx, egui::Key::Enter);
    settled(app, ctx);
    assert_eq!(
        app.controller.session_snapshot().cas.last_value,
        Some(ppl_cas::CasValue::Integer("5".into()))
    );
    type_text(app, ctx, "sin(x)+7");
    press_with(app, ctx, egui::Key::ArrowLeft, egui::Modifiers::SHIFT);
    press_with(app, ctx, egui::Key::ArrowLeft, egui::Modifiers::SHIFT);
    app.native_tools.open(Page::Variables);
}
struct Preserved {
    answer: Option<ppl_cas::CasValue>,
    home_answer: ppl_vm::PortableValue,
    history: serde_json::Value,
    home_history: serde_json::Value,
    draft: String,
    selection: std::ops::Range<usize>,
    cursor: usize,
}
impl Preserved {
    fn capture(app: &IdeDesktopApp) -> Self {
        let s = app.controller.session_snapshot();
        Self {
            answer: s.cas.last_value,
            home_answer: s.globals["Ans"].clone(),
            history: serde_json::to_value(&app.state.native.history).unwrap(),
            home_history: serde_json::to_value(&app.native_tools.memory.home_history).unwrap(),
            draft: app.state.native.home_input.clone(),
            selection: app.state.native.editor.selection(),
            cursor: app.state.native.editor.cursor(),
        }
    }
    fn check(&self, app: &IdeDesktopApp) {
        let s = app.controller.session_snapshot();
        assert!(app.state.native.cas_mode);
        assert_eq!(s.cas.last_value, self.answer);
        assert_eq!(s.globals["Ans"], self.home_answer);
        assert_eq!(
            serde_json::to_value(&app.state.native.history).unwrap(),
            self.history
        );
        assert_eq!(
            serde_json::to_value(&app.native_tools.memory.home_history).unwrap(),
            self.home_history
        );
        assert_eq!(app.state.native.home_input, self.draft);
        assert_eq!(app.state.native.editor.text(), self.draft);
        assert_eq!(app.state.native.editor.selection(), self.selection);
        assert_eq!(app.state.native.editor.cursor(), self.cursor);
    }
}
fn store(app: &mut IdeDesktopApp, ctx: &egui::Context, name: &str, value: &str) {
    app.native_tools.memory.name = name.into();
    app.native_tools.memory.content = value.into();
    click(app, ctx, "Store");
    settled(app, ctx);
}
fn assert_symbolic_zero(app: &IdeDesktopApp, expression: &str) {
    let mut verifier = app.controller.session_snapshot().cas;
    let result = verifier
        .evaluate(
            &format!("simplify({expression})"),
            &Default::default(),
            &AtomicBool::new(false),
        )
        .expect("actual Giac semantic verification");
    assert_eq!(result, ppl_cas::CasValue::Integer("0".into()));
}

#[test]
fn cas_catalog_scope_clicks_store_symbolic_q_and_preserve_answers_history_editor() {
    let (mut app, ctx) = base();
    seed(&mut app, &ctx);
    let saved = Preserved::capture(&app);
    click(&mut app, &ctx, "CAS");
    assert_eq!(app.native_tools.page, Some(Page::CasVariables));
    store(&mut app, &ctx, "q", "x+1");
    assert!(
        app.controller
            .session_snapshot()
            .cas
            .bindings
            .contains_key("q")
    );
    assert!(!app.controller.session_snapshot().globals.contains_key("q"));
    assert_symbolic_zero(&app, "q-(x+1)");
    saved.check(&app);
    click(&mut app, &ctx, "Home");
    assert_eq!(app.native_tools.page, Some(Page::Variables));
    click_matching(&mut app, &ctx, |text| text.starts_with("A: "));
    assert_eq!(app.native_tools.memory.name, "A");
    assert_eq!(app.native_tools.memory.content, "7");
    saved.check(&app);
    click(&mut app, &ctx, "CAS");
    click_matching(&mut app, &ctx, |text| text.starts_with("q: "));
    assert_eq!(app.native_tools.memory.name, "q");
    assert_eq!(
        app.native_tools.memory.content,
        app.controller.session_snapshot().cas.bindings["q"]
    );
    saved.check(&app);
}
#[test]
fn cas_catalog_selected_expression_edit_cancel_commit_and_delete_are_transactional() {
    let (mut app, ctx) = base();
    seed(&mut app, &ctx);
    let saved = Preserved::capture(&app);
    click(&mut app, &ctx, "CAS");
    store(&mut app, &ctx, "q", "x+1");
    click_matching(&mut app, &ctx, |text| text.starts_with("q: "));
    let original = app.native_tools.memory.content.clone();
    click(&mut app, &ctx, &original);
    assert!(app.math_entry.is_some());
    press_with(&mut app, &ctx, egui::Key::A, egui::Modifiers::COMMAND);
    type_text(&mut app, &ctx, "x^2+3");
    press(&mut app, &ctx, egui::Key::Escape);
    assert!(app.math_entry.is_none());
    assert_eq!(app.native_tools.page, Some(Page::CasVariables));
    assert_eq!(app.native_tools.memory.name, "q");
    assert_eq!(app.native_tools.memory.content, original);
    assert_symbolic_zero(&app, "q-(x+1)");
    saved.check(&app);
    click(&mut app, &ctx, &original);
    press_with(&mut app, &ctx, egui::Key::A, egui::Modifiers::COMMAND);
    type_text(&mut app, &ctx, "x^2+3");
    press(&mut app, &ctx, egui::Key::Enter);
    assert_eq!(app.native_tools.memory.content, "x^2+3");
    assert_symbolic_zero(&app, "q-(x+1)");
    saved.check(&app);
    click(&mut app, &ctx, "Store");
    settled(&mut app, &ctx);
    assert_symbolic_zero(&app, "q-(x^2+3)");
    saved.check(&app);
    click(&mut app, &ctx, "Delete");
    settled(&mut app, &ctx);
    assert!(
        !app.controller
            .session_snapshot()
            .cas
            .bindings
            .contains_key("q")
    );
    assert!(app.native_tools.memory.name.is_empty());
    assert!(app.native_tools.memory.content.is_empty());
    saved.check(&app);
}
#[test]
fn cas_catalog_named_uppercase_assignment_updates_home_without_replacing_cas_answer() {
    let (mut app, ctx) = base();
    seed(&mut app, &ctx);
    let saved = Preserved::capture(&app);
    click(&mut app, &ctx, "CAS");
    store(&mut app, &ctx, "q", "x+1");
    store(&mut app, &ctx, "A", "14");
    let s = app.controller.session_snapshot();
    assert_eq!(s.globals["A"].display(&s.settings), "14");
    assert!(!s.cas.bindings.contains_key("A"));
    assert_symbolic_zero(&app, "q-(x+1)");
    saved.check(&app);
    click(&mut app, &ctx, "Home");
    click_matching(&mut app, &ctx, |text| text.starts_with("A: "));
    assert_eq!(app.native_tools.memory.content, "14");
    saved.check(&app);
}
#[test]
fn cas_catalog_busy_and_invalid_names_leave_saved_cas_state_and_editor_unchanged() {
    let (mut app, ctx) = base();
    seed(&mut app, &ctx);
    let saved = Preserved::capture(&app);
    click(&mut app, &ctx, "CAS");
    store(&mut app, &ctx, "q", "x+1");
    let bindings = app.controller.session_snapshot().cas.bindings;
    let owner = app.controller.reserve_native_operation().unwrap();
    app.native_tools.memory.name = "q".into();
    app.native_tools.memory.content = "99".into();
    click(&mut app, &ctx, "Store");
    assert_eq!(
        app.native_tools.error.as_deref(),
        Some("Calculator is busy")
    );
    assert_eq!(app.controller.session_snapshot().cas.bindings, bindings);
    drop(owner);
    saved.check(&app);
    app.native_tools.memory.name = "Ans".into();
    click(&mut app, &ctx, "Store");
    assert_eq!(
        app.native_tools.error.as_deref(),
        Some("Enter a valid CAS variable name")
    );
    assert_eq!(app.controller.session_snapshot().cas.bindings, bindings);
    saved.check(&app);
}

/// Two child test processes use the real startup loader and independently exit;
/// the parent never changes process-global profile environment during parallel tests.
#[test]
fn cas_catalog_profile_restarts_through_startup() {
    const PHASE: &str = "PPL_CAS_CATALOG_TEST_RESTART_PHASE";
    if let Ok(phase) = std::env::var(PHASE) {
        let (mut app, ctx) = base();
        app.initialize_native_memory();
        assert!(app.profile_problem.is_none(), "{:?}", app.profile_problem);
        match phase.as_str() {
            "write" => {
                seed(&mut app, &ctx);
                let saved = Preserved::capture(&app);
                click(&mut app, &ctx, "CAS");
                store(&mut app, &ctx, "q", "x+1");
                store(&mut app, &ctx, "A", "14");
                saved.check(&app);
                click_matching(&mut app, &ctx, |text| text.starts_with("q: "));
                app.persist_native_ui();
                app.controller.native_profile().unwrap().flush().unwrap();
            }
            "read" => {
                let snapshot = app.controller.session_snapshot();
                assert_eq!(snapshot.globals["A"].display(&snapshot.settings), "14");
                assert_eq!(snapshot.globals["Ans"].display(&snapshot.settings), "77");
                assert_eq!(
                    snapshot.cas.last_value,
                    Some(ppl_cas::CasValue::Integer("5".into()))
                );
                assert!(snapshot.cas.bindings.contains_key("q"));
                assert!(app.state.native.cas_mode);
                assert_eq!(app.state.native.home_input, "sin(x)+7");
                assert_eq!(app.state.native.history.len(), 1);
                assert_eq!(app.state.native.history[0].expression, "2+3");
                assert_eq!(app.native_tools.memory.name, "q");
                assert_symbolic_zero(&app, "q-(x+1)");
                // Present the restored CAS entry without changing its environment first:
                // Alpha's default must already have been restored by startup.
                app.state.native.set_view(NativeView::Home);
                app.route_native_press(KeyPress::code(crate::keymap::KEY_ALPHA), "keypad", &ctx);
                app.route_native_press(KeyPress::code(crate::keymap::KEY_SIN), "keypad", &ctx);
                assert_eq!(app.state.native.home_input, "sin(x)+7*g");
                app.native_tools.open(Page::Variables);
                click(&mut app, &ctx, "CAS");
                click_matching(&mut app, &ctx, |text| text.starts_with("q: "));
                assert_eq!(app.native_tools.memory.content, snapshot.cas.bindings["q"]);
                click(&mut app, &ctx, "Delete");
                settled(&mut app, &ctx);
                assert!(
                    !app.controller
                        .session_snapshot()
                        .cas
                        .bindings
                        .contains_key("q")
                );
            }
            _ => panic!("unknown restart phase {phase}"),
        }
        return;
    }
    let directory = std::env::temp_dir().join(format!(
        "ppl-cas-catalog-restart-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let module = module_path!().split_once("::").unwrap().1;
    let name = format!("{module}::cas_catalog_profile_restarts_through_startup");
    for phase in ["write", "read"] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &name, "--nocapture"])
            .env(PHASE, phase)
            .env("PPL_NATIVE_PROFILE", &directory)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "restart {phase} failed; profile preserved at {}\n{}\n{}",
            directory.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    std::fs::remove_dir_all(directory).unwrap();
}
