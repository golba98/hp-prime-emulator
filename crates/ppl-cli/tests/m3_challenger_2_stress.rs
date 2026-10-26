//! Milestone 3 Challenger 2 Adversarial Stress & Verification Test Suite
//! Authored by Challenger 2 for Milestone 3: Lazy Boot, Session Lock & Dual-Backend State Stress Testing (Features F14–F17)
//!
//! Objectives:
//! 1. Lazy boot invariant: verify `IdeDesktopApp` never spawns a session helper on creation.
//! 2. Concurrent session locking (`session.lock`) and candidate fallback probing (`ide-session-2`..`8`).
//! 3. Dual-backend state preservation under rapid switching (Native VM vs Firmware).

use fs2::FileExt;
use ppl_engine::session::{self, Command};
use ppl_graphics::Grob;
use ppl_gui::IdeDesktopApp;
use ppl_gui::ide::display::CalculatorDisplay;
use ppl_gui::ide::{IdeAction, PersistentPanelState, VmExecutionState};
use std::collections::{BTreeSet, HashSet};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;

// ============================================================================
// HELPERS FOR CANDIDATE PROBING & DIRECTORY ISOLATION
// ============================================================================

/// Emulates the exact candidate probing algorithm from `crates/ppl-gui/src/ide/firmware.rs`.
fn probe_session_candidate(base_profile: &Path) -> Result<(PathBuf, File), String> {
    let mut chosen = None;
    let candidates = std::iter::once(base_profile.to_path_buf()).chain((2..=8).map(|i| {
        let file_name = format!(
            "{}-{}",
            base_profile.file_name().unwrap().to_string_lossy(),
            i
        );
        base_profile.with_file_name(file_name)
    }));
    for cand in candidates {
        if fs::create_dir_all(&cand).is_ok() {
            if let Ok(file) = OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(cand.join("session.lock"))
            {
                if file.try_lock_exclusive().is_ok() {
                    chosen = Some((cand, file));
                    break;
                }
            }
        }
    }
    chosen.ok_or_else(|| {
        "All calculator profiles (primary and secondary 2-8) are currently locked by other sessions"
            .to_string()
    })
}

/// Emulates the explicit profile lock algorithm from `crates/ppl-gui/src/ide/firmware.rs`.
fn lock_explicit_profile(profile: &Path) -> Result<File, String> {
    fs::create_dir_all(profile).map_err(|e| e.to_string())?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(profile.join("session.lock"))
        .map_err(|e| e.to_string())?;
    file.try_lock_exclusive().map_err(|_| {
        format!(
            "Calculator profile at '{}' is locked by another running session",
            profile.display()
        )
    })?;
    Ok(file)
}

fn copy_tree(source: &Path, target: &Path) -> Result<(), String> {
    fs::create_dir_all(target).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let to = target.join(entry.file_name());
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            copy_tree(&entry.path(), &to)?;
        } else {
            fs::copy(entry.path(), to).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

// ============================================================================
// SECTION 1: LAZY BOOT INVARIANT EMPIRICAL STRESS TESTS
// ============================================================================

#[test]
fn test_lazy_boot_default_instance_zero_side_effects() {
    let app = IdeDesktopApp::default();

    // 1. Firmware client must be completely absent (None)
    assert!(
        app.firmware.is_none(),
        "Default IdeDesktopApp must not have an active firmware client"
    );

    // 2. Initial firmware status must be 'Firmware not started'
    assert_eq!(
        app.firmware_status, "Firmware not started",
        "Initial status must reflect unstarted firmware"
    );

    // 3. Backend must be Native PPL by default
    assert!(
        !app.state.firmware_backend,
        "Default backend must be Native PPL, not Firmware"
    );
}

#[test]
fn test_lazy_boot_persisted_firmware_backend_never_spawns_on_init() {
    // Simulate loading saved preferences where the user previously had firmware backend active.
    let mut app = IdeDesktopApp::default();
    let persistent = PersistentPanelState {
        firmware_backend: true,
        ..Default::default()
    };
    app.state.apply_persistent(&persistent);

    // Invariant: Even though firmware_backend is true, NO helper process is spawned on init!
    assert!(
        app.state.firmware_backend,
        "State must record firmware_backend as true from persistent preferences"
    );
    assert!(
        app.firmware.is_none(),
        "Firmware client must remain None after applying persistent settings"
    );
    assert_eq!(
        app.firmware_status, "Firmware not started",
        "Firmware status must remain unstarted"
    );
}

#[test]
fn test_lazy_boot_idle_frames_never_trigger_spawn() {
    let mut app = IdeDesktopApp::default();
    app.state.firmware_backend = true;

    // Simulate 100 UI frames without user interaction.
    // Ensure calc_input_focus is false (user is not typing into calculator).
    app.state.calc_input_focus = false;

    for _ in 0..100 {
        // Handle pending shortcuts/actions without interaction
        let ctx = egui::Context::default();
        let actions = app.state.take_actions();
        for a in actions {
            app.handle_action(a, &ctx);
        }

        assert!(
            app.firmware.is_none(),
            "Idle frame must never lazily spawn a firmware helper"
        );
        assert_eq!(app.firmware_status, "Firmware not started");
    }
}

#[test]
fn test_lazy_boot_editor_operations_never_trigger_spawn() {
    let mut app = IdeDesktopApp::default();
    let ctx = egui::Context::default();
    app.state.firmware_backend = true;

    // Perform regular IDE editor actions: typing code, adding breakpoints, formatting, toggling panels
    if let Some(tab) = app.editor.active_tab_mut() {
        tab.content = "EXPORT FOO()\nBEGIN\n  LOCAL x := 42;\n  RETURN x;\nEND;\n".into();
        tab.breakpoints.insert(3);
    }
    app.handle_action(IdeAction::FormatSource, &ctx);
    app.handle_action(IdeAction::ToggleExplorer, &ctx);
    app.handle_action(IdeAction::ToggleBottomDock, &ctx);

    // Firmware MUST remain completely unstarted
    assert!(
        app.firmware.is_none(),
        "Editor/workspace operations must never trigger firmware boot"
    );
    assert_eq!(app.firmware_status, "Firmware not started");
}

#[test]
fn test_lazy_boot_native_backend_calculator_input_never_spawns_firmware() {
    let mut app = IdeDesktopApp::default();
    let ctx = egui::Context::default();

    // With Native PPL active (firmware_backend == false)
    assert!(!app.state.firmware_backend);
    app.state.calc_input_focus = true;

    // Simulate key presses to calculator
    app.state.push_key_event(30); // Enter key
    app.state.set_key_down(30, true);

    let actions = app.state.take_actions();
    for a in actions {
        app.handle_action(a, &ctx);
    }

    assert!(
        app.firmware.is_none(),
        "Key input on Native PPL must never trigger firmware boot"
    );
    assert_eq!(app.firmware_status, "Firmware not started");
}

#[test]
fn test_lazy_boot_explicit_action_triggers_firmware_on_demand() {
    let mut app = IdeDesktopApp::default();
    let ctx = egui::Context::default();

    assert!(!app.state.firmware_backend);
    assert!(app.firmware.is_none());

    // Explicit user action: switch to Firmware backend
    app.handle_action(IdeAction::SelectBackend(true), &ctx);

    // Now firmware backend is selected AND firmware client has been lazily initiated!
    assert!(app.state.firmware_backend);
    assert!(
        app.firmware.is_some(),
        "Selecting firmware backend must lazily boot the firmware client"
    );
    assert_eq!(
        app.display.last_generation,
        u64::MAX,
        "Backend selection must reset display generation sentinel to u64::MAX"
    );
    assert!(
        app.display.texture.is_none(),
        "Backend selection must clear previous texture for clean repaint"
    );
}

// ============================================================================
// SECTION 2: CONCURRENT SESSION LOCKING & CANDIDATE FALLBACK PROBING
// ============================================================================

#[test]
fn test_session_lock_mutual_exclusion_direct() {
    let tmp = tempfile::tempdir().unwrap();
    let profile = tmp.path().join("ide-session");
    fs::create_dir_all(&profile).unwrap();

    // 1. Process A acquires exclusive lock on session.lock
    let lock_a = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(profile.join("session.lock"))
        .unwrap();
    assert!(
        lock_a.try_lock_exclusive().is_ok(),
        "Initial lock acquisition must succeed"
    );

    // 2. Process B attempts exclusive lock on the same session.lock -> MUST FAIL
    let lock_b = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(profile.join("session.lock"))
        .unwrap();
    assert!(
        lock_b.try_lock_exclusive().is_err(),
        "Concurrent lock attempt must fail with mutual exclusion"
    );

    // 3. Process A drops lock -> Process B immediately succeeds
    drop(lock_a);
    assert!(
        lock_b.try_lock_exclusive().is_ok(),
        "Lock must become available immediately after first lock is dropped"
    );
}

#[test]
fn test_candidate_fallback_probing_chain_1_through_8() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().join("ide-session");

    let mut held_locks = Vec::new();

    // Initially, candidate 1 (primary) should be chosen
    let (chosen1, lock1) = probe_session_candidate(&base).unwrap();
    assert_eq!(chosen1, base);
    held_locks.push(lock1);

    // When candidate 1 is held, candidate 2 must be chosen
    let (chosen2, lock2) = probe_session_candidate(&base).unwrap();
    assert_eq!(chosen2, tmp.path().join("ide-session-2"));
    held_locks.push(lock2);

    // When candidates 1 and 2 are held, candidate 3 must be chosen
    let (chosen3, lock3) = probe_session_candidate(&base).unwrap();
    assert_eq!(chosen3, tmp.path().join("ide-session-3"));
    held_locks.push(lock3);

    // Continue probing candidates 4 through 8
    for i in 4..=8 {
        let (chosen_i, lock_i) = probe_session_candidate(&base).unwrap();
        assert_eq!(chosen_i, tmp.path().join(format!("ide-session-{i}")));
        held_locks.push(lock_i);
    }

    // Now all 8 candidates (primary + 2..=8) are locked!
    assert_eq!(held_locks.len(), 8);

    // Probing candidate 9 MUST FAIL CLOSED
    let err = probe_session_candidate(&base).unwrap_err();
    assert_eq!(
        err,
        "All calculator profiles (primary and secondary 2-8) are currently locked by other sessions"
    );
}

#[test]
fn test_candidate_reuse_after_intermediate_candidate_released() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().join("ide-session");

    // Lock all 8 candidates
    let mut locks: Vec<File> = Vec::new();
    for i in 1..=8 {
        let path = if i == 1 {
            base.clone()
        } else {
            tmp.path().join(format!("ide-session-{i}"))
        };
        fs::create_dir_all(&path).unwrap();
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path.join("session.lock"))
            .unwrap();
        file.try_lock_exclusive().unwrap();
        locks.push(file);
    }

    // All locked
    assert!(probe_session_candidate(&base).is_err());

    // Release candidate 4 (index 3 in locks)
    drop(locks.remove(3));

    // Next candidate probe MUST pick candidate 4!
    let (chosen, lock) = probe_session_candidate(&base).unwrap();
    assert_eq!(chosen, tmp.path().join("ide-session-4"));

    // Release candidate 2 (index 1 in remaining locks)
    drop(locks.remove(1));

    // Next candidate probe MUST pick candidate 2 (lowest available candidate probed first)!
    let (chosen2, lock2) = probe_session_candidate(&base).unwrap();
    assert_eq!(chosen2, tmp.path().join("ide-session-2"));

    drop(lock);
    drop(lock2);
}

#[test]
fn test_explicit_profile_bypasses_candidate_probing() {
    let tmp = tempfile::tempdir().unwrap();
    let explicit = tmp.path().join("custom-calc-profile");

    // Process A locks explicit profile
    let lock_a = lock_explicit_profile(&explicit).unwrap();

    // Process B attempts to lock same explicit profile -> MUST NOT fall back to custom-calc-profile-2
    let result = lock_explicit_profile(&explicit);
    assert!(result.is_err());
    let err_msg = result.unwrap_err();
    assert!(
        err_msg.contains("is locked by another running session"),
        "Error message must specify profile is locked: {err_msg}"
    );

    // Verify custom-calc-profile-2 was NEVER created
    assert!(
        !tmp.path().join("custom-calc-profile-2").exists(),
        "Explicit profile must never probe fallback candidate directories"
    );

    drop(lock_a);
    // After dropping, acquisition must succeed
    assert!(lock_explicit_profile(&explicit).is_ok());
}

#[test]
fn test_concurrent_multi_threaded_candidate_contention_stress() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().join("ide-session");

    const THREAD_COUNT: usize = 8;
    let barrier = Arc::new(Barrier::new(THREAD_COUNT));
    let acquired_paths = Arc::new(Mutex::new(Vec::new()));
    let held_locks = Arc::new(Mutex::new(Vec::new()));

    let mut handles = Vec::new();

    for _ in 0..THREAD_COUNT {
        let b = barrier.clone();
        let base_clone = base.clone();
        let paths_clone = acquired_paths.clone();
        let locks_clone = held_locks.clone();

        handles.push(thread::spawn(move || {
            // Wait for all 8 threads to synchronize and start probing simultaneously
            b.wait();

            let (path, file) = probe_session_candidate(&base_clone)
                .expect("Each of the 8 threads must acquire a valid candidate");

            paths_clone.lock().unwrap().push(path);
            locks_clone.lock().unwrap().push(file);
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    let paths = acquired_paths.lock().unwrap();
    assert_eq!(
        paths.len(),
        THREAD_COUNT,
        "All 8 threads must have acquired a candidate"
    );

    // Invariant: All 8 paths must be completely unique (no race condition collisions)!
    let unique_paths: HashSet<_> = paths.iter().cloned().collect();
    assert_eq!(
        unique_paths.len(),
        THREAD_COUNT,
        "Every thread must have acquired a unique candidate path, got: {:?}",
        paths
    );

    // A 9th thread attempting to acquire candidate now must fail
    assert!(
        probe_session_candidate(&base).is_err(),
        "9th concurrent candidate probe must fail when all 8 are locked"
    );
}

#[test]
fn test_stale_pending_directory_cleanup() {
    let tmp = tempfile::tempdir().unwrap();
    let profile = tmp.path().join("ide-session");
    let factory = tmp.path().join("factory");

    // Setup dummy factory directory
    fs::create_dir_all(factory.join("programs/misc")).unwrap();
    fs::write(
        factory.join("programs/misc/armfir.dat"),
        b"ARM_FIRMWARE_DAT",
    )
    .unwrap();
    fs::write(factory.join("meta.txt"), b"FACTORY_METADATA").unwrap();

    // Create a stale, corrupt A.pending directory in the profile
    let temporary = profile.join("A.pending");
    fs::create_dir_all(&temporary).unwrap();
    fs::write(temporary.join("corrupt_partial.dat"), b"CORRUPT_BYTES").unwrap();

    // Replicate the supervisor provisioning logic:
    assert!(!profile.join("A").exists());
    assert!(profile.join("A.pending").exists());

    // 1. Stale pending directory cleanup
    let _ = fs::remove_dir_all(&temporary);
    assert!(!temporary.exists(), "Stale A.pending must be removed");

    // 2. Clean copy and rename
    copy_tree(&factory, &temporary).unwrap();
    fs::rename(&temporary, profile.join("A")).unwrap();

    // Verify provisioned directory
    assert!(profile.join("A").exists());
    assert!(profile.join("A/programs/misc/armfir.dat").is_file());
    assert_eq!(
        fs::read(profile.join("A/programs/misc/armfir.dat")).unwrap(),
        b"ARM_FIRMWARE_DAT"
    );
    assert!(
        !profile.join("A/corrupt_partial.dat").exists(),
        "Corrupt pending files must not bleed into final profile"
    );
}

// ============================================================================
// SECTION 3: DUAL-BACKEND STATE PRESERVATION UNDER RAPID SWITCHING
// ============================================================================

#[test]
fn test_dual_backend_state_preservation_basic() {
    let ctx = egui::Context::default();
    let mut app = IdeDesktopApp::default();

    // Setup Native PPL state
    if let Some(tab) = app.editor.active_tab_mut() {
        tab.content = "EXPORT FOO()\nBEGIN\n  PRINT(\"Native State\");\nEND;\n".into();
        tab.breakpoints.insert(2);
        tab.breakpoints.insert(4);
    }
    app.console.print_diagnostic("Native log line 1");
    app.console.print_diagnostic("Native log line 2");
    app.state.vm_state = VmExecutionState::Paused;

    // Invariant checks on initial Native PPL state
    assert!(!app.state.firmware_backend);
    assert_eq!(app.state.vm_state, VmExecutionState::Paused);

    // Switch to G1 Firmware backend
    app.handle_action(IdeAction::SelectBackend(true), &ctx);
    assert!(app.state.firmware_backend);
    assert_eq!(app.state.vm_state, VmExecutionState::Idle);

    // Verify Native VM state details are preserved across the switch:
    assert_eq!(
        app.editor.active_tab().unwrap().breakpoints,
        BTreeSet::from([2, 4]),
        "Breakpoints must be preserved when switching to Firmware"
    );
    let console_text = app.console.plain_text();
    assert!(
        console_text.contains("Native log line 1") && console_text.contains("Native log line 2"),
        "Console output history must be preserved when switching to Firmware"
    );
    assert_eq!(
        app.display.last_generation,
        u64::MAX,
        "Display generation must be reset to sentinel u64::MAX on backend switch"
    );
    assert!(app.display.texture.is_none());

    // Simulate Firmware state change
    app.state.vm_state = VmExecutionState::Running;
    app.console.print_diagnostic("Firmware log line 3");

    // Switch back to Native PPL backend
    app.handle_action(IdeAction::SelectBackend(false), &ctx);
    assert!(!app.state.firmware_backend);

    // Invariant: Native VM state restored to Paused!
    assert_eq!(
        app.state.vm_state,
        VmExecutionState::Paused,
        "Native VM state must be faithfully restored from native_vm_state"
    );
    assert_eq!(
        app.editor.active_tab().unwrap().breakpoints,
        BTreeSet::from([2, 4]),
        "Breakpoints must remain intact after switching back to Native PPL"
    );
    let console_text_after = app.console.plain_text();
    assert!(
        console_text_after.contains("Native log line 1")
            && console_text_after.contains("Native log line 2")
            && console_text_after.contains("Firmware log line 3"),
        "All console history must persist across multiple backend switches"
    );
    assert_eq!(
        app.display.last_generation,
        u64::MAX,
        "Display generation must reset to u64::MAX on switch back"
    );
}

#[test]
fn test_dual_backend_rapid_switching_stress_loop_500() {
    let ctx = egui::Context::default();
    let mut app = IdeDesktopApp::default();

    if let Some(tab) = app.editor.active_tab_mut() {
        tab.content = "EXPORT STRESS()\nBEGIN\n  PRINT(\"Stress\");\nEND;\n".into();
        tab.breakpoints.insert(1);
        tab.breakpoints.insert(3);
        tab.breakpoints.insert(5);
    }
    app.console.print_diagnostic("Persistent anchor log");

    // Stress test: 500 rapid alternating switches between Native and Firmware
    for i in 0..500 {
        let to_firmware = i % 2 == 0;
        app.handle_action(IdeAction::SelectBackend(to_firmware), &ctx);

        assert_eq!(
            app.state.firmware_backend, to_firmware,
            "Iteration {i}: Backend flag must match requested mode"
        );
        assert_eq!(
            app.display.last_generation,
            u64::MAX,
            "Iteration {i}: Display generation must be u64::MAX"
        );
        assert!(
            app.display.texture.is_none(),
            "Iteration {i}: Texture must be None after backend selection"
        );

        // Breakpoints must never be corrupted or dropped
        let tab = app.editor.active_tab().unwrap();
        assert_eq!(
            tab.breakpoints,
            BTreeSet::from([1, 3, 5]),
            "Iteration {i}: Breakpoints corrupted during rapid switching"
        );
    }

    // Verify console remains complete
    assert!(
        app.console.plain_text().contains("Persistent anchor log"),
        "Console output was wiped or corrupted during 500 rapid switches"
    );
}

#[test]
fn test_monotonic_frame_sequencing_and_generation_sentinel_reset() {
    let ctx = egui::Context::default();
    let mut display = CalculatorDisplay::new();

    let pixels_frame1 = vec![0x00FF_0000u32; 320 * 240];
    let pixels_stale = vec![0x0000_FF00u32; 320 * 240];
    let pixels_frame2 = vec![0x0000_00FFu32; 320 * 240];

    // 1. Initial frame with generation 10 is accepted
    display.update_xrgb(&ctx, &pixels_frame1, 10);
    assert_eq!(display.last_generation, 10);
    assert!(display.texture.is_some());

    // 2. Stale frame with generation 5 (< 10) must be DROPPED
    display.update_xrgb(&ctx, &pixels_stale, 5);
    assert_eq!(
        display.last_generation, 10,
        "Stale frame (gen 5 < 10) must be rejected"
    );

    // 3. Duplicate frame with generation 10 (== 10) must be SKIPPED
    display.update_xrgb(&ctx, &pixels_stale, 10);
    assert_eq!(
        display.last_generation, 10,
        "Duplicate frame (gen 10 == 10) must be deduplicated"
    );

    // 4. Newer frame with generation 15 (> 10) must be ACCEPTED
    display.update_xrgb(&ctx, &pixels_frame2, 15);
    assert_eq!(
        display.last_generation, 15,
        "Newer frame (gen 15 > 10) must be accepted"
    );

    // 5. Simulate Backend Switch: sentinel reset to u64::MAX and texture cleared
    display.last_generation = u64::MAX;
    display.texture = None;

    // 6. First frame from newly switched backend arrives with generation 1 (much lower than 15!)
    // Invariant: MUST BE ACCEPTED because last_generation == u64::MAX sentinel!
    let pixels_switched = vec![0x00FF_FFFFu32; 320 * 240];
    display.update_xrgb(&ctx, &pixels_switched, 1);
    assert_eq!(
        display.last_generation, 1,
        "First frame after backend switch (gen 1) must be accepted via u64::MAX sentinel"
    );
    assert!(display.texture.is_some());

    // 7. Subsequent frames on new backend obey normal monotonicity
    display.update_xrgb(&ctx, &pixels_stale, 1); // duplicate
    assert_eq!(display.last_generation, 1);

    display.update_xrgb(&ctx, &pixels_frame1, 2); // newer
    assert_eq!(display.last_generation, 2);
}

#[test]
fn test_update_xrgb_rejects_corrupted_dimension_frames() {
    let ctx = egui::Context::default();
    let mut display = CalculatorDisplay::new();

    // Frame with fewer pixels than 320 * 240
    let truncated_pixels = vec![0x00FF_0000u32; 100];
    display.update_xrgb(&ctx, &truncated_pixels, 1);
    assert_eq!(
        display.last_generation, 0,
        "Truncated frame must be rejected"
    );
    assert!(display.texture.is_none());

    // Frame with more pixels than 320 * 240
    let oversized_pixels = vec![0x00FF_0000u32; 320 * 240 + 1];
    display.update_xrgb(&ctx, &oversized_pixels, 1);
    assert_eq!(
        display.last_generation, 0,
        "Oversized frame must be rejected"
    );
    assert!(display.texture.is_none());
}

#[test]
fn test_dual_backend_grob_and_xrgb_interleaved_display_refresh() {
    let ctx = egui::Context::default();
    let mut display = CalculatorDisplay::new();

    // 1. Native Grob display frame (gen 100)
    let grob1 = Grob::new(320, 240, 0);
    display.update_from_grob(&ctx, &grob1, 100);
    assert_eq!(display.last_generation, 100);
    assert!(display.texture.is_some());

    // Deduplication check on Grob
    display.update_from_grob(&ctx, &grob1, 100);
    assert_eq!(display.last_generation, 100);

    // 2. Backend switch to Firmware
    display.last_generation = u64::MAX;
    display.texture = None;

    // 3. Firmware XRGB frame (gen 5) accepted despite 5 < 100
    let pixels_xrgb = vec![0x0080_8080u32; 320 * 240];
    display.update_xrgb(&ctx, &pixels_xrgb, 5);
    assert_eq!(display.last_generation, 5);
    assert!(display.texture.is_some());

    // 4. Backend switch back to Native PPL
    display.last_generation = u64::MAX;
    display.texture = None;

    // 5. Native Grob frame (gen 3) accepted despite 3 < 5
    let grob2 = Grob::new(320, 240, 0);
    display.update_from_grob(&ctx, &grob2, 3);
    assert_eq!(display.last_generation, 3);
    assert!(display.texture.is_some());
}

#[test]
fn test_wire_framing_load_source_and_frame_request_integrity() {
    // Verify that Command::LoadSource and Command::FrameRequest serialize/deserialize
    // faithfully without corruption across protocol pipes
    let cmd_load = Command::LoadSource {
        request: 42,
        source: "EXPORT TEST()\nBEGIN\n  PRINT(\"Hello\");\nEND;\n".into(),
    };
    let mut buf = Vec::new();
    session::write_command(&mut buf, &cmd_load).unwrap();

    let mut cursor = std::io::Cursor::new(buf);
    let decoded = session::read_command(&mut cursor).unwrap();
    match decoded {
        Command::LoadSource { request, source } => {
            assert_eq!(request, 42);
            assert!(source.contains("PRINT(\"Hello\")"));
        }
        other => panic!("Expected Command::LoadSource, got {:?}", other),
    }

    let cmd_frame = Command::FrameRequest;
    let mut buf2 = Vec::new();
    session::write_command(&mut buf2, &cmd_frame).unwrap();

    let mut cursor2 = std::io::Cursor::new(buf2);
    let decoded2 = session::read_command(&mut cursor2).unwrap();
    match decoded2 {
        Command::FrameRequest => {}
        other => panic!("Expected Command::FrameRequest, got {:?}", other),
    }
}
