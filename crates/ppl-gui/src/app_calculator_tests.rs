//! Native output dismissal preserves the completed run and calculator routing.
use super::*;
use crate::ide::{VmExecutionState, layout::WorkspaceLayout};

fn wait_until(app: &mut IdeDesktopApp, predicate: impl Fn(&IdeDesktopApp) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        app.poll_execution();
        if predicate(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Native run did not reach expected state: {:?}",
            app.state.vm_state
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn key_input(key: egui::Key, modifiers: egui::Modifiers) -> egui::RawInput {
    let mut input = egui::RawInput {
        modifiers,
        ..Default::default()
    };
    input.events.push(egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    });
    input
}

#[test]
fn native_finished_output_dismissal_preserves_evidence_and_invalidates_comparison() {
    let ctx = egui::Context::default();
    let mut app = IdeDesktopApp::default();
    let source = "EXPORT TEST() BEGIN PRINT(\"retained output\"); RETURN 42; END;";
    app.editor.active_tab_mut().unwrap().content = source.into();
    app.state.layout = WorkspaceLayout::Calculator;
    app.state.calc_input_focus = true;
    app.handle_action(IdeAction::RunProgram, &ctx);
    wait_until(&mut app, |a| {
        a.state.vm_state == VmExecutionState::Terminated
    });
    let history = app.console.full_text();
    let outcome = format!("{:?}", app.controller.outcome());
    let raw_generation = app
        .controller
        .shared
        .screen_generation
        .load(std::sync::atomic::Ordering::Acquire);
    let last_run = app.controller.shared.get_frame();
    let (old_frame, old_identity) = app.capture_compare_frame().unwrap();
    app.compare.begin(old_frame, old_identity.clone());

    let _ = ctx.run(key_input(egui::Key::Escape, egui::Modifiers::NONE), |ctx| {
        app.forward_calc_keys(ctx);
    });
    let actions = app.state.take_actions();
    assert!(actions.is_empty());
    for _ in 0..5 {
        app.poll_execution();
    }

    assert_eq!(app.state.vm_state, VmExecutionState::Idle);
    assert_eq!(
        app.state.native.view,
        crate::ide::native::NativeView::Catalog
    );
    assert_eq!(app.state.layout, WorkspaceLayout::Calculator);
    assert_eq!(app.editor.active_tab().unwrap().content, source);
    assert_eq!(app.console.full_text(), history);
    assert_eq!(format!("{:?}", app.controller.outcome()), outcome);
    assert_eq!(app.controller.shared.get_frame(), last_run);
    assert_eq!(
        app.controller
            .shared
            .screen_generation
            .load(std::sync::atomic::Ordering::Acquire),
        raw_generation
    );
    let (ready, identity) = app.capture_compare_frame().unwrap();
    assert_ne!(ready, last_run);
    assert!(identity.generation > old_identity.generation);
    assert_eq!(app.display.last_generation, identity.generation);
    assert_eq!(app.current_compare_identity(), Some(identity));
    assert!(
        app.compare
            .is_stale(app.current_compare_identity().as_ref())
    );
    assert!(!app.state.can_dismiss_output());

    app.handle_action(IdeAction::RestartProgram, &ctx);
    wait_until(&mut app, |a| {
        a.state.vm_state == VmExecutionState::Terminated
    });
    assert_eq!(
        app.state.native.view,
        crate::ide::native::NativeView::ProgramOutput
    );
    assert!(app.state.can_dismiss_output());
    assert_eq!(app.console.full_text().matches("returned 42").count(), 2);
}

#[test]
fn native_running_escape_cancels_input_without_dismissing_output() {
    let ctx = egui::Context::default();
    let mut app = IdeDesktopApp::default();
    app.editor.active_tab_mut().unwrap().content =
        "EXPORT TEST() BEGIN LOCAL n:=7; LOCAL ok:=INPUT(n,\"Value\",\"N\"); RETURN {ok,n}; END;"
            .into();
    app.state.calc_input_focus = true;
    app.handle_action(IdeAction::RunProgram, &ctx);
    wait_until(&mut app, |a| {
        a.controller.worker_alive() && a.native_display_generation() > 0
    });
    let _ = ctx.run(key_input(egui::Key::Escape, egui::Modifiers::NONE), |ctx| {
        app.menu_bar.handle_shortcuts(ctx, &mut app.state);
        assert!(app.state.take_actions().is_empty());
        app.forward_calc_keys(ctx);
    });
    wait_until(&mut app, |a| {
        a.state.vm_state == VmExecutionState::Terminated
    });
    assert!(
        matches!(app.controller.outcome(), Some(WorkerOutcome::Success { value_str, .. }) if value_str == "{0,7}")
    );
    assert_eq!(
        app.state.native.view,
        crate::ide::native::NativeView::ProgramOutput
    );
}

#[test]
fn shift_f5_stops_a_waiting_native_program_with_calculator_focus() {
    let ctx = egui::Context::default();
    let mut app = IdeDesktopApp::default();
    app.editor.active_tab_mut().unwrap().content =
        "EXPORT TEST() BEGIN PRINT(\"waiting\"); WAIT(0); RETURN 99; END;".into();
    app.state.calc_input_focus = true;
    app.handle_action(IdeAction::RunProgram, &ctx);
    wait_until(&mut app, |a| {
        a.controller.worker_alive() && a.native_display_generation() > 0
    });
    let _ = ctx.run(key_input(egui::Key::F5, egui::Modifiers::SHIFT), |ctx| {
        app.menu_bar.handle_shortcuts(ctx, &mut app.state);
    });
    let actions = app.state.take_actions();
    assert!(matches!(actions.as_slice(), [IdeAction::StopProgram]));
    app.handle_action(actions.into_iter().next().unwrap(), &ctx);
    wait_until(&mut app, |a| {
        !a.controller.worker_alive() && !a.native_stopping
    });
    assert_eq!(app.state.vm_state, VmExecutionState::Idle);
    assert!(app.console.full_text().contains("Stop requested by user"));
    assert!(!app.console.full_text().contains("returned 99"));
}

#[test]
fn native_dismissal_is_ignored_while_running_or_on_firmware() {
    let ctx = egui::Context::default();
    for (firmware, vm_state) in [
        (false, VmExecutionState::Running),
        (false, VmExecutionState::Paused),
        (true, VmExecutionState::Terminated),
    ] {
        let mut app = IdeDesktopApp::default();
        app.state.firmware_backend = firmware;
        app.state.vm_state = vm_state.clone();
        let frame = app.controller.shared.get_frame();
        app.handle_action(IdeAction::DismissOutput, &ctx);
        assert_eq!(app.state.vm_state, vm_state);
        assert_eq!(
            app.state.native.view,
            crate::ide::native::NativeView::Catalog
        );
        assert_eq!(app.controller.shared.get_frame(), frame);
    }
}
