//! Standalone calculator catalogs through real egui requests and VM workers.
//! No physical calculator connection or firmware writes are involved.
use super::*;
use crate::ide::native_tools::{Page, Request};

fn frame(
    app: &mut IdeDesktopApp,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let input = egui::RawInput {
        focused: true,
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1280., 900.),
        )),
        events,
        ..Default::default()
    };
    ctx.run(input, |ctx| {
        app.poll_execution();
        app.forward_calc_keys(ctx);
        app.render_native_tools(ctx);
    })
}
fn fixture(page: Page) -> (IdeDesktopApp, egui::Context) {
    let mut app = IdeDesktopApp::default();
    app.state.layout = crate::ide::layout::WorkspaceLayout::Calculator;
    app.state.calc_input_focus = true;
    app.state.native.set_view(NativeView::Home);
    app.native_tools.open(page);
    app.display.last_display_rect =
        egui::Rect::from_min_size(egui::pos2(40., 40.), egui::vec2(640., 480.));
    (app, egui::Context::default())
}
fn text_rect(shape: &egui::epaint::Shape, text: &str) -> Option<egui::Rect> {
    match shape {
        egui::epaint::Shape::Text(t) if t.galley.text() == text => {
            Some(t.galley.rect.translate(t.pos.to_vec2()))
        }
        egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(|s| text_rect(s, text)),
        _ => None,
    }
}
fn click(app: &mut IdeDesktopApp, ctx: &egui::Context, label: &str) {
    let mut output = frame(app, ctx, vec![]);
    let mut rect = None;
    for _ in 0..3 {
        rect = output
            .shapes
            .iter()
            .find_map(|s| text_rect(&s.shape, label).filter(|r| s.clip_rect.contains(r.center())));
        if rect.is_some() {
            break;
        }
        output = frame(app, ctx, vec![]);
    }
    let pos = rect
        .unwrap_or_else(|| panic!("No visible button {label}"))
        .center();
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
fn wait(app: &mut IdeDesktopApp, ctx: &egui::Context, predicate: impl Fn(&IdeDesktopApp) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        frame(app, ctx, vec![]);
        if predicate(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "catalog did not complete: {}",
            app.state.status_message
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn catalog_save_selects_the_actual_saved_object_for_subsequent_rename() {
    let (mut app, ctx) = fixture(Page::Notes);
    app.native_tools.memory.name = "FIRST".into();
    app.native_tools.memory.content = "standalone note".into();
    click(&mut app, &ctx, "Save");
    assert_eq!(
        app.controller.session_snapshot().notes["FIRST"],
        "standalone note"
    );
    assert_eq!(app.native_tools.memory.original_name, "FIRST");
    app.native_tools.memory.name = "SECOND".into();
    click(&mut app, &ctx, "Rename");
    assert_eq!(app.native_tools.memory.original_name, "SECOND");
    assert!(
        !app.controller
            .session_snapshot()
            .notes
            .contains_key("FIRST")
    );
    app.native_tools.memory.name = "THIRD".into();
    click(&mut app, &ctx, "Rename");
    assert!(
        app.controller
            .session_snapshot()
            .notes
            .contains_key("THIRD")
    );
    assert!(
        !app.controller
            .session_snapshot()
            .notes
            .contains_key("SECOND")
    );
}
#[test]
fn catalog_async_store_errors_are_visible_after_an_earlier_completed_operation() {
    let (mut app, ctx) = fixture(Page::Variables);
    app.state.native.outcome_handled = true;
    app.native_tools.memory.name = "VALUE".into();
    app.native_tools.memory.content = "1/0".into();
    click(&mut app, &ctx, "Store");
    wait(&mut app, &ctx, |a| {
        !a.controller.running() && a.catalog_task.is_none()
    });
    assert_eq!(
        app.native_tools.error.as_deref(),
        Some("Error: Division by 0")
    );
    assert!(
        !app.controller
            .session_snapshot()
            .globals
            .contains_key("VALUE")
    );
    assert_eq!(app.native_tools.page, Some(Page::Variables));
    assert_eq!(app.state.native.view, NativeView::Home);
    assert!(app.state.native.history.is_empty());
}

fn type_text(app: &mut IdeDesktopApp, ctx: &egui::Context, text: &str) {
    frame(app, ctx, vec![egui::Event::Text(text.into())]);
}
fn press(app: &mut IdeDesktopApp, ctx: &egui::Context, key: egui::Key) {
    frame(
        app,
        ctx,
        vec![
            egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::Key {
                key,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
}
fn pointer_dialog(app: &IdeDesktopApp, x: i64, y: i64) {
    app.controller
        .shared
        .push_dialog_event(crate::dialog::DialogEvent::Pointer { x, y });
}
fn wait_dialog(app: &mut IdeDesktopApp, ctx: &egui::Context) {
    wait(app, ctx, |a| {
        a.controller.shared.active_dialog.lock().unwrap().is_some()
    });
}
fn complete(app: &mut IdeDesktopApp, ctx: &egui::Context) {
    wait(app, ctx, |a| {
        a.catalog_task.is_none() && !a.controller.running()
    });
}

#[test]
fn catalog_list_creation_edit_cancel_and_store_keep_home_history_and_ans() {
    let (mut app, ctx) = fixture(Page::Lists);
    app.state.native.outcome_handled = true;
    app.native_tools.memory.name = "MYLIST".into();
    app.controller
        .mutate_memory(|m| {
            m.set_variable(
                "Ans",
                ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(42)),
            )
            .map_err(|e| e.to_string())
        })
        .unwrap();
    click(&mut app, &ctx, "Open editor / Create");
    wait_dialog(&mut app, &ctx);
    pointer_dialog(&app, 20, 202);
    type_text(&mut app, &ctx, "7");
    pointer_dialog(&app, 20, 202);
    type_text(&mut app, &ctx, "8");
    pointer_dialog(&app, 40, 227);
    complete(&mut app, &ctx);
    let snapshot = app.controller.session_snapshot();
    assert_eq!(
        snapshot.globals["MYLIST"].display(&snapshot.settings),
        "{7,8}"
    );
    assert_eq!(snapshot.globals["Ans"].display(&snapshot.settings), "42");
    assert_eq!(app.native_tools.memory.content, "{7,8}");
    assert_eq!(app.native_tools.page, Some(Page::Lists));
    assert_eq!(app.state.native.view, NativeView::Home);
    assert!(app.state.native.history.is_empty());
    click(&mut app, &ctx, "Open editor / Create");
    wait_dialog(&mut app, &ctx);
    type_text(&mut app, &ctx, "999");
    pointer_dialog(&app, 140, 227);
    complete(&mut app, &ctx);
    let snapshot = app.controller.session_snapshot();
    assert_eq!(
        snapshot.globals["MYLIST"].display(&snapshot.settings),
        "{7,8}"
    );
    assert!(app.native_tools.error.is_none());
}

#[test]
fn catalog_matrix_dimensions_cells_and_cancel_have_real_memory_writeback() {
    let (mut app, ctx) = fixture(Page::Matrices);
    app.native_tools.memory.name = "MYMATRIX".into();
    click(&mut app, &ctx, "Open editor / Create");
    wait_dialog(&mut app, &ctx);
    pointer_dialog(&app, 250, 227);
    type_text(&mut app, &ctx, "2");
    press(&mut app, &ctx, egui::Key::ArrowDown);
    type_text(&mut app, &ctx, "3");
    press(&mut app, &ctx, egui::Key::Enter);
    type_text(&mut app, &ctx, "9");
    press(&mut app, &ctx, egui::Key::Enter);
    type_text(&mut app, &ctx, "10");
    pointer_dialog(&app, 40, 227);
    complete(&mut app, &ctx);
    let snapshot = app.controller.session_snapshot();
    let ppl_vm::PortableValue::Matrix {
        rows, cols, data, ..
    } = &snapshot.globals["MYMATRIX"]
    else {
        panic!("matrix missing")
    };
    assert_eq!((*rows, *cols), (2, 3));
    assert_eq!(data[0].display(&snapshot.settings), "9");
    assert_eq!(data[1].display(&snapshot.settings), "10");
    assert_eq!(app.native_tools.page, Some(Page::Matrices));
    assert!(app.state.native.history.is_empty());
    click(&mut app, &ctx, "Open editor / Create");
    wait_dialog(&mut app, &ctx);
    type_text(&mut app, &ctx, "123");
    pointer_dialog(&app, 140, 227);
    complete(&mut app, &ctx);
    assert_eq!(
        app.controller.session_snapshot().globals["MYMATRIX"],
        snapshot.globals["MYMATRIX"]
    );
}

#[test]
fn catalog_create_is_explicit_and_cancel_only_discards_editor_changes() {
    for (page, matrix) in [(Page::Lists, false), (Page::Matrices, true)] {
        let (mut app, ctx) = fixture(page);
        app.native_tools.memory.name = "CREATED".into();
        click(&mut app, &ctx, "Open editor / Create");
        wait_dialog(&mut app, &ctx);
        pointer_dialog(&app, 140, 227);
        complete(&mut app, &ctx);
        let snapshot = app.controller.session_snapshot();
        if matrix {
            assert!(matches!(
                snapshot.globals["CREATED"],
                ppl_vm::PortableValue::Matrix {
                    rows: 1,
                    cols: 1,
                    ..
                }
            ));
        } else {
            assert_eq!(
                snapshot.globals["CREATED"],
                ppl_vm::PortableValue::List(vec![])
            );
        }
        assert!(app.native_tools.error.is_none());
        assert_eq!(app.native_tools.page, Some(page));
    }
}

#[test]
fn catalog_on_interrupts_actual_edit_and_wait_workers_then_allows_reuse() {
    let (mut app, ctx) = fixture(Page::Matrices);
    app.native_tools.memory.name = "SAFE".into();
    click(&mut app, &ctx, "Open editor / Create");
    wait_dialog(&mut app, &ctx);
    type_text(&mut app, &ctx, "99");
    press(&mut app, &ctx, egui::Key::F12);
    complete(&mut app, &ctx);
    let snapshot = app.controller.session_snapshot();
    assert_eq!(
        snapshot.globals["SAFE"].display(&snapshot.settings),
        "[[0]]"
    );
    assert!(
        app.controller
            .shared
            .active_dialog
            .lock()
            .unwrap()
            .is_none()
    );
    assert!(app.native_tools.error.is_none());
    assert!(!app.native_stopping);
    app.native_tools.open(Page::Variables);
    app.native_tools.memory.name = "VALUE".into();
    app.native_tools.memory.content = "WAIT(999)".into();
    click(&mut app, &ctx, "Store");
    wait(&mut app, &ctx, |a| a.controller.running());
    press(&mut app, &ctx, egui::Key::F12);
    complete(&mut app, &ctx);
    assert!(
        !app.controller
            .session_snapshot()
            .globals
            .contains_key("VALUE")
    );
    assert!(app.native_tools.error.is_none());
    assert!(app.state.native.history.is_empty());
    app.native_tools.memory.content = "6*7".into();
    click(&mut app, &ctx, "Store");
    complete(&mut app, &ctx);
    let s = app.controller.session_snapshot();
    assert_eq!(s.globals["VALUE"].display(&s.settings), "42");
    assert_eq!(app.native_tools.page, Some(Page::Variables));
}

#[test]
fn catalog_busy_profile_and_type_errors_do_not_mutate_objects() {
    let (mut app, ctx) = fixture(Page::Notes);
    let owner = app.controller.reserve_native_operation().unwrap();
    app.native_tools.memory.name = "NOTE".into();
    app.native_tools.memory.content = "draft".into();
    click(&mut app, &ctx, "Save");
    assert!(
        app.native_tools
            .error
            .as_ref()
            .is_some_and(|e| e.contains("executing"))
    );
    assert!(app.controller.session_snapshot().notes.is_empty());
    assert!(app.native_tools.memory.original_name.is_empty());
    drop(owner);
    app.profile_problem = Some("Profile is locked; memory is unsaved".into());
    click(&mut app, &ctx, "Save");
    assert_eq!(app.native_tools.error, app.profile_problem);
    assert!(app.controller.session_snapshot().notes.is_empty());
    app.profile_problem = None;
    click(&mut app, &ctx, "Save");
    assert_eq!(app.native_tools.memory.original_name, "NOTE");
    app.native_tools.open(Page::Lists);
    app.native_tools.memory.name = "WRONG".into();
    app.controller
        .mutate_memory(|m| {
            m.set_variable(
                "WRONG",
                ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(7)),
            )
            .map_err(|e| e.to_string())
        })
        .unwrap();
    click(&mut app, &ctx, "Open editor / Create");
    assert_eq!(
        app.native_tools.error.as_deref(),
        Some("Object has a different type")
    );
    assert!(app.catalog_task.is_none());
    assert!(!app.controller.running());
}

#[test]
fn catalog_failed_rename_preserves_source_target_and_selection_then_delete_clears_it() {
    let (mut app, ctx) = fixture(Page::Notes);
    for (name, text) in [("SOURCE", "source"), ("TAKEN", "target")] {
        app.handle_native_tool_request(Request::SaveNote(name.into(), text.into()), &ctx);
    }
    app.native_tools.memory.original_name = "SOURCE".into();
    app.native_tools.memory.name = "TAKEN".into();
    app.native_tools.memory.content = "unsaved draft".into();
    click(&mut app, &ctx, "Rename");
    assert!(app.native_tools.error.is_some());
    assert_eq!(app.native_tools.memory.original_name, "SOURCE");
    let s = app.controller.session_snapshot();
    assert_eq!(s.notes["SOURCE"], "source");
    assert_eq!(s.notes["TAKEN"], "target");
    app.native_tools.memory.name = "RENAMED".into();
    click(&mut app, &ctx, "Rename");
    assert_eq!(app.native_tools.memory.original_name, "RENAMED");
    assert_eq!(app.native_tools.memory.content, "unsaved draft");
    click(&mut app, &ctx, "Delete");
    assert!(
        !app.controller
            .session_snapshot()
            .notes
            .contains_key("RENAMED")
    );
    assert!(app.native_tools.memory.name.is_empty());
    assert!(app.native_tools.memory.original_name.is_empty());
    assert!(app.native_tools.memory.content.is_empty());
}
#[test]
fn catalog_program_run_uses_saved_namespace_without_opening_ide_tabs() {
    let (mut app, ctx) = fixture(Page::Programs);
    app.editor.active_tab_mut().unwrap().content = "EXPORT SENTINEL() BEGIN RETURN 0; END;".into();
    let tabs = app.editor.tabs.len();
    app.native_tools.memory.name = "COUNTED".into();
    app.native_tools.memory.content =
        "LOCAL calls:=0; EXPORT COUNTED() BEGIN calls:=calls+1; RETURN calls; END;".into();
    click(&mut app, &ctx, "Save");
    for expected in [1, 2] {
        app.native_tools.open(Page::Programs);
        click(&mut app, &ctx, "Run");
        wait(&mut app, &ctx, |a| {
            a.native_compile.is_none()
                && !a.controller.running()
                && matches!(a.state.vm_state, crate::ide::VmExecutionState::Terminated)
        });
        let s = app.controller.session_snapshot();
        assert_eq!(
            s.last_value.as_ref().unwrap().display(&s.settings),
            expected.to_string()
        );
    }
    assert_eq!(app.editor.tabs.len(), tabs);
    assert_eq!(
        app.editor.active_tab().unwrap().content,
        "EXPORT SENTINEL() BEGIN RETURN 0; END;"
    );
}

fn profile_directory(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "ppl-catalog-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}
#[test]
fn catalog_saved_objects_survive_closed_tabs_and_a_reopened_app_profile() {
    let dir = profile_directory("restart");
    let (mut app, ctx) = fixture(Page::Notes);
    app.controller.open_native_profile(&dir).unwrap();
    app.native_tools.memory.name = "NOTE_SAVED".into();
    app.native_tools.memory.content = "π\ncalculator note".into();
    click(&mut app, &ctx, "Save");
    app.native_tools.open(Page::Programs);
    app.native_tools.memory.name = "PROGRAM_SAVED".into();
    let source = "EXPORT PROGRAM_SAVED() BEGIN RETURN 42; END;";
    app.native_tools.memory.content = source.into();
    click(&mut app, &ctx, "Save");
    app.editor.new_tab("PROGRAM_SAVED.hpprgm", source);
    app.editor.close_tab(app.editor.active_tab_index);
    assert_eq!(
        app.controller.session_snapshot().programs["PROGRAM_SAVED"],
        source
    );
    app.native_tools.open(Page::Lists);
    app.native_tools.memory.name = "LIST_SAVED".into();
    app.native_tools.memory.content = "{1,2,3}".into();
    click(&mut app, &ctx, "Store");
    complete(&mut app, &ctx);
    app.native_tools.open(Page::Matrices);
    app.native_tools.memory.name = "MATRIX_SAVED".into();
    click(&mut app, &ctx, "Open editor / Create");
    wait_dialog(&mut app, &ctx);
    type_text(&mut app, &ctx, "9");
    pointer_dialog(&app, 40, 227);
    complete(&mut app, &ctx);
    app.persist_native_ui();
    app.controller.native_profile().unwrap().flush().unwrap();
    drop(app);
    let (mut reopened, ctx2) = fixture(Page::Notes);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match reopened.controller.open_native_profile(&dir) {
            Ok(_) => break,
            Err(e) => {
                assert!(Instant::now() < deadline, "profile did not reopen: {e}");
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }
    let snapshot = reopened.controller.session_snapshot();
    assert_eq!(snapshot.notes["NOTE_SAVED"], "π\ncalculator note");
    assert_eq!(snapshot.programs["PROGRAM_SAVED"], source);
    assert_eq!(
        snapshot.globals["LIST_SAVED"].display(&snapshot.settings),
        "{1,2,3}"
    );
    assert_eq!(
        snapshot.globals["MATRIX_SAVED"].display(&snapshot.settings),
        "[[9]]"
    );
    click(&mut reopened, &ctx2, "NOTE_SAVED");
    assert_eq!(reopened.native_tools.memory.content, "π\ncalculator note");
    reopened.native_tools.open(Page::Programs);
    click(&mut reopened, &ctx2, "PROGRAM_SAVED");
    assert_eq!(reopened.native_tools.memory.content, source);
    assert_eq!(reopened.native_tools.memory.original_name, "PROGRAM_SAVED");
    reopened
        .controller
        .native_profile()
        .unwrap()
        .flush()
        .unwrap();
    drop(reopened);
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn catalog_rejected_profile_never_overwrites_future_data() {
    let dir = profile_directory("future");
    std::fs::create_dir_all(&dir).unwrap();
    let state = dir.join("state-v1.json");
    let original = b"{\"version\":99,\"do_not_overwrite\":true}";
    std::fs::write(&state, original).unwrap();
    let (mut app, ctx) = fixture(Page::Notes);
    let error = app.controller.open_native_profile(&dir).unwrap_err();
    app.profile_problem = Some(error.clone());
    app.native_tools.memory.name = "NOTE".into();
    app.native_tools.memory.content = "unsaved".into();
    click(&mut app, &ctx, "Save");
    assert_eq!(app.native_tools.error.as_ref(), Some(&error));
    assert!(app.controller.session_snapshot().notes.is_empty());
    assert_eq!(std::fs::read(&state).unwrap(), original);
    drop(app);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn catalog_profile_write_failure_is_reported_without_losing_in_memory_object() {
    let dir = profile_directory("write-error");
    let (mut app, ctx) = fixture(Page::Notes);
    app.controller.open_native_profile(&dir).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    app.native_tools.memory.name = "UNSAVED".into();
    app.native_tools.memory.content = "retain this draft".into();
    click(&mut app, &ctx, "Save");
    assert!(app.controller.native_profile().unwrap().flush().is_err());
    app.persist_native_ui();
    assert!(
        app.state.status_message.contains("not saved"),
        "{}",
        app.state.status_message
    );
    assert_eq!(
        app.controller.session_snapshot().notes["UNSAVED"],
        "retain this draft"
    );
    assert_eq!(app.native_tools.memory.content, "retain this draft");
}

#[test]
fn catalog_program_prefers_named_export_links_installed_dependencies_and_returns_to_tools() {
    let (mut app, ctx) = fixture(Page::Programs);
    let tabs = app.editor.tabs.len();
    app.controller
        .mutate_memory(|m| {
            m.set_variable(
                "Ans",
                ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(99)),
            )
            .map_err(|e| e.to_string())
        })
        .unwrap();
    app.handle_native_tool_request(
        Request::SaveProgram(
            "TRIPLE".into(),
            "EXPORT TRIPLE(x) BEGIN RETURN 3*x; END;".into(),
        ),
        &ctx,
    );
    let source =
        "EXPORT HELPER() BEGIN RETURN 0; END; EXPORT RUNNER() BEGIN RETURN TRIPLE(14); END;";
    app.native_tools.memory.name = "RUNNER".into();
    app.native_tools.memory.content = source.into();
    click(&mut app, &ctx, "Save");
    click(&mut app, &ctx, "Run");
    wait(&mut app, &ctx, |a| {
        !a.controller.running()
            && matches!(a.state.vm_state, crate::ide::VmExecutionState::Terminated)
    });
    let s = app.controller.session_snapshot();
    assert_eq!(s.last_value.as_ref().unwrap().display(&s.settings), "42");
    assert_eq!(s.globals["Ans"].display(&s.settings), "99");
    assert_eq!(app.native_tools.page, None);
    assert_eq!(app.state.native.view, NativeView::ProgramOutput);
    assert!(app.state.native.history.is_empty());
    assert_eq!(app.editor.tabs.len(), tabs);
    press(&mut app, &ctx, egui::Key::Escape);
    assert_eq!(app.native_tools.page, Some(Page::Programs));
    assert_eq!(app.state.native.view, NativeView::Home);
    assert_eq!(app.native_tools.memory.content, source);
}
#[test]
fn catalog_program_on_cancels_modal_and_loop_then_can_run_again() {
    let (mut app, ctx) = fixture(Page::Programs);
    let tabs = app.editor.tabs.len();
    app.native_tools.memory.name = "SPIN".into();
    app.native_tools.memory.content =
        "EXPORT SPIN() BEGIN LOCAL n:=0; INPUT(n); RETURN n; END;".into();
    click(&mut app, &ctx, "Save");
    click(&mut app, &ctx, "Run");
    wait_dialog(&mut app, &ctx);
    type_text(&mut app, &ctx, "9");
    press(&mut app, &ctx, egui::Key::F12);
    wait(&mut app, &ctx, |a| {
        !a.controller.running() && !a.native_stopping && a.state.native.operation.is_none()
    });
    assert_eq!(app.native_tools.page, Some(Page::Programs));
    assert!(
        app.controller
            .shared
            .active_dialog
            .lock()
            .unwrap()
            .is_none()
    );
    app.native_tools.memory.content = "EXPORT SPIN() BEGIN WHILE 1 DO END; END;".into();
    click(&mut app, &ctx, "Run");
    wait(&mut app, &ctx, |a| a.controller.running());
    press(&mut app, &ctx, egui::Key::F12);
    wait(&mut app, &ctx, |a| {
        !a.controller.running() && !a.native_stopping && a.state.native.operation.is_none()
    });
    assert_eq!(app.native_tools.page, Some(Page::Programs));
    app.native_tools.memory.content = "EXPORT SPIN() BEGIN RETURN 9; END;".into();
    click(&mut app, &ctx, "Run");
    wait(&mut app, &ctx, |a| {
        !a.controller.running()
            && matches!(a.state.vm_state, crate::ide::VmExecutionState::Terminated)
    });
    let s = app.controller.session_snapshot();
    assert_eq!(s.last_value.as_ref().unwrap().display(&s.settings), "9");
    assert_eq!(app.editor.tabs.len(), tabs);
    assert!(app.state.native.history.is_empty());
}

#[test]
fn catalog_parameter_prompt_validates_cancels_and_accepts_calculator_expressions() {
    let (mut app, ctx) = fixture(Page::Programs);
    app.controller
        .mutate_memory(|m| {
            m.set_variable(
                "A",
                ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(5)),
            )
            .unwrap();
            m.set_variable(
                "Ans",
                ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(99)),
            )
            .map_err(|e| e.to_string())
        })
        .unwrap();
    app.native_tools.memory.name = "FOO".into();
    app.native_tools.memory.content =
        "LOCAL calls:=0; EXPORT FOO(x) BEGIN calls:=calls+1; RETURN {calls,x}; END;".into();
    click(&mut app, &ctx, "Save");
    click(&mut app, &ctx, "Run");
    wait_dialog(&mut app, &ctx);
    {
        let d = app.controller.shared.active_dialog.lock().unwrap();
        let Some(ppl_vm::host::Dialog::Input { title, fields, .. }) = d.as_ref() else {
            panic!("argument dialog missing")
        };
        assert_eq!(title, "Run FOO");
        assert_eq!(fields[0].label, "x");
    }
    type_text(&mut app, &ctx, "bad_parameter");
    press(&mut app, &ctx, egui::Key::Enter);
    wait(
        &mut app,
        &ctx,
        |a| matches!(a.controller.shared.active_dialog.lock().unwrap().as_ref(),Some(ppl_vm::host::Dialog::Input{error:Some(_),fields,..})if fields[0].current=="bad_parameter"),
    );
    type_text(&mut app, &ctx, "A+1");
    press(&mut app, &ctx, egui::Key::Enter);
    wait(&mut app, &ctx, |a| {
        !a.controller.running()
            && matches!(a.state.vm_state, crate::ide::VmExecutionState::Terminated)
    });
    let s = app.controller.session_snapshot();
    assert_eq!(s.last_value.unwrap().display(&s.settings), "{1,6}");
    assert_eq!(s.globals["A"].display(&s.settings), "5");
    assert_eq!(s.globals["Ans"].display(&s.settings), "99");
    press(&mut app, &ctx, egui::Key::Escape);
    click(&mut app, &ctx, "Run");
    wait_dialog(&mut app, &ctx);
    press(&mut app, &ctx, egui::Key::Escape);
    wait(&mut app, &ctx, |a| {
        !a.controller.running() && a.state.native.operation.is_none()
    });
    assert_eq!(app.native_tools.page, Some(Page::Programs));
    assert!(app.native_tools.error.is_none());
    assert_eq!(app.state.status_message, "Program arguments cancelled");
    click(&mut app, &ctx, "Run");
    wait_dialog(&mut app, &ctx);
    press(&mut app, &ctx, egui::Key::F12);
    wait(&mut app, &ctx, |a| {
        !a.controller.running() && !a.native_stopping && a.state.native.operation.is_none()
    });
    assert_eq!(app.native_tools.page, Some(Page::Programs));
    let source = app.native_tools.memory.content.clone();
    app.run_native_catalog_program_configured(
        "FOO".into(),
        source,
        Some("FOO".into()),
        "A+1".into(),
    )
    .unwrap();
    wait(&mut app, &ctx, |a| {
        !a.controller.running()
            && matches!(a.state.vm_state, crate::ide::VmExecutionState::Terminated)
    });
    let s = app.controller.session_snapshot();
    assert_eq!(s.last_value.unwrap().display(&s.settings), "{2,6}");
    assert!(app.state.native.history.is_empty());
}
#[test]
fn catalog_configured_entry_and_arguments_execute_without_ide_or_ans_changes() {
    let (mut app, ctx) = fixture(Page::Programs);
    let tabs = app.editor.tabs.len();
    app.controller
        .mutate_memory(|m| {
            m.set_variable(
                "A",
                ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(5)),
            )
            .unwrap();
            m.set_variable(
                "Ans",
                ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(77)),
            )
            .map_err(|e| e.to_string())
        })
        .unwrap();
    let source = "EXPORT FIRST(x) BEGIN RETURN x+100; END; EXPORT FOO(x) BEGIN RETURN x*3; END;";
    app.native_tools.memory.name = "LIBRARY".into();
    app.native_tools.memory.content = source.into();
    click(&mut app, &ctx, "Save");
    app.run_native_catalog_program_configured(
        "LIBRARY".into(),
        source.into(),
        Some("FOO".into()),
        "A+1".into(),
    )
    .unwrap();
    wait(&mut app, &ctx, |a| {
        !a.controller.running()
            && matches!(a.state.vm_state, crate::ide::VmExecutionState::Terminated)
    });
    let s = app.controller.session_snapshot();
    assert_eq!(s.last_value.unwrap().display(&s.settings), "18");
    assert_eq!(s.globals["Ans"].display(&s.settings), "77");
    assert_eq!(app.editor.tabs.len(), tabs);
    assert!(
        app.controller
            .shared
            .active_dialog
            .lock()
            .unwrap()
            .is_none()
    );
    press(&mut app, &ctx, egui::Key::Escape);
    app.run_native_catalog_program_configured(
        "LIBRARY".into(),
        source.into(),
        Some("MISSING".into()),
        "A+1".into(),
    )
    .unwrap();
    wait(&mut app, &ctx, |a| {
        !a.controller.running()
            && matches!(a.state.vm_state, crate::ide::VmExecutionState::Error(_))
    });
    assert!(
        app.state
            .native
            .error
            .as_ref()
            .is_some_and(|e| e.contains("MISSING"))
    );
    press(&mut app, &ctx, egui::Key::Escape);
    assert_eq!(app.native_tools.page, Some(Page::Programs));
    assert_eq!(app.editor.tabs.len(), tabs);
}

#[test]
fn catalog_opening_an_editor_from_split_view_transfers_keyboard_ownership() {
    let (mut app, ctx) = fixture(Page::Matrices);
    app.state.layout = crate::ide::layout::WorkspaceLayout::Split;
    app.state.show_calculator = true;
    app.state.calc_input_focus = false;
    app.native_tools.memory.name = "SPLIT_MATRIX".into();
    click(&mut app, &ctx, "Open editor / Create");
    wait_dialog(&mut app, &ctx);
    assert_eq!(
        app.native_input_owner(),
        crate::ide::native::InputOwner::CalculatorModal
    );
    type_text(&mut app, &ctx, "9");
    pointer_dialog(&app, 40, 227);
    complete(&mut app, &ctx);
    let s = app.controller.session_snapshot();
    assert_eq!(s.globals["SPLIT_MATRIX"].display(&s.settings), "[[9]]");
}
