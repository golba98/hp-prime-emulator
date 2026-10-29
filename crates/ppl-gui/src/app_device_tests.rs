//! Send to calculator, driven through the real [`IdeDesktopApp`]. No test
//! here reaches USB: the flow is stopped before a device job, and the
//! device link refuses to open a calculator under `cfg(test)` anyway.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::*;
use crate::ide::send::{SendOutcome, SendRequest, SendStage, payload_sha256};

const VALID: &str = "EXPORT DEMO()\nBEGIN\n  PRINT(\"Hello\");\nEND;\n";
const UNCLOSED_IF: &str = "EXPORT DEMO()\nBEGIN\n  LOCAL a;\n  IF a == 1 THEN\n";
const UNTESTED: &str = "EXPORT DEMO()\nBEGIN\n  RETURN SOLVE(X^2-1, X, 2);\nEND;\n";

/// A program saved in its own temporary folder, open in a fresh app.
fn app_with_file(test: &str, src: &str) -> (IdeDesktopApp, PathBuf) {
    let dir = std::env::temp_dir().join(format!("ppl-send-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("DEMO.ppl");
    std::fs::write(&path, src).unwrap();
    let mut app = IdeDesktopApp {
        program_store: ppl_programs::delete_store::Store::new(&dir.join("journal")),
        ..Default::default()
    };
    app.open_file(path.clone());
    (app, path)
}

fn frame(app: &mut IdeDesktopApp, ctx: &egui::Context) {
    let _ = ctx.run(Default::default(), |ctx| {
        app.schedule_diagnostics(ctx);
        app.poll_device(ctx);
    });
}

/// Runs frames until the send leaves the "checking" stage.
fn settle(app: &mut IdeDesktopApp, ctx: &egui::Context) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while matches!(app.send.stage, SendStage::WaitingForCheck(_)) {
        frame(app, ctx);
        assert!(Instant::now() < deadline, "check never settled");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn errors_block_the_send_and_show_problems() {
    let ctx = egui::Context::default();
    let (mut app, _) = app_with_file("invalid", UNCLOSED_IF);
    app.begin_send(&ctx);
    settle(&mut app, &ctx);
    assert!(matches!(app.send.stage, SendStage::Idle));
    assert!(app.state.status_message.starts_with("Not sent: fix"));
    assert_eq!(app.state.active_dock_tab, DockTab::Problems);
    assert!(app.device.busy().is_none());
    let message = app.state.status_message.clone();
    frame(&mut app, &ctx);
    assert_eq!(app.state.status_message, message);
}

#[test]
fn unsaved_edits_are_saved_before_sending() {
    let ctx = egui::Context::default();
    let (mut app, path) = app_with_file("dirty", VALID);
    let tab = app.editor.active_tab_mut().unwrap();
    tab.content = VALID.replace("Hello", "Saved first");
    tab.modified = true;
    app.begin_send(&ctx);
    assert!(!app.editor.active_tab().unwrap().modified);
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("Saved first")
    );
}

#[test]
fn first_send_asks_for_a_valid_calculator_name() {
    let ctx = egui::Context::default();
    let (mut app, path) = app_with_file("first", VALID);
    app.begin_send(&ctx);
    settle(&mut app, &ctx);
    match &app.send.stage {
        SendStage::AskName { name, path: p, .. } => {
            assert_eq!(name, "DEMO");
            assert_eq!(p, &path);
        }
        other => panic!("expected the name dialog, got {other:?}"),
    }
    assert!(app.device.busy().is_none());
}

#[test]
fn untested_features_are_confirmed_before_any_device_job() {
    let ctx = egui::Context::default();
    let (mut app, path) = app_with_file("untested", UNTESTED);
    app.state.send_names.insert(path, "DEMO".into());
    app.begin_send(&ctx);
    settle(&mut app, &ctx);
    match &app.send.stage {
        SendStage::ConfirmUntested { request, untested } => {
            assert_eq!(request.name, "DEMO");
            assert_eq!(request.sha256, payload_sha256(UNTESTED));
            assert!(untested[0].starts_with("Line 3:"), "{untested:?}");
        }
        other => panic!("expected the untested-features warning, got {other:?}"),
    }
    assert!(app.device.busy().is_none());
}

fn prepare_replace_dialog(app: &mut IdeDesktopApp, path: PathBuf) -> String {
    let tab = app.editor.active_tab().unwrap().id;
    let image =
        ppl_programs::ProgramImage::new("DEMO", ppl_programs::Source::text(VALID, "DEMO").unwrap())
            .unwrap();
    let plan = ppl_programs::replacement::Plan {
        schema_version: 1,
        operation_id: "ZZT_D12345".into(),
        device: ppl_programs::DeviceIdentity {
            serial_id: "test".into(),
            usb_pid: 0x1541,
            firmware_build: None,
            boot_version: None,
        },
        name: "DEMO".into(),
        old_sha256: payload_sha256(VALID),
        new_sha256: payload_sha256(VALID),
        sizes: image.sizes.clone(),
        atomic: false,
        checks: ppl_engine::checking::check(image.source.clone(), None, &Default::default()),
    };
    let bytes: Vec<_> = VALID.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let op: ppl_programs::delete_store::Operation = serde_json::from_value(serde_json::json!({
        "schema_version":1, "id":plan.operation_id, "device_serial_id":"test", "usb_pid":0x1541,
        "control":ppl_device::DEFAULT_SETTINGS_ANCHOR, "state":"prepared", "mechanism":"usb_delete",
        "created_utc":"test", "updated_utc":"test", "helper_uploaded":false, "artifacts":[],
        "cleanup_state":"not_required", "runs_dir":app.program_store.root, "class":"user",
        "targets":[{"name":"DEMO", "ty":"program", "sha256":plan.old_sha256, "bytes":bytes.len(), "status":"not_submitted"}]
    })).unwrap();
    let _lock = app.program_store.lock().unwrap();
    app.program_store
        .create(
            &op,
            &[ppl_device::Backup {
                ty: ppl_device::FileType::Program,
                name: "DEMO".into(),
                data: bytes,
                crc_ok: true,
            }],
        )
        .unwrap();
    std::fs::write(
        app.program_store
            .directory(&op.id)
            .unwrap()
            .join("replacement.json"),
        serde_json::to_vec(&plan).unwrap(),
    )
    .unwrap();
    app.send.stage = SendStage::ConfirmReplace {
        request: SendRequest {
            tab,
            path,
            name: "DEMO".into(),
            source: VALID.into(),
            sha256: payload_sha256(VALID),
        },
        plan: Box::new(plan),
    };
    op.id
}

#[test]
fn editing_during_replace_confirmation_cancels_it() {
    let ctx = egui::Context::default();
    let (mut app, path) = app_with_file("replace", VALID);
    let id = prepare_replace_dialog(&mut app, path);
    frame(&mut app, &ctx);
    assert!(matches!(app.send.stage, SendStage::ConfirmReplace { .. }));

    app.editor.active_tab_mut().unwrap().content = VALID.replace("Hello", "Edited");
    frame(&mut app, &ctx);
    assert!(matches!(
        &app.send.stage,
        SendStage::Finished(SendOutcome::Failed { message, .. }) if message.contains("changed")
    ));
    assert!(app.device.busy().is_none());
    let op = app.program_store.load(&id).unwrap();
    assert_eq!(op.state, "not_submitted");
    assert_eq!(
        ppl_device::program::utf16(&app.program_store.backups(&op).unwrap()[0].data).unwrap(),
        VALID
    );

    // The next Send must check the new source rather than reuse the old plan.
    app.editor.active_tab_mut().unwrap().content = UNCLOSED_IF.into();
    app.editor.active_tab_mut().unwrap().modified = true;
    app.begin_send(&ctx);
    settle(&mut app, &ctx);
    assert!(matches!(app.send.stage, SendStage::Idle));
    assert!(app.state.status_message.starts_with("Not sent: fix"));
}

#[test]
fn cancelling_the_dialog_retires_the_durable_plan_without_usb() {
    let ctx = egui::Context::default();
    let (mut app, path) = app_with_file("cancel", VALID);
    let id = prepare_replace_dialog(&mut app, path);
    app.send.stage = SendStage::Idle;
    app.handle_send_event(
        crate::ide::send::SendEvent::CancelReplacement(id.clone()),
        &ctx,
    );
    assert_eq!(app.program_store.load(&id).unwrap().state, "not_submitted");
    assert!(app.device.busy().is_none());
}

#[test]
fn cancellation_persistence_failure_exposes_recovery() {
    let ctx = egui::Context::default();
    let (mut app, path) = app_with_file("cancel-failure", VALID);
    let id = prepare_replace_dialog(&mut app, path);
    std::fs::remove_file(
        app.program_store
            .directory(&id)
            .unwrap()
            .join("replacement.json"),
    )
    .unwrap();
    app.handle_send_event(
        crate::ide::send::SendEvent::CancelReplacement(id.clone()),
        &ctx,
    );
    assert!(
        matches!(&app.send.stage, SendStage::Finished(SendOutcome::Failed { recovery_id:Some(r), .. }) if r == &id)
    );
    assert_eq!(app.program_store.load(&id).unwrap().state, "prepared");
    assert!(app.device.busy().is_none());
}

#[test]
fn workspace_and_send_names_survive_a_restart() {
    let (mut app, path) = app_with_file("persist", VALID);
    let folder = path.parent().unwrap().to_path_buf();
    app.state.workspace_root = Some(folder.clone());
    app.state.send_names.insert(path.clone(), "DEMO".into());
    let json = serde_json::to_string(&app.state.to_persistent()).unwrap();

    let mut restarted = crate::ide::IdeAppState::default();
    restarted.apply_persistent(&serde_json::from_str(&json).unwrap());
    assert_eq!(restarted.workspace_root.as_ref(), Some(&folder));
    assert_eq!(restarted.recent_files, vec![path.clone()]);
    assert_eq!(
        restarted.send_names.get(&path).map(String::as_str),
        Some("DEMO")
    );

    // Older layout files without these fields still load.
    let old: crate::ide::PersistentPanelState = serde_json::from_str("{}").unwrap();
    assert!(old.send_names.is_empty() && old.workspace_root.is_none());
}

#[test]
fn calculator_retrieval_opens_a_new_unsaved_tab_without_touching_disk() {
    let ctx = egui::Context::default();
    let (mut app, path) = app_with_file("read-copy", VALID);
    let before = app.editor.tabs.len();
    app.device.inject(
        crate::ide::device::Purpose::ReadProgram,
        Ok(crate::ide::device::JobOutput::Program {
            name: "DEMO".into(),
            source: VALID.replace("Hello", "Calculator copy"),
        }),
    );
    frame(&mut app, &ctx);
    assert_eq!(app.editor.tabs.len(), before + 1);
    let tab = app.editor.active_tab().unwrap();
    assert!(tab.path.is_none() && tab.modified);
    assert!(tab.content.contains("Calculator copy"));
    assert_eq!(std::fs::read_to_string(path).unwrap(), VALID);
}

#[test]
fn refresh_and_retrieval_errors_are_visible() {
    use crate::ide::device::{JobError, Purpose};
    let ctx = egui::Context::default();
    let (mut app, _) = app_with_file("read-errors", VALID);
    let before = app.editor.tabs.len();
    for purpose in [Purpose::ListPrograms, Purpose::ReadProgram] {
        app.device.inject(
            purpose,
            Err(JobError {
                message: "USB disconnected while reading".into(),
                hint: None,
            }),
        );
        frame(&mut app, &ctx);
        match purpose {
            Purpose::ListPrograms => assert!(
                app.explorer
                    .calculator
                    .error
                    .as_ref()
                    .unwrap()
                    .contains("USB disconnected")
            ),
            _ => assert!(
                app.state
                    .status_message
                    .contains("Could not read the program: USB disconnected")
            ),
        }
    }
    assert_eq!(app.editor.tabs.len(), before);
}

#[test]
fn comparison_uses_selected_firmware_frame_and_never_falls_back_to_native() {
    use crate::ide::firmware::{FirmwareClient, Frame};
    let mut app = IdeDesktopApp::default();
    *app.controller.shared.screen.lock().unwrap() = ppl_graphics::Grob::screen();
    app.controller
        .shared
        .screen_generation
        .store(3, std::sync::atomic::Ordering::Release);
    let (native, identity) = app.capture_compare_frame().unwrap();
    assert!(!identity.firmware);
    assert_eq!(identity.generation, 3);
    assert_eq!(native, app.state.native.screen().unwrap());
    app.state
        .native
        .set_view(crate::ide::native::NativeView::ProgramOutput);
    let (native, identity) = app.capture_compare_frame().unwrap();
    assert!(!identity.firmware);
    assert_eq!(native.pixels()[0], ppl_graphics::WHITE);
    app.state.firmware_backend = true;
    assert!(app.capture_compare_frame().is_none());
    assert!(app.current_compare_identity().is_none());
    app.firmware = Some(FirmwareClient::with_test_frame(Frame {
        generation: 42,
        micros: 1,
        pixels: vec![0xff00_001f; 320 * 240].into(),
    }));
    let (firmware, identity) = app.capture_compare_frame().unwrap();
    assert!(identity.firmware);
    assert_eq!(identity.generation, 42);
    assert_eq!(firmware.pixels()[0], 0x0000_001f);
    assert_eq!(app.current_compare_identity(), Some(identity));
    app.firmware
        .as_ref()
        .unwrap()
        .alive
        .store(false, std::sync::atomic::Ordering::Release);
    assert!(app.capture_compare_frame().is_none());
}

#[test]
fn comparison_visibility_survives_restart_without_captures_or_hardware_access() {
    let mut app = IdeDesktopApp::default();
    app.state.show_compare = true;
    let json = serde_json::to_string(&app.state.to_persistent()).unwrap();
    let restored: crate::ide::PersistentPanelState = serde_json::from_str(&json).unwrap();
    let mut restarted = IdeDesktopApp::default();
    restarted.state.apply_persistent(&restored);
    assert!(restarted.state.show_compare);
    assert!(restarted.device.busy().is_none());
    let old_preferences: crate::ide::PersistentPanelState = serde_json::from_str("{}").unwrap();
    assert!(!old_preferences.show_compare);
    restarted.state.show_compare = false;
    assert!(!restarted.state.to_persistent().show_compare);
}
