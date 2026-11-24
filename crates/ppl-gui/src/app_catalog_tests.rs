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
        app.poll_native_tools(ctx);
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
            "catalog did not complete: {}; VM {:?}; tool error {:?}; native outcome {:?}",
            app.state.status_message,
            app.state.vm_state,
            app.native_tools.error,
            app.state.native.last_outcome
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn character_pages_select_insert_and_evaluate_through_real_gui_requests() {
    let (mut app, ctx) = fixture(Page::Characters);
    crate::theme::apply_theme(&ctx);
    app.state.native.edit(|editor| editor.type_text("2"));
    click(&mut app, &ctx, "Latin");
    click(&mut app, &ctx, "Greek and Coptic");
    assert_eq!(app.native_tools.memory.characters.page, 7);
    click(&mut app, &ctx, "π");
    assert_eq!(app.native_tools.memory.characters.code, 0x3c0);
    click(&mut app, &ctx, "OK");
    assert!(app.native_tools.page.is_none());
    assert_eq!(app.state.native.home_input, "2*π");
    app.route_native_press(KeyPress::code(30), "keypad", &ctx);
    wait(&mut app, &ctx, |app| {
        app.state.native.last_outcome.is_some()
    });
    assert_eq!(
        app.state.native.last_outcome.as_ref().unwrap()["result"],
        "6.28318530718"
    );
}

#[test]
fn characters_keyboard_navigation_changes_selection_and_enter_inserts_once() {
    let (mut app, ctx) = fixture(Page::Characters);
    app.native_tools.memory.characters.select_page(7);
    let key = |key| egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    for _ in 0..5 {
        frame(&mut app, &ctx, vec![key(egui::Key::ArrowDown)]);
    }
    assert_eq!(app.native_tools.memory.characters.code, 0x3c0);
    frame(&mut app, &ctx, vec![key(egui::Key::ArrowRight)]);
    assert_eq!(app.native_tools.memory.characters.code, 0x3c1);
    frame(&mut app, &ctx, vec![key(egui::Key::ArrowLeft)]);
    frame(&mut app, &ctx, vec![key(egui::Key::Enter)]);
    assert_eq!(app.state.native.home_input, "π");
    assert!(app.native_tools.page.is_none());
    assert!(!app.controller.running());
}

#[test]
fn character_page_menu_accepts_keyboard_navigation_without_editing_home() {
    let (mut app, ctx) = fixture(Page::Characters);
    click(&mut app, &ctx, "Latin");
    let key = |key| egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    for _ in 0..7 {
        frame(&mut app, &ctx, vec![key(egui::Key::ArrowDown)]);
    }
    frame(&mut app, &ctx, vec![key(egui::Key::Enter)]);
    assert_eq!(app.native_tools.memory.characters.page, 7);
    assert_eq!(app.native_tools.memory.characters.code, 0x370);
    assert_eq!(app.native_tools.page, Some(Page::Characters));
    assert_eq!(app.state.native.home_input, "");
    assert!(!egui::Popup::is_any_open(&ctx));
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

#[test]
fn statistics_fit_picker_calculates_and_plots_the_selected_nonlinear_model() {
    use crate::ide::native_regression::FitKind;
    let (mut app, ctx) = fixture(Page::Numeric);
    app.native_tools.memory.selected_app = "Statistics 2Var".into();
    app.native_tools.memory.models.statistics2.pairs = vec![[1., 2.], [2., 8.], [3., 18.]];
    app.sync_native_statistics_memory();
    app.native_tools.open(Page::Symbolic);
    let lcd_click = |app: &mut IdeDesktopApp, x: f32, y: f32| {
        frame(app, &ctx, Vec::new());
        frame(app, &ctx, Vec::new());
        let pos = app.display.last_display_rect.min + egui::vec2(x, y) * 2.;
        for pressed in [true, false] {
            frame(
                app,
                &ctx,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    };
    lcd_click(&mut app, 258., 59.);
    lcd_click(&mut app, 100., 97.);
    assert_eq!(
        app.native_tools
            .memory
            .models
            .statistics2_book
            .as_ref()
            .unwrap()
            .analyses[0]
            .fit,
        FitKind::Power
    );
    lcd_click(&mut app, 185., 230.);
    wait(&mut app, &ctx, |app| {
        app.native_tools
            .memory
            .models
            .results
            .contains_key("Statistics 2Var")
    });
    let result = &app.native_tools.memory.models.results["Statistics 2Var"]["S1"]["regression"];
    assert!((result["coefficients"][0].as_f64().unwrap() - 2.).abs() < 1e-10);
    assert!((result["coefficients"][1].as_f64().unwrap() - 2.).abs() < 1e-10);
    app.native_tools.memory.model_windows.insert(
        "Statistics 2Var".into(),
        crate::ide::native_apps::PlotWindow {
            xmin: 0.5,
            xmax: 3.5,
            ..Default::default()
        },
    );
    app.route_native_press(
        crate::dialog::KeyPress::code(crate::keymap::KEY_PLOT),
        "test",
        &ctx,
    );
    app.consume_native_tool(&ctx);
    wait(&mut app, &ctx, |app| {
        app.native_tools
            .memory
            .models
            .results
            .contains_key("Statistics 2Var:plot")
    });
    let plot: crate::ide::native_model_plots::PlotData = serde_json::from_value(
        app.native_tools.memory.models.results["Statistics 2Var:plot"].clone(),
    )
    .unwrap();
    assert_eq!(plot.curves[1].len(), 257);
    assert_eq!(plot.curves[1].first().unwrap()[0], 0.5);
    assert_eq!(plot.curves[1].last().unwrap()[0], 3.5);
    for &[x, y] in &plot.curves[1] {
        assert!((y - 2. * x * x).abs() < 1e-8);
    }
}

#[test]
fn function_split_views_keep_reference_fixed_and_run_plot_and_table_together() {
    let (mut app, ctx) = fixture(Page::Views);
    app.controller
        .mutate_memory(|memory| {
            memory
                .set_variable("NumStep", ppl_vm::PortableValue::Real(ppl_math::Real::ONE))
                .map_err(|e| e.to_string())
        })
        .unwrap();
    app.native_tools
        .memory
        .graphs
        .get_mut("Function")
        .unwrap()
        .numeric_step = 1.;
    app.native_tools
        .memory
        .graphs
        .get_mut("Function")
        .unwrap()
        .curves[0]
        .expression = "X^2".into();
    app.native_tools
        .memory
        .graphs
        .get_mut("Function")
        .unwrap()
        .curves[0]
        .enabled = true;
    click(&mut app, &ctx, "Split Screen: Plot Detail");
    wait(&mut app, &ctx, |app| !app.native_tools.calculating());
    assert_eq!(app.native_tools.page, Some(Page::PlotDetail));
    let (left_window, left_points) = app.native_tools.detail_plot().unwrap().clone();
    assert!(!left_points.is_empty());
    for p in left_points[0].iter().flatten() {
        assert!((p[1] - p[0] * p[0]).abs() < 1e-8);
    }
    app.route_native_press(
        crate::dialog::KeyPress::code(crate::keymap::KEY_PLUS),
        "test",
        &ctx,
    );
    wait(&mut app, &ctx, |app| !app.native_tools.calculating());
    let right = &app.native_tools.memory.graphs["Function"].window;
    assert!((right.xmax - right.xmin - (left_window.xmax - left_window.xmin) / 2.).abs() < 1e-10);
    assert_eq!(
        serde_json::to_value(app.native_tools.detail_plot().unwrap()).unwrap(),
        serde_json::to_value((&left_window, &left_points)).unwrap()
    );
    assert_ne!(app.native_tools.plot_samples(), left_points);
    click(&mut app, &ctx, "Replace reference");
    assert_eq!(
        app.native_tools.detail_plot().unwrap().1,
        app.native_tools.plot_samples()
    );
    assert_eq!(
        app.native_tools.detail_plot().unwrap().0.xmin,
        app.native_tools.memory.graphs["Function"].window.xmin
    );
    click(&mut app, &ctx, "Unsplit");
    assert_eq!(app.native_tools.page, Some(Page::Plot));
    assert!(app.native_tools.detail_plot().is_none());
    app.native_tools.open(Page::Views);
    click(&mut app, &ctx, "Split Screen: Plot Table");
    wait(&mut app, &ctx, |app| !app.native_tools.calculating());
    assert_eq!(app.native_tools.page, Some(Page::PlotTable));
    assert!(!app.native_tools.plot_samples().is_empty());
    assert_eq!(
        &app.native_tools.numeric_table()[..3],
        &[
            vec![Some(0.), Some(0.)],
            vec![Some(1.), Some(1.)],
            vec![Some(2.), Some(4.)]
        ]
    );
    let output = frame(&mut app, &ctx, vec![]);
    let cell = |label| {
        output
            .shapes
            .iter()
            .find_map(|shape| text_rect(&shape.shape, label))
            .expect("visible numeric row")
    };
    let first = cell("0.00000 0.00000");
    let second = cell("1.00000 1.00000");
    assert!(
        second.top() > first.bottom(),
        "table rows must use a vertical layout"
    );
    assert!(
        first.left() > app.display.last_display_rect.center().x,
        "table must stay in the right pane"
    );
    assert!(
        first.right() <= app.display.last_display_rect.right(),
        "table must fit inside the LCD"
    );
    click(&mut app, &ctx, "1.00000 1.00000");
    let point = app
        .native_tools
        .trace
        .current(app.native_tools.plot_samples())
        .expect("table row selects plotted sample");
    assert!((point.point[0] - 1.).abs() < 0.05);
    assert!((point.point[1] - point.point[0].powi(2)).abs() < 1e-8);
}

#[test]
fn startview_uses_live_program_values_and_returns_preset_bounds_to_the_vm() {
    let (mut app, ctx) = fixture(Page::Programs);
    app.native_tools
        .memory
        .graphs
        .get_mut("Function")
        .unwrap()
        .curves[0]
        .expression = "A*X".into();
    app.native_tools
        .memory
        .graphs
        .get_mut("Function")
        .unwrap()
        .curves[0]
        .enabled = true;
    for (body, expected, page) in [
        (
            "A:=3; Xmin:=1; Xmax:=4; STARTAPP(\"Function\"); STARTVIEW(9,1); RETURN Xmin;",
            "1",
            Page::PlotTable,
        ),
        ("STARTVIEW(11,1); RETURN Xmax-Xmin;", "31.8", Page::Plot),
        ("STARTVIEW(10,1); RETURN Ymax-Ymin;", "110.664", Page::Plot),
    ] {
        let source = format!("EXPORT LIVEVIEW() BEGIN {body} END;");
        app.handle_native_tool_request(
            Request::SaveProgram("LIVEVIEW".into(), source.clone()),
            &ctx,
        );
        app.handle_native_tool_request(Request::RunProgram("LIVEVIEW".into(), source), &ctx);
        wait(&mut app, &ctx, |a| {
            a.native_compile.is_none()
                && !a.controller.running()
                && matches!(a.state.vm_state, crate::ide::VmExecutionState::Terminated)
        });
        let s = app.controller.session_snapshot();
        let result = s
            .last_value
            .as_ref()
            .expect("program must return")
            .display(&s.settings);
        assert_eq!(
            result, expected,
            "{body}; error {:?}",
            app.native_tools.error
        );
        wait(&mut app, &ctx, |a| !a.native_tools.calculating());
        assert_eq!(app.native_tools.page, Some(page));
        assert!(
            app.native_tools.error.is_none(),
            "{:?}",
            app.native_tools.error
        );
        if page == Page::PlotTable {
            let points = app.native_tools.plot_samples();
            assert_eq!(points[0].first().unwrap().unwrap()[0], 1.);
            assert_eq!(points[0].last().unwrap().unwrap()[0], 4.);
            for point in points[0].iter().flatten() {
                assert!((point[1] - 3. * point[0]).abs() < 1e-8);
            }
        }
    }
}

#[test]
fn view_requests_survive_key_release_but_reject_retired_execution_owners() {
    use ppl_vm::{Host, host::CalculatorView};
    for retire in [false, true] {
        let (mut app, ctx) = fixture(Page::Programs);
        let shared = app.controller.shared.clone();
        let mut host = crate::host::GuiHost::new(shared.clone());
        let worker = std::thread::spawn(move || {
            host.calculator_view(CalculatorView::StartApp("Finance".into()))
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        while shared.calculator_views.lock().unwrap().is_empty() {
            assert!(Instant::now() < deadline, "view request was not enqueued");
            std::thread::yield_now();
        }
        // This exact interleaving caused the intermittent STARTVIEW failure:
        // launch worker, worker queues view, UI retires the starting key press.
        app.release_native_input();
        if retire {
            shared.reset_for_run();
        }
        app.poll_native_tools(&ctx);
        let result = worker.join().unwrap();
        if retire {
            assert!(result.is_err());
            assert_eq!(app.native_tools.page, Some(Page::Programs));
            assert_ne!(app.native_tools.memory.selected_app, "Finance");
        } else {
            assert!(result.unwrap());
            assert_eq!(app.native_tools.memory.selected_app, "Finance");
            assert_eq!(app.native_tools.page, Some(Page::Numeric));
        }
    }
    let shared = std::sync::Arc::new(crate::host::GuiSharedState::new());
    let mut retired = crate::host::GuiHost::new(shared.clone());
    shared.reset_for_run();
    assert!(
        retired
            .calculator_view(CalculatorView::StartApp("Finance".into()))
            .is_err()
    );
    assert!(
        shared.calculator_views.lock().unwrap().is_empty(),
        "old hosts must not adopt the next run's epoch"
    );
}

#[test]
fn startapp_carries_live_context_and_restores_destination_app_bounds() {
    let (mut app, ctx) = fixture(Page::Programs);
    let source = "EXPORT LIVEAPPS() BEGIN STARTAPP(\"Function\"); Xmin:=1;Xmax:=4;Ymin:=0;Ymax:=2;STARTVIEW(0); STARTAPP(\"Parametric\");Xmin:=-5;Xmax:=5;Ymin:=-8;Ymax:=8;STARTVIEW(0); STARTAPP(\"Function\"); RETURN {Xmin,Xmax,Ymin,Ymax}; END;";
    app.handle_native_tool_request(Request::SaveProgram("LIVEAPPS".into(), source.into()), &ctx);
    app.handle_native_tool_request(Request::RunProgram("LIVEAPPS".into(), source.into()), &ctx);
    wait(&mut app, &ctx, |a| {
        a.native_compile.is_none()
            && !a.controller.worker_alive()
            && matches!(
                a.state.vm_state,
                crate::ide::VmExecutionState::Terminated | crate::ide::VmExecutionState::Error(_)
            )
    });
    assert!(
        matches!(app.state.vm_state, crate::ide::VmExecutionState::Terminated),
        "{:?}",
        app.state.native.last_outcome
    );
    let memory = app.controller.session_snapshot();
    assert_eq!(
        memory.last_value.unwrap().display(&memory.settings),
        "{1,4,0,2}"
    );
    assert_eq!(memory.active_app, "Function");
}

#[test]
fn non_function_startview_numbers_execute_the_selected_apps_actual_actions() {
    let (mut app, ctx) = fixture(Page::Programs);
    for (name, curve, second) in [
        ("Advanced Graphing", "X^2+Y^2=4", ""),
        ("Parametric", "T", "T^2"),
        ("Polar", "2", ""),
        ("Sequence", "N^2", ""),
    ] {
        let graph = app.native_tools.memory.graphs.get_mut(name).unwrap();
        graph.curves[0].expression = curve.into();
        graph.curves[0].second = second.into();
    }
    app.native_tools.memory.models.equation = "x^2=A".into();
    // These view numbers deliberately differ: Advanced 9 is Decimal, 10
    // Integer; Sequence has no Decimal, while Polar has no Integer/Trig.
    for (name, body, expected, page) in [
        (
            "Advanced Graphing",
            "STARTVIEW(9,1); RETURN Xmax-Xmin;",
            "31.8",
            Page::Plot,
        ),
        (
            "Advanced Graphing",
            "Ymin:=2; Ymax:=6; STARTVIEW(10,1); RETURN {Xmax-Xmin,Ymin,Ymax};",
            "{318,2,6}",
            Page::Plot,
        ),
        (
            "Parametric",
            "Tmin:=0; Tmax:=3; NumStep:=1; RETURN STARTVIEW(9,1);",
            "1",
            Page::PlotTable,
        ),
        (
            "Polar",
            "HAngle:=0; Tmin:=0; Tmax:=2*π; NumStart:=0; NumStep:=π/2; RETURN STARTVIEW(9,1);",
            "1",
            Page::PlotTable,
        ),
        (
            "Parametric",
            "AAngle:=3; HAngle:=0; STARTVIEW(13,1); RETURN Xmax-Xmin;",
            "2650",
            Page::Plot,
        ),
        ("Polar", "RETURN STARTVIEW(12,1);", "0", Page::Symbolic),
        ("Sequence", "RETURN STARTVIEW(11,1);", "0", Page::Symbolic),
        ("Sequence", "RETURN STARTVIEW(9,1);", "1", Page::PlotTable),
        (
            "Solve",
            "A:=4; Xmin:=1; Xmax:=3; RETURN STARTVIEW(8,1);",
            "1",
            Page::PlotDetail,
        ),
        (
            "Solve",
            "STARTVIEW(10,1); RETURN Xmax-Xmin;",
            "31.8",
            Page::Plot,
        ),
        (
            "Solve",
            "Ymin:=2; Ymax:=6; STARTVIEW(11,1); RETURN {Xmax-Xmin,Ymin,Ymax};",
            "{318,2,6}",
            Page::Plot,
        ),
    ] {
        let source = format!("EXPORT APPVIEW() BEGIN STARTAPP(\"{name}\"); {body} END;");
        app.handle_native_tool_request(
            Request::SaveProgram("APPVIEW".into(), source.clone()),
            &ctx,
        );
        app.handle_native_tool_request(Request::RunProgram("APPVIEW".into(), source), &ctx);
        wait(&mut app, &ctx, |a| {
            a.native_compile.is_none()
                && !a.controller.worker_alive()
                && matches!(
                    a.state.vm_state,
                    crate::ide::VmExecutionState::Terminated
                        | crate::ide::VmExecutionState::Error(_)
                )
        });
        assert!(
            matches!(app.state.vm_state, crate::ide::VmExecutionState::Terminated),
            "{name}: {:?}",
            app.state.native.last_outcome
        );
        let memory = app.controller.session_snapshot();
        assert_eq!(
            memory
                .last_value
                .as_ref()
                .unwrap()
                .display(&memory.settings),
            expected,
            "{name}: {:?}",
            app.state.native.last_outcome
        );
        assert_eq!(app.native_tools.page, Some(page), "{name}");
        assert!(
            app.native_tools.error.is_none(),
            "{name}: {:?}",
            app.native_tools.error
        );
        if name == "Solve" {
            assert!(!app.native_tools.plot_samples().is_empty());
            let data: crate::ide::native_model_plots::PlotData = serde_json::from_value(
                app.native_tools.memory.models.results["Solve:plot"].clone(),
            )
            .unwrap();
            let right = data
                .labels
                .iter()
                .zip(&data.curves)
                .filter(|(label, _)| label.starts_with("Right:"));
            assert!(right.clone().next().is_some());
            for (_, curve) in right {
                for point in curve {
                    assert_eq!(point[1], 4.);
                }
            }
            if page == Page::PlotDetail {
                assert_eq!(app.native_tools.plot_samples()[0][0].unwrap(), [1., 1.]);
                assert_eq!(
                    app.native_tools.plot_samples()[0].last().unwrap().unwrap(),
                    [3., 9.]
                );
            }
        }
        if name == "Advanced Graphing" && expected == "31.8" {
            assert!(
                app.native_tools
                    .plot_samples()
                    .iter()
                    .flatten()
                    .flatten()
                    .next()
                    .is_some()
            );
        }
        if name == "Polar" && page == Page::PlotTable {
            click(&mut app, &ctx, "1.57080 2.00000");
            let point = app
                .native_tools
                .trace
                .current(app.native_tools.plot_samples())
                .unwrap()
                .point;
            assert!(point[0].abs() < 0.02 && (point[1] - 2.).abs() < 0.02);
        }
        if name == "Parametric" && page == Page::PlotTable {
            click(&mut app, &ctx, "2.000 2.000 4.000");
            let point = app
                .native_tools
                .trace
                .current(app.native_tools.plot_samples())
                .unwrap()
                .point;
            assert!((point[0] - 2.).abs() < 0.02 && (point[1] - 4.).abs() < 0.05);
            assert_eq!(
                &app.native_tools.numeric_table()[..3],
                &[
                    vec![Some(0.), Some(0.), Some(0.)],
                    vec![Some(1.), Some(1.), Some(1.)],
                    vec![Some(2.), Some(2.), Some(4.)]
                ]
            );
            for point in app.native_tools.plot_samples()[0].iter().flatten() {
                assert!((point[1] - point[0] * point[0]).abs() < 1E-9);
            }
        }
    }
}

#[test]
fn statistics_special_views_run_inside_ppl_without_second_owner_and_keep_reference_on_zoom() {
    let (mut app, ctx) = fixture(Page::Programs);
    app.native_tools.memory.models.statistics1.samples = vec![1., 2., 2., 3., 4.];
    app.native_tools.memory.models.statistics2.pairs = vec![[1., 3.], [2., 5.], [3., 7.]];
    for name in ["Statistics 1Var", "Statistics 2Var"] {
        let source = format!(
            "EXPORT STATVIEW() BEGIN D1:={{1,2,2,3,4}};C1:={{1,2,3}};C2:={{3,5,7}};STARTAPP(\"{name}\"); RETURN STARTVIEW(8,1); END;"
        );
        app.handle_native_tool_request(
            Request::SaveProgram("STATVIEW".into(), source.clone()),
            &ctx,
        );
        app.handle_native_tool_request(Request::RunProgram("STATVIEW".into(), source), &ctx);
        wait(&mut app, &ctx, |a| {
            a.native_compile.is_none()
                && !a.controller.worker_alive()
                && matches!(
                    a.state.vm_state,
                    crate::ide::VmExecutionState::Terminated
                        | crate::ide::VmExecutionState::Error(_)
                )
        });
        assert_eq!(app.native_tools.page, Some(Page::PlotDetail));
        assert!(
            app.native_tools.error.is_none(),
            "{:?}",
            app.native_tools.error
        );
        assert!(
            matches!(app.state.vm_state, crate::ide::VmExecutionState::Terminated),
            "{:?}",
            app.state.native.last_outcome
        );
        let reference = app.native_tools.detail_plot().unwrap().clone();
        assert!(!reference.1[0].is_empty());
        let initial_span = reference.0.xmax - reference.0.xmin;
        app.route_native_press(
            crate::dialog::KeyPress::code(crate::keymap::KEY_PLUS),
            "test",
            &ctx,
        );
        frame(&mut app, &ctx, vec![]);
        assert_eq!(
            serde_json::to_value(app.native_tools.detail_plot()).unwrap(),
            serde_json::to_value(Some(&reference)).unwrap()
        );
        assert!(
            (app.native_tools.trace_window().xmax
                - app.native_tools.trace_window().xmin
                - initial_span / 2.)
                .abs()
                < 1E-9
        );
        wait(&mut app, &ctx, |a| !a.native_tools.calculating());
        click(&mut app, &ctx, "Replace reference");
        assert_eq!(
            app.native_tools.detail_plot().unwrap().0.xmin,
            app.native_tools.trace_window().xmin
        );
        let source =
            format!("EXPORT STATVIEW() BEGIN STARTAPP(\"{name}\"); RETURN STARTVIEW(9,1); END;");
        app.handle_native_tool_request(
            Request::SaveProgram("STATVIEW".into(), source.clone()),
            &ctx,
        );
        app.handle_native_tool_request(Request::RunProgram("STATVIEW".into(), source), &ctx);
        wait(&mut app, &ctx, |a| {
            a.native_compile.is_none()
                && !a.controller.worker_alive()
                && matches!(
                    a.state.vm_state,
                    crate::ide::VmExecutionState::Terminated
                        | crate::ide::VmExecutionState::Error(_)
                )
        });
        assert_eq!(app.native_tools.page, Some(Page::Plot));
        assert!(app.native_tools.error.is_none());
        let memory = app.controller.session_snapshot();
        assert!(!memory.globals.contains_key("Ymin"));
        let ymin = memory
            .plot_variable_for(name, "Ymin")
            .unwrap()
            .to_value()
            .as_real()
            .unwrap()
            .to_f64();
        let ymax = memory
            .plot_variable_for(name, "Ymax")
            .unwrap()
            .to_value()
            .as_real()
            .unwrap()
            .to_f64();
        assert!(ymax > ymin);
        assert!((ymin - app.native_tools.trace_window().ymin).abs() < 1E-9);
        assert!((ymax - app.native_tools.trace_window().ymax).abs() < 1E-9);
    }
}

#[test]
fn startapp_restores_each_models_window_in_a_single_live_ppl_execution() {
    let (mut app, ctx) = fixture(Page::Programs);
    app.native_tools.memory.models.statistics1.samples = vec![1., 2., 3.];
    app.native_tools.memory.models.statistics2.pairs = vec![[1., 3.], [2., 5.], [3., 7.]];
    let source = "EXPORT APPWINDOWS() BEGIN D1:={1,2,3};C1:={1,2,3};C2:={3,5,7};STARTAPP(\"Statistics 1Var\"); Xmin:=1;Xmax:=4;Ymin:=0;Ymax:=2;STARTVIEW(8,1); STARTAPP(\"Statistics 2Var\");Xmin:=-5;Xmax:=5;Ymin:=-8;Ymax:=8;STARTVIEW(8,1); STARTAPP(\"Statistics 1Var\"); RETURN {Xmin,Xmax,Ymin,Ymax}; END;";
    app.handle_native_tool_request(
        Request::SaveProgram("APPWINDOWS".into(), source.into()),
        &ctx,
    );
    app.handle_native_tool_request(
        Request::RunProgram("APPWINDOWS".into(), source.into()),
        &ctx,
    );
    wait(&mut app, &ctx, |a| {
        a.native_compile.is_none()
            && !a.controller.worker_alive()
            && matches!(
                a.state.vm_state,
                crate::ide::VmExecutionState::Terminated | crate::ide::VmExecutionState::Error(_)
            )
    });
    assert!(
        matches!(app.state.vm_state, crate::ide::VmExecutionState::Terminated),
        "{:?}",
        app.state.native.last_outcome
    );
    let memory = app.controller.session_snapshot();
    assert_eq!(
        memory.last_value.unwrap().display(&memory.settings),
        "{1,4,0,2}"
    );
    assert_eq!(memory.active_app, "Statistics 1Var");
}

#[test]
fn solve_reference_captures_window_and_samples_before_an_initial_zoom() {
    let (mut app, ctx) = fixture(Page::Numeric);
    app.native_tools.memory.selected_app = "Solve".into();
    app.native_tools.memory.models.equation = "x^2=4".into();
    assert!(
        app.native_tools
            .open_special(8, true, app.controller.session_snapshot())
    );
    let original = app
        .native_tools
        .detail_plot()
        .expect("reference window is captured immediately")
        .0
        .clone();
    assert_eq!((original.xmin, original.xmax), (-10., 10.));
    // Do not paint or poll the original workers before changing the right side.
    app.route_native_press(
        crate::dialog::KeyPress::code(crate::keymap::KEY_PLUS),
        "test",
        &ctx,
    );
    wait(&mut app, &ctx, |a| !a.native_tools.calculating());
    assert!(
        app.native_tools.error.is_none(),
        "{:?}",
        app.native_tools.error
    );
    let (window, points) = app.native_tools.detail_plot().unwrap();
    assert_eq!((window.xmin, window.xmax), (-10., 10.));
    assert_eq!(points[0][0].unwrap(), [-10., 100.]);
    assert_eq!(points[0].last().unwrap().unwrap(), [10., 100.]);
    assert_eq!(app.native_tools.plot_samples()[0][0].unwrap(), [-5., 25.]);
    assert_eq!(
        app.native_tools.plot_samples()[0].last().unwrap().unwrap(),
        [5., 25.]
    );
}

#[test]
fn a_successful_right_plot_cannot_hide_an_unsupported_reference_call() {
    let (mut app, ctx) = fixture(Page::Programs);
    app.controller
        .mutate_memory(|memory| {
            memory
                .store_program(
                    "RISKY",
                    "EXPORT RISKY(X) BEGIN IF X<0 THEN STARTVIEW(1); END; RETURN X; END;".into(),
                )
                .map_err(|e| e.to_string())
        })
        .unwrap();
    let graph = app.native_tools.memory.graphs.get_mut("Function").unwrap();
    graph.curves[0].expression = "RISKY(X)".into();
    graph.curves[0].enabled = true;
    graph.window.xmin = -1.;
    graph.window.xmax = 1.;
    assert!(
        app.native_tools
            .open_special(8, true, app.controller.session_snapshot())
    );
    let graph = app.native_tools.memory.graphs.get_mut("Function").unwrap();
    graph.window.xmin = 1.;
    graph.window.xmax = 2.;
    app.native_tools.plot(app.controller.session_snapshot());
    wait(&mut app, &ctx, |a| !a.native_tools.calculating());
    assert!(!app.native_tools.plot_samples()[0].is_empty());
    assert!(
        app.native_tools.plot_samples()[0]
            .iter()
            .flatten()
            .all(|p| (p[0] - p[1]).abs() < 1E-9)
    );
    assert!(
        app.native_tools.error.is_some(),
        "successful right pane must not erase reference failure"
    );
    assert!(app.native_tools.detail_plot().unwrap().1.is_empty());
}

fn picker_key(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn command_search_keyboard_and_help_insert_the_selected_real_function_once() {
    let (mut app, ctx) = fixture(Page::Toolbox);
    app.state.native.edit(|e| e.type_text("7+"));
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![egui::Event::Text("sin".into())]);
    assert_eq!(app.native_tools.memory.commands.query, "sin");
    assert_eq!(app.native_tools.memory.commands.selected, "SIN");
    assert_eq!(app.state.native.home_input, "7+");
    frame(&mut app, &ctx, vec![picker_key(egui::Key::ArrowDown)]);
    assert_eq!(app.native_tools.memory.commands.selected, "SINH");
    frame(&mut app, &ctx, vec![picker_key(egui::Key::ArrowUp)]);
    assert_eq!(
        app.native_tools.memory.commands.selected,
        "SIN",
        "focus {:?}",
        ctx.memory(|m| m.focused())
    );
    click(&mut app, &ctx, "Help");
    assert_eq!(app.native_tools.page, Some(Page::Help));
    assert_eq!(app.native_tools.memory.help_topic, "SIN");
    click(&mut app, &ctx, "Insert command");
    assert_eq!(app.state.native.home_input, "7+SIN()");
    assert!(app.native_tools.page.is_none());
    assert!(app.state.native.history.is_empty());
    frame(&mut app, &ctx, vec![egui::Event::Text("0".into())]);
    frame(&mut app, &ctx, vec![picker_key(egui::Key::Enter)]);
    wait(&mut app, &ctx, |app| app.state.native.history.len() == 1);
    assert_eq!(app.state.native.history.back().unwrap().result, "7");
}

#[test]
fn empty_catalog_search_cannot_insert_or_evaluate_and_category_popup_owns_keys() {
    let (mut app, ctx) = fixture(Page::Toolbox);
    app.state.native.edit(|e| e.type_text("6*7"));
    frame(&mut app, &ctx, vec![]);
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::Text("no_such_command".into())],
    );
    frame(&mut app, &ctx, vec![picker_key(egui::Key::Enter)]);
    assert_eq!(app.native_tools.page, Some(Page::Toolbox));
    assert_eq!(app.state.native.home_input, "6*7");
    assert!(app.state.native.history.is_empty());
    click(&mut app, &ctx, "All");
    assert!(egui::Popup::is_any_open(&ctx));
    click(&mut app, &ctx, "math");
    assert_eq!(app.native_tools.memory.commands.category, "math");
    assert_eq!(app.state.native.home_input, "6*7");
    assert!(app.native_tools.memory.commands.selected.is_empty());
}

#[test]
fn templates_desktop_and_virtual_keys_edit_real_fields_and_evaluate() {
    let (mut app, ctx) = fixture(Page::Templates);
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![picker_key(egui::Key::ArrowDown)]);
    frame(&mut app, &ctx, vec![picker_key(egui::Key::ArrowRight)]);
    assert_eq!(app.native_tools.memory.template_selection, "Nth root");
    app.route_native_press(KeyPress::code(30), "keypad", &ctx);
    assert_eq!(app.state.native.home_input, "NTHROOT(,)");
    frame(&mut app, &ctx, vec![egui::Event::Text("3".into())]);
    frame(&mut app, &ctx, vec![picker_key(egui::Key::ArrowRight)]);
    frame(&mut app, &ctx, vec![egui::Event::Text("8".into())]);
    frame(&mut app, &ctx, vec![picker_key(egui::Key::Enter)]);
    wait(&mut app, &ctx, |app| app.state.native.history.len() == 1);
    assert_eq!(app.state.native.history.back().unwrap().result, "2");
}

#[test]
fn memory_inspects_both_scopes_and_deletes_only_selected_home_object() {
    let (mut app, ctx) = fixture(Page::Memory);
    app.controller
        .mutate_memory(|memory| {
            memory
                .set_variable(
                    "TOOLVALUE",
                    ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(17)),
                )
                .map_err(|e| e.to_string())?;
            memory.cas.bindings.insert("toolvalue".into(), "23".into());
            Ok(())
        })
        .unwrap();
    frame(&mut app, &ctx, vec![picker_key(egui::Key::ArrowDown)]);
    frame(&mut app, &ctx, vec![picker_key(egui::Key::Enter)]);
    assert_eq!(app.native_tools.page, Some(Page::CasVariables));
    click(&mut app, &ctx, "toolvalue: 23");
    assert_eq!(app.native_tools.memory.content, "23");
    click(&mut app, &ctx, "Home");
    click(&mut app, &ctx, "TOOLVALUE: 17");
    click(&mut app, &ctx, "Delete");
    let snapshot = app.controller.session_snapshot();
    assert!(!snapshot.globals.contains_key("TOOLVALUE"));
    assert_eq!(snapshot.cas.bindings["toolvalue"], "23");
    assert_eq!(snapshot.globals["Ans"].display(&snapshot.settings), "0");
}

#[test]
fn units_search_inserts_real_suffix_and_unsupported_prefix_remains_an_error() {
    let (mut app, ctx) = fixture(Page::Units);
    app.state.native.edit(|e| e.type_text("20"));
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![egui::Event::Text("centimetre".into())]);
    frame(&mut app, &ctx, vec![picker_key(egui::Key::Enter)]);
    assert_eq!(app.state.native.home_input, "20_cm");
    assert!(app.native_tools.page.is_none());
    app.native_tools.memory.units.query = "metre".into();
    app.native_tools.memory.units.selected = "m".into();
    app.native_tools.memory.unit_prefix = "f".into();
    app.native_tools.open(Page::Units);
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![picker_key(egui::Key::Enter)]);
    assert!(app.native_tools.error.as_deref().unwrap().contains("fm"));
    assert_eq!(app.native_tools.page, Some(Page::Units));
    assert_eq!(app.state.native.home_input, "20_cm");
}

#[test]
fn unit_prefix_popup_scrolls_with_keyboard_and_applies_the_actual_selected_prefix() {
    let (mut app, ctx) = fixture(Page::Units);
    app.state.native.edit(|editor| editor.type_text("2"));
    app.native_tools.memory.units.query = "metre".into();
    frame(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, "None");
    for _ in 0..8 {
        frame(&mut app, &ctx, vec![picker_key(egui::Key::ArrowDown)]);
    }
    frame(&mut app, &ctx, vec![picker_key(egui::Key::Enter)]);
    assert_eq!(app.native_tools.memory.unit_prefix, "k");
    assert_eq!(app.state.native.home_input, "2");
    assert!(!egui::Popup::is_any_open(&ctx));
    frame(&mut app, &ctx, vec![picker_key(egui::Key::Enter)]);
    assert_eq!(app.state.native.home_input, "2_km");
    assert!(app.native_tools.page.is_none());
}

#[test]
fn home_store_softkey_and_shift_store_use_ans_and_commit_real_home_variables() {
    let (mut app, ctx) = fixture(Page::Variables);
    app.native_tools.close();
    app.state.native.edit(|editor| editor.type_text("2+3"));
    app.submit_native_home(&ctx);
    wait(&mut app, &ctx, |app| app.state.native.history.len() == 1);
    // Pointer coordinates project to the real first LCD softkey, not a request.
    let rect = app.display.last_display_rect;
    let pos = rect.min + egui::vec2(20. / 320. * rect.width(), 230. / 240. * rect.height());
    let _ = ctx.run(
        egui::RawInput {
            focused: true,
            events: vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        },
        |ctx| app.route_native_pointer(ctx),
    );
    assert_eq!(app.state.native.home_input, "Ans▶");
    app.state.native.edit(|editor| editor.type_text("A"));
    app.submit_native_home(&ctx);
    wait(&mut app, &ctx, |app| app.state.native.history.len() == 2);
    let snapshot = app.controller.session_snapshot();
    assert_eq!(snapshot.globals["A"].display(&snapshot.settings), "5");
    app.route_native_press(KeyPress::code(crate::keymap::KEY_SHIFT), "keypad", &ctx);
    app.route_native_press(KeyPress::code(crate::keymap::KEY_EEX), "keypad", &ctx);
    assert_eq!(app.state.native.home_input, "Ans▶");
}

#[test]
fn geometry_recalculates_when_home_revision_changes_without_promoting_ans() {
    let (mut app, ctx) = fixture(Page::Programs);
    app.handle_native_tool_request(Request::SelectApp("Geometry".into()), &ctx);
    let mut book = crate::ide::native_geometry::Book::default();
    book.objects.push(crate::ide::native_geometry::Object {
        name: "A".into(),
        source: "point(A,0)".into(),
        ..Default::default()
    });
    app.native_tools.memory.models.geometry_book = Some(book);
    app.controller
        .mutate_memory(|memory| {
            memory
                .set_variable(
                    "A",
                    ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(3)),
                )
                .map_err(|e| e.to_string())
        })
        .unwrap();
    let before = app.controller.session_snapshot();
    app.native_tools.open(Page::Plot);
    app.native_tools.plot(before.clone());
    let x = |app: &IdeDesktopApp| {
        app.native_tools
            .memory
            .models
            .results
            .get("Geometry:plot")
            .and_then(|v| v["geometry"]["objects"][0]["value"]["Ok"]["Point"]["x"].as_f64())
    };
    wait(&mut app, &ctx, |app| x(app) == Some(3.));
    app.controller
        .mutate_memory(|memory| {
            memory
                .set_variable(
                    "A",
                    ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(9)),
                )
                .map_err(|e| e.to_string())
        })
        .unwrap();
    wait(&mut app, &ctx, |app| x(app) == Some(9.));
    let after = app.controller.session_snapshot();
    assert_eq!(after.globals["Ans"], before.globals["Ans"]);
    assert!(!after.globals.contains_key("GA"));
}
#[test]
fn geometry_view_pins_live_ppl_values_until_the_execution_commits() {
    let (mut app, ctx) = fixture(Page::Programs);
    app.native_tools.memory.models.geometry_book = Some(crate::ide::native_geometry::Book {
        objects: vec![crate::ide::native_geometry::Object {
            name: "A".into(),
            source: "point(A,0)".into(),
            ..Default::default()
        }],
        ..Default::default()
    });
    app.controller
        .mutate_memory(|memory| {
            memory
                .set_variable(
                    "A",
                    ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(1)),
                )
                .map_err(|e| e.to_string())
        })
        .unwrap();
    let source =
        "EXPORT GEOLIVE() BEGIN STARTAPP(\"Geometry\");A:=7;STARTVIEW(0);WAIT(0.3);RETURN A;END;";
    app.handle_native_tool_request(Request::SaveProgram("GEOLIVE".into(), source.into()), &ctx);
    app.handle_native_tool_request(Request::RunProgram("GEOLIVE".into(), source.into()), &ctx);
    let x = |app: &IdeDesktopApp| {
        app.native_tools
            .memory
            .models
            .results
            .get("Geometry:plot")
            .and_then(|v| v["geometry"]["objects"][0]["value"]["Ok"]["Point"]["x"].as_f64())
    };
    wait(&mut app, &ctx, |app| {
        x(app) == Some(7.) && app.controller.worker_alive()
    });
    assert_eq!(
        app.controller.session_snapshot().globals["A"].display(&ppl_vm::Settings::default()),
        "1"
    );
    for _ in 0..4 {
        frame(&mut app, &ctx, vec![]);
        assert_eq!(x(&app), Some(7.));
    }
    wait(&mut app, &ctx, |app| !app.controller.worker_alive());
    wait(&mut app, &ctx, |app| {
        x(app) == Some(7.) && !app.native_tools.calculating()
    });
    let snapshot = app.controller.session_snapshot();
    assert_eq!(snapshot.globals["A"].display(&snapshot.settings), "7");
    assert!(!snapshot.globals.contains_key("GA"));
}

#[test]
fn spreadsheet_grid_recalculates_live_ppl_view_without_reserving_another_execution_owner() {
    let (mut app, ctx) = fixture(Page::Programs);
    app.native_tools
        .memory
        .models
        .spreadsheet
        .set("A1", "=A*2")
        .unwrap();
    app.controller
        .mutate_memory(|m| {
            m.set_variable(
                "A",
                ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(1)),
            )
            .map_err(|e| e.to_string())
        })
        .unwrap();
    let source = "EXPORT SHEETLIVE() BEGIN STARTAPP(\"Spreadsheet\");A:=7;STARTVIEW(0);WAIT(0.3);RETURN A;END;";
    app.handle_native_tool_request(
        Request::SaveProgram("SHEETLIVE".into(), source.into()),
        &ctx,
    );
    app.handle_native_tool_request(Request::RunProgram("SHEETLIVE".into(), source.into()), &ctx);
    let value = |a: &IdeDesktopApp| {
        a.native_tools
            .memory
            .models
            .results
            .get("Spreadsheet")
            .and_then(|v| v["A1"]["Number"].as_f64())
    };
    wait(&mut app, &ctx, |a| {
        value(a) == Some(14.) && a.controller.worker_alive()
    });
    assert_eq!(
        app.controller.session_snapshot().globals["A"].display(&ppl_vm::Settings::default()),
        "1"
    );
    for _ in 0..4 {
        frame(&mut app, &ctx, vec![]);
        assert_eq!(value(&app), Some(14.));
        assert!(app.model_task.is_none());
    }
    wait(&mut app, &ctx, |a| !a.controller.worker_alive());
    wait(&mut app, &ctx, |a| {
        value(a) == Some(14.) && !a.native_tools.calculating()
    });
    app.controller
        .mutate_memory(|m| {
            m.set_variable(
                "A",
                ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(9)),
            )
            .map_err(|e| e.to_string())
        })
        .unwrap();
    wait(&mut app, &ctx, |a| value(a) == Some(18.));
    let s = app.controller.session_snapshot();
    assert_eq!(s.globals["Ans"].display(&s.settings), "0");
    assert!(app.native_tools.error.is_none());
}

#[path = "app_plot_settings_tests.rs"]
mod plot_settings_regressions;

#[path = "app_help_tests.rs"]
mod help_regressions;
