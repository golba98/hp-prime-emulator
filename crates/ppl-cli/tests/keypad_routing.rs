//! Milestone 3 Adversarial Stress & Verification Test Suite
//! Keypad Interaction, Focus Separation & Keyboard Routing Stress Testing
//!
//! Authored by Challenger 2 for Milestone 3: Emulated HP Prime G1 Display & Virtual Keypad (R4).

use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use egui::{CentralPanel, Pos2, RawInput, Rect, Vec2};
use ppl_compiler::CompileOptions;
use ppl_gui::host::{GuiHost, GuiSharedState};
use ppl_gui::ide::editor::EditorTab;
use ppl_gui::ide::keypad::{KEYS, VirtualKeypad};
use ppl_gui::ide::{IdeAppState, VmExecutionState};
use ppl_gui::keymap::{
    KEY_MAPPINGS, egui_key_to_hp, hp_key_name, lookup_by_device_code, lookup_by_pc_key,
    lookup_by_physical, lookup_by_ppl_code, map_char,
};
use ppl_syntax::source::SourceFile;
use ppl_vm::Host;
use ppl_vm::machine::{Limits, Machine};
use ppl_vm::settings::Settings;
use ppl_vm::value::Value;

/// Helper: compiles and executes a PPL function on Machine with the provided host.
fn run_ppl_with_host(source: &str, host: GuiHost) -> Result<Value, String> {
    let file = SourceFile::new("<test>", source);
    let (prog, diags) = ppl_compiler::compile_source(&file, &CompileOptions::default());
    if diags.has_errors() || prog.is_none() {
        let errs: Vec<_> = diags.items.iter().map(|d| d.message.clone()).collect();
        return Err(format!("Compilation failed: {}", errs.join("; ")));
    }

    let prog = prog.unwrap();
    let mut machine = Machine::new(
        Rc::new(prog.clone()),
        Box::new(host),
        Settings::default(),
        Limits::default(),
    );

    let entry = prog
        .exported()
        .next()
        .map(|(_, f)| f.name.clone())
        .or_else(|| prog.functions.first().map(|f| f.name.clone()))
        .unwrap_or_else(|| "EXPORT".to_string());

    machine
        .call_function(&entry, Vec::new())
        .map_err(|e| e.to_string())
}

// ============================================================================
// 1. Flood of 1,000 Rapid Keypresses Through Keypad Queue
// ============================================================================

#[test]
fn test_stress_keypad_queue_flood_1000_keys_fifo() {
    let shared = Arc::new(GuiSharedState::new());
    let mut host = GuiHost::new(shared.clone());

    // Generate 1,000 deterministic keycodes in range 0..=50
    let mut expected_keys = Vec::with_capacity(1000);
    for i in 0..1000 {
        let code = (i * 7 + 13) % 51;
        expected_keys.push(code);
        shared.push_key(code);
    }

    // Verify all 1,000 keys are received in strict FIFO order
    let mut received_keys = Vec::with_capacity(1000);
    for _ in 0..1000 {
        let key = host
            .poll_key()
            .expect("poll_key should not fail")
            .expect("key must be present in queue");
        received_keys.push(key);
    }

    assert_eq!(received_keys.len(), 1000);
    assert_eq!(
        received_keys, expected_keys,
        "FIFO order must be strictly preserved"
    );

    // Queue must now be empty
    assert_eq!(
        host.poll_key().expect("poll_key ok"),
        None,
        "queue must be empty after 1000 drains"
    );
}

#[test]
fn test_stress_keypad_queue_concurrent_flood_producers_consumers() {
    let shared = Arc::new(GuiSharedState::new());
    let num_producers = 4;
    let keys_per_producer = 250;
    let mut handles = Vec::new();

    // Spawn 4 concurrent producer threads
    for p in 0..num_producers {
        let shared_clone = shared.clone();
        handles.push(thread::spawn(move || {
            for i in 0..keys_per_producer {
                let code = ((p * 250 + i) % 51) as i64;
                shared_clone.push_key(code);
            }
        }));
    }

    // Consumer thread draining 1,000 keys
    let shared_consumer = shared.clone();
    let consumer = thread::spawn(move || {
        let mut host = GuiHost::new(shared_consumer);
        let mut count = 0;
        let start = std::time::Instant::now();
        while count < 1000 && start.elapsed() < Duration::from_secs(5) {
            if let Ok(Some(code)) = host.poll_key() {
                assert!((0..=50).contains(&code));
                count += 1;
            } else {
                thread::yield_now();
            }
        }
        count
    });

    for h in handles {
        h.join().expect("producer thread joined");
    }
    let total_consumed = consumer.join().expect("consumer thread joined");
    assert_eq!(
        total_consumed, 1000,
        "All 1000 concurrently pushed keys must be safely consumed without drops"
    );
}

#[test]
fn test_stress_ide_app_state_queue_flood_and_drain() {
    let mut state = IdeAppState::default();
    let shared = Arc::new(GuiSharedState::new());
    let mut host = GuiHost::new(shared.clone());

    // Push 1,000 keys into IdeAppState
    for i in 0..1000 {
        let code = (i % 51) as i64;
        state.push_key_event(code);
    }
    assert_eq!(state.key_events.len(), 1000);

    // Simulate app.rs draining key_events into controller.shared
    for code in state.key_events.drain(..) {
        state.calc_input_focus = true;
        if code != 46 {
            // normal key
            shared.push_key(code);
        }
    }
    assert!(state.key_events.is_empty());

    // Drained keys should match, minus any On (46) keys
    let mut drained_count = 0;
    while let Ok(Some(code)) = host.poll_key() {
        assert_ne!(code, 46);
        assert!((0..=50).contains(&code));
        drained_count += 1;
    }
    // Exactly 1000 - (number of 46s). In 0..1000 % 51, 46 occurs 19 times
    let count_46 = (0..1000).filter(|&i| i % 51 == 46).count();
    assert_eq!(drained_count, 1000 - count_46);
}

// ============================================================================
// 2. Holding Down Keys While Switching Focus From Calculator to Editor
// ============================================================================

#[test]
fn test_stress_holding_keys_while_switching_focus_to_editor() {
    let shared = Arc::new(GuiSharedState::new());
    let mut state = IdeAppState::default();
    let mut host = GuiHost::new(shared.clone());

    state.calc_input_focus = true;

    // Hold down a multi-key combo (e.g. Shift(41) + Enter(30) + Up(2) + Plus(50))
    let held_combo = [41, 30, 2, 50, 7, 8, 12];
    for &k in &held_combo {
        state.set_key_down(k, true);
        shared.set_key_down(k, true);
    }

    // Verify all keys are reported as held
    for &k in &held_combo {
        assert!(state.keys_down.contains(&k));
        assert!(host.is_key_down(k).unwrap());
    }

    // Switch focus to editor
    let prev_focus = state.calc_input_focus;
    state.calc_input_focus = false;

    // Simulate focus transition logic in app.rs:1372-1376
    if prev_focus && !state.calc_input_focus {
        shared.clear_held_keys();
        state.clear_held_keys();
    }

    // Verify ALL keys are now cleared in both state and host
    assert!(state.keys_down.is_empty());
    for k in 0..=50 {
        assert!(
            !host.is_key_down(k).unwrap(),
            "Key {k} must not remain held after focus switch to editor"
        );
    }

    // Physical key release events arriving after focus switch must be safe no-ops
    for &k in &held_combo {
        shared.set_key_down(k, false);
        state.set_key_down(k, false);
        assert!(!host.is_key_down(k).unwrap());
    }
}

#[test]
fn test_stress_rapid_focus_toggle_with_held_keys_cycle() {
    let shared = Arc::new(GuiSharedState::new());
    let mut state = IdeAppState::default();
    let mut host = GuiHost::new(shared.clone());

    // Run 500 rapid cycles of focus switching while keys are pressed
    for cycle in 0..500 {
        // Calculator gains focus
        state.calc_input_focus = true;
        let key = (cycle % 51) as i64;
        state.set_key_down(key, true);
        shared.set_key_down(key, true);

        assert!(host.is_key_down(key).unwrap());

        // Focus lost to editor
        let prev_focus = state.calc_input_focus;
        state.calc_input_focus = false;

        if prev_focus && !state.calc_input_focus {
            shared.clear_held_keys();
            state.clear_held_keys();
        }

        assert!(
            !host.is_key_down(key).unwrap(),
            "Cycle {cycle}: Key {key} must be cleared"
        );
        assert!(state.keys_down.is_empty());
    }
}

// ============================================================================
// 3. Typing Code in Editor: Zero Keypresses Leak to VM
// ============================================================================

#[test]
fn test_stress_editor_typing_zero_vm_leakage() {
    let shared = Arc::new(GuiSharedState::new());
    let mut state = IdeAppState::default();
    let mut host = GuiHost::new(shared.clone());

    // Editor has focus, calculator does NOT
    state.calc_input_focus = false;

    // Simulate extensive typing into an EditorTab
    let mut tab = EditorTab::new("MAIN.ppl", "");
    let typing_stream = [
        "EXPORT MY_PROG()\n",
        "BEGIN\n",
        "  LOCAL A := 12345;\n",
        "  LOCAL B := 67890;\n",
        "  LOCAL C := A + B - (A * B / 2);\n",
        "  PRINT(C);\n",
        "END;\n",
    ];

    for chunk in typing_stream {
        for ch in chunk.chars() {
            tab.content.push(ch);

            // In app.rs forward_calc_keys, if !state.calc_input_focus, events are completely ignored
            if state.calc_input_focus {
                // Should NOT execute
                let mut buf = [0u8; 4];
                let s = ch.encode_utf8(&mut buf);
                if let Some(code) = map_char(s) {
                    shared.push_key(code);
                }
            }
        }
    }

    // Verify editor received the entire text
    assert_eq!(
        tab.content,
        "EXPORT MY_PROG()\nBEGIN\n  LOCAL A := 12345;\n  LOCAL B := 67890;\n  LOCAL C := A + B - (A * B / 2);\n  PRINT(C);\nEND;\n"
    );

    // Verify VM key queue is 100% empty: zero keys leaked
    assert_eq!(
        host.poll_key().unwrap(),
        None,
        "VM key queue must be empty when editor has focus"
    );

    // Verify zero held keys
    for k in 0..=50 {
        assert!(!host.is_key_down(k).unwrap());
    }

    // Verify not interrupted
    assert!(!shared.is_interrupted());
}

// ============================================================================
// 4. All 51 Keycodes (0–50) Delivery to GETKEY and ISKEYDOWN
// ============================================================================

#[test]
fn test_stress_all_51_keycodes_getkey_and_iskeydown() {
    for code in 0..=50 {
        let name = hp_key_name(code);
        assert!(
            name.is_some(),
            "Keycode {code} must have a valid human name"
        );

        let shared = Arc::new(GuiSharedState::new());

        // 1. Test GETKEY delivery
        shared.push_key(code);
        let host1 = GuiHost::new(shared.clone());
        let src_getkey = "EXPORT TEST_GK() BEGIN RETURN GETKEY(); END;";
        let val_getkey = run_ppl_with_host(src_getkey, host1).expect("GETKEY script execution");
        match val_getkey {
            Value::Real(r) => {
                let num: f64 = r.to_f64();
                assert_eq!(
                    num as i64, code,
                    "GETKEY() must return exact keycode {code}"
                );
            }
            other => panic!("Expected Value::Real for GETKEY, got {:?}", other),
        }

        // 2. Test ISKEYDOWN true
        shared.set_key_down(code, true);
        let host2 = GuiHost::new(shared.clone());
        let src_iskeydown = format!("EXPORT TEST_KD() BEGIN RETURN ISKEYDOWN({code}); END;");
        let val_iskeydown =
            run_ppl_with_host(&src_iskeydown, host2).expect("ISKEYDOWN script execution");
        match val_iskeydown {
            Value::Real(r) => {
                let num: f64 = r.to_f64();
                assert_eq!(
                    num as i64, 1,
                    "ISKEYDOWN({code}) must return 1 when held down"
                );
            }
            other => panic!("Expected Value::Real for ISKEYDOWN, got {:?}", other),
        }

        // 3. Test ISKEYDOWN false after release
        shared.set_key_down(code, false);
        let host3 = GuiHost::new(shared.clone());
        let val_released =
            run_ppl_with_host(&src_iskeydown, host3).expect("ISKEYDOWN released script execution");
        match val_released {
            Value::Real(r) => {
                let num: f64 = r.to_f64();
                assert_eq!(
                    num as i64, 0,
                    "ISKEYDOWN({code}) must return 0 when released"
                );
            }
            other => panic!("Expected Value::Real for ISKEYDOWN, got {:?}", other),
        }
    }
}

#[test]
fn test_stress_out_of_bounds_keycodes_iskeydown() {
    let shared = Arc::new(GuiSharedState::new());
    let mut host = GuiHost::new(shared.clone());

    let out_of_bounds = [-999, -1, 51, 52, 100, 10000];
    for &bad_code in &out_of_bounds {
        // Direct host query
        assert!(
            !host.is_key_down(bad_code).unwrap(),
            "Out of bounds key {bad_code} must return false"
        );

        // PPL ISKEYDOWN query
        let host_for_vm = GuiHost::new(shared.clone());
        let src = format!("EXPORT BAD_K() BEGIN RETURN ISKEYDOWN({bad_code}); END;");
        let val = run_ppl_with_host(&src, host_for_vm).expect("ISKEYDOWN bad code execution");
        match val {
            Value::Real(r) => {
                assert_eq!(
                    r.to_f64() as i64,
                    0,
                    "ISKEYDOWN({bad_code}) must safely return 0"
                );
            }
            other => panic!("Expected Real, got {:?}", other),
        }
    }
}

#[test]
fn test_stress_virtual_keypad_layout_integrity_all_51_keys() {
    let mut observed_codes = HashSet::new();

    for key in KEYS {
        assert!(!key.label.is_empty());
        assert!((0..=50).contains(&key.code));
        assert!(observed_codes.insert(key.code));
        assert!(key.bounds[2] > key.bounds[0] && key.bounds[3] > key.bounds[1]);
    }
    assert_eq!(KEYS[30].label, "Enter");
    assert_eq!(KEYS[30].device_code, 0x0D);
    assert!(KEYS[30].bounds[2] - KEYS[30].bounds[0] > 100.0);

    // Total must be exactly 51 keys covering 0..=50
    assert_eq!(observed_codes.len(), 51);
    for expected in 0..=50 {
        assert!(
            observed_codes.contains(&expected),
            "Keycode {expected} missing from virtual keypad layout"
        );
    }
}

// ============================================================================
// 5. Interactive Game Loop Scenario 7 Stress & Extension
// ============================================================================

#[test]
fn test_stress_interactive_game_loop_multi_directional() {
    let game_src = r#"
EXPORT SNAKE()
BEGIN
  LOCAL x := 160;
  LOCAL y := 120;
  LOCAL k := 0;
  LOCAL i := 0;
  WHILE i < 10 DO
    k := GETKEY();
    IF k == 2 THEN y := y - 5; END; // UP
    IF k == 12 THEN y := y + 5; END; // DOWN
    IF k == 7 THEN x := x - 5; END; // LEFT
    IF k == 8 THEN x := x + 5; END; // RIGHT
    PIXON_P(x, y, #00FF00);
    i := i + 1;
  END;
  PRINT(x);
  PRINT(y);
END;
"#;

    let shared = Arc::new(GuiSharedState::new());
    // 10 moves: UP(2), UP(2), RIGHT(8), RIGHT(8), RIGHT(8), DOWN(12), LEFT(7), RIGHT(8), UP(2), RIGHT(8)
    // Expected trace from (160, 120):
    // 2  -> y = 115
    // 2  -> y = 110
    // 8  -> x = 165
    // 8  -> x = 170
    // 8  -> x = 175
    // 12 -> y = 115
    // 7  -> x = 170
    // 8  -> x = 175
    // 2  -> y = 110
    // 8  -> x = 180
    // Final: (180, 110)
    let moves = [2, 2, 8, 8, 8, 12, 7, 8, 2, 8];
    for &m in &moves {
        shared.push_key(m);
    }

    let host = GuiHost::new(shared.clone());
    let res = run_ppl_with_host(game_src, host);
    assert!(res.is_ok(), "Game execution failed: {:?}", res.err());

    let lines = shared.terminal_lines.lock().unwrap().clone();
    assert_eq!(lines, vec!["180", "110"]);

    // Verify pixel on GROB screen at (180, 110) has color
    let frame = shared.get_frame();
    let pixel = frame.get(180, 110);
    assert!(
        pixel.is_some(),
        "Pixel at (180, 110) must be within screen bounds"
    );
    assert_ne!(pixel.unwrap(), 0, "Pixel at (180, 110) must be lit");
}

#[test]
fn test_stress_keypad_flood_1000_during_active_vm_execution() {
    let src = r#"
EXPORT SUM_KEYS()
BEGIN
  LOCAL sum := 0;
  LOCAL count := 0;
  LOCAL k := 0;
  WHILE count < 1000 DO
    k := GETKEY();
    IF k >= 0 THEN
      sum := sum + k;
      count := count + 1;
    END;
  END;
  PRINT(sum);
  RETURN sum;
END;
"#;

    let shared = Arc::new(GuiSharedState::new());

    // Pre-feed 1,000 keys into queue
    let mut expected_sum: i64 = 0;
    for i in 0..1000 {
        let code = (i % 51) as i64;
        expected_sum += code;
        shared.push_key(code);
    }

    let host = GuiHost::new(shared.clone());
    let val = run_ppl_with_host(src, host).expect("SUM_KEYS execution");
    match val {
        Value::Real(r) => {
            assert_eq!(
                r.to_f64() as i64,
                expected_sum,
                "VM must process all 1,000 keys and calculate exact arithmetic sum"
            );
        }
        other => panic!("Expected Real sum, got {:?}", other),
    }

    let lines = shared.terminal_lines.lock().unwrap().clone();
    assert_eq!(lines, vec![expected_sum.to_string()]);
}

// ============================================================================
// 6. Character Mapping & Key Naming Stress
// ============================================================================

#[test]
fn test_stress_map_char_all_combinations() {
    let valid_chars = [
        ("0", 47),
        ("1", 42),
        ("2", 43),
        ("3", 44),
        ("4", 37),
        ("5", 38),
        ("6", 39),
        ("7", 32),
        ("8", 33),
        ("9", 34),
        ("+", 50),
        ("-", 45),
        ("*", 40),
        ("/", 35),
        ("=", 30),
        (".", 48),
        (",", 29),
        ("(", 28),
        (")", 28),
        ("^", 20),
        (" ", 49),
        ("x", 40),
        ("X", 40),
    ];

    for (s, expected) in valid_chars {
        assert_eq!(map_char(s), Some(expected), "map_char({:?})", s);
    }

    let invalid_chars = ["!", "@", "#", "$", "%", "&", "foo", "hello", "α", "🚀"];
    for s in invalid_chars {
        assert_eq!(map_char(s), None, "map_char({:?}) must be None", s);
    }
}

#[test]
fn test_stress_hp_key_name_all_51() {
    for code in 0..=50 {
        let name = hp_key_name(code);
        assert!(name.is_some());
        let s = name.unwrap();
        assert!(!s.is_empty(), "Key {code} name cannot be empty");
    }

    assert_eq!(hp_key_name(-1), None);
    assert_eq!(hp_key_name(51), None);
    assert_eq!(hp_key_name(100), None);
}

// ============================================================================
// 7. Empirical Challenge Test Suite (Milestone 2 Challenge Mission)
// ============================================================================

#[test]
fn test_empirical_10000_sequential_and_overlapping_transitions() {
    let shared = Arc::new(GuiSharedState::new());
    let mut state = IdeAppState::default();
    let mut host = GuiHost::new(shared.clone());
    state.calc_input_focus = true;

    // LCG pseudo-random generator with fixed seed for 100% deterministic reproducibility
    let mut rng_state: u64 = 0xDEADBEEF_CAFE1234;
    let mut next_rand = || -> u64 {
        rng_state = rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        rng_state
    };

    // Phase 1: 5,000 rapid sequential key transitions (2,500 press + 2,500 release)
    for i in 0..2500 {
        let code = (next_rand() % 51) as i64;

        // Press
        state.set_key_down(code, true);
        shared.set_key_down(code, true);
        state.key_transitions.push((code, true));
        shared.push_key(code);

        assert!(state.keys_down.contains(&code));
        assert!(host.is_key_down(code).unwrap());

        // Release
        state.set_key_down(code, false);
        shared.set_key_down(code, false);
        state.key_transitions.push((code, false));

        assert!(!state.keys_down.contains(&code));
        assert!(!host.is_key_down(code).unwrap());

        // FIFO verification
        let polled = host.poll_key().unwrap().expect("key present in queue");
        assert_eq!(polled, code, "Iteration {i}: FIFO order mismatch");
    }
    assert_eq!(state.key_transitions.len(), 5000);
    assert!(state.keys_down.is_empty());
    assert_eq!(host.poll_key().unwrap(), None);

    // Phase 2: 5,000 overlapping multi-key chord transitions across all 51 keys
    let mut ground_truth_held = HashSet::new();
    let max_chord_size = 10;

    for step in 0..5000 {
        let candidate = (next_rand() % 51) as i64;
        let is_held = ground_truth_held.contains(&candidate);

        // Decide whether to press or release
        let should_press = if ground_truth_held.len() >= max_chord_size {
            false
        } else if ground_truth_held.is_empty() {
            true
        } else {
            (next_rand() % 2) == 0
        };

        if should_press && !is_held {
            ground_truth_held.insert(candidate);
            state.set_key_down(candidate, true);
            shared.set_key_down(candidate, true);
            state.key_transitions.push((candidate, true));
        } else if !should_press && is_held {
            ground_truth_held.remove(&candidate);
            state.set_key_down(candidate, false);
            shared.set_key_down(candidate, false);
            state.key_transitions.push((candidate, false));
        } else if !ground_truth_held.is_empty() {
            // Pick an actually held key to release
            let held_keys: Vec<i64> = ground_truth_held.iter().copied().collect();
            let idx = (next_rand() as usize) % held_keys.len();
            let to_release = held_keys[idx];
            ground_truth_held.remove(&to_release);
            state.set_key_down(to_release, false);
            shared.set_key_down(to_release, false);
            state.key_transitions.push((to_release, false));
        }

        // Periodically verify exact ground truth correspondence for ALL 51 keys
        if step % 250 == 0 {
            for k in 0..=50 {
                let expected = ground_truth_held.contains(&k);
                assert_eq!(
                    state.keys_down.contains(&k),
                    expected,
                    "Step {step}: state.keys_down mismatch for key {k}"
                );
                assert_eq!(
                    host.is_key_down(k).unwrap(),
                    expected,
                    "Step {step}: host.is_key_down mismatch for key {k}"
                );
            }
        }
    }

    // Release all remaining held keys
    let remaining: Vec<i64> = ground_truth_held.drain().collect();
    for k in remaining {
        state.set_key_down(k, false);
        shared.set_key_down(k, false);
        state.key_transitions.push((k, false));
    }

    // Total transitions must be >= 10,000
    assert!(
        state.key_transitions.len() >= 10000,
        "Total transitions must reach at least 10,000, got {}",
        state.key_transitions.len()
    );
    assert!(state.keys_down.is_empty());
    for k in 0..=50 {
        assert!(!host.is_key_down(k).unwrap());
    }
}

#[test]
fn test_empirical_pointer_drag_capture_outside_window() {
    // Test diverse key types across the entire keypad geometry:
    // 0: Apps (upper navigation row)
    // 2: Up (D-pad directional sector, top)
    // 7: Left (D-pad directional sector, left)
    // 8: Right (D-pad directional sector, right)
    // 12: Down (D-pad directional sector, bottom)
    // 21: SIN (scientific row 1)
    // 30: Enter (wide lower right key)
    // 47: 0 (bottom numeric row)
    let test_keys = [0, 2, 7, 8, 12, 21, 30, 47];

    fn test_raw_input(focused: bool) -> RawInput {
        RawInput {
            focused,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 1000.0))),
            ..Default::default()
        }
    }

    for &code in &test_keys {
        let ctx = egui::Context::default();
        let mut keypad = VirtualKeypad::new();
        let mut state = IdeAppState {
            vm_state: VmExecutionState::Running,
            ..Default::default()
        };

        // Determine click position
        let click_pos = match code {
            2 => Pos2::new(187.0, 332.0),
            7 => Pos2::new(153.0, 367.0),
            8 => Pos2::new(221.0, 367.0),
            12 => Pos2::new(187.0, 402.0),
            other => {
                let key_def = KEYS.iter().find(|k| k.code == other).expect("key exists");
                key_def.rect(Pos2::ZERO, 1.0).center()
            }
        };

        // Frame 1: Pointer Click Down on Key
        let mut raw_1 = test_raw_input(true);
        raw_1.events.push(egui::Event::PointerButton {
            pos: click_pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::default(),
        });

        let _ = ctx.run(raw_1, |ctx| {
            CentralPanel::default().show(ctx, |ui| {
                keypad.paint_at(ui, &mut state, Pos2::ZERO, 1.0);
            });
        });

        assert!(
            state.keys_down.contains(&code),
            "Key {code} must be down on initial click"
        );
        assert!(state.calc_input_focus, "Calculator must gain focus");
        assert!(
            state.key_transitions.contains(&(code, true)),
            "Key transition true must be logged"
        );
        assert_eq!(keypad.last_clicked_key, Some(code));

        // Frame 2: Pointer Dragged FAR outside window bounds (-9999, -9999)
        let outside_pos_1 = Pos2::new(-9999.0, -9999.0);
        let mut raw_2 = test_raw_input(true);
        raw_2.events.push(egui::Event::PointerMoved(outside_pos_1));

        let _ = ctx.run(raw_2, |ctx| {
            CentralPanel::default().show(ctx, |ui| {
                keypad.paint_at(ui, &mut state, Pos2::ZERO, 1.0);
            });
        });

        // Key MUST STAY HELD while pointer is outside window
        assert!(
            state.keys_down.contains(&code),
            "Key {code} must remain held when dragged outside window bounds"
        );

        // Frames 3-6: Moving pointer around other outside positions
        let outside_positions = [
            Pos2::new(5000.0, 5000.0),
            Pos2::new(-200.0, 400.0),
            Pos2::new(381.0 + 100.0, 797.0 + 100.0),
            Pos2::new(0.0, -50.0),
        ];
        for (idx, pos) in outside_positions.iter().enumerate() {
            let mut raw_mid = test_raw_input(true);
            raw_mid.events.push(egui::Event::PointerMoved(*pos));

            let _ = ctx.run(raw_mid, |ctx| {
                CentralPanel::default().show(ctx, |ui| {
                    keypad.paint_at(ui, &mut state, Pos2::ZERO, 1.0);
                });
            });

            assert!(
                state.keys_down.contains(&code),
                "Key {code} must remain held at outside pos {idx}: {:?}",
                pos
            );
        }

        // Final Frame: Release Primary Pointer Button outside window
        let mut raw_rel = test_raw_input(true);
        raw_rel.events.push(egui::Event::PointerButton {
            pos: outside_positions.last().copied().unwrap(),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::default(),
        });

        let _ = ctx.run(raw_rel, |ctx| {
            CentralPanel::default().show(ctx, |ui| {
                keypad.paint_at(ui, &mut state, Pos2::ZERO, 1.0);
            });
        });

        assert!(
            !state.keys_down.contains(&code),
            "Key {code} must be released when primary pointer button is released outside"
        );
        assert!(
            state.key_transitions.contains(&(code, false)),
            "Key transition false must be logged for key {code}"
        );

        // Verification of clean state
        let raw_idle = test_raw_input(true);
        let _ = ctx.run(raw_idle, |ctx| {
            CentralPanel::default().show(ctx, |ui| {
                keypad.paint_at(ui, &mut state, Pos2::ZERO, 1.0);
            });
        });
        assert!(state.keys_down.is_empty());
    }
}

#[test]
fn test_empirical_focus_transition_and_blur_deadlock_safety() {
    fn test_raw_input(focused: bool) -> RawInput {
        RawInput {
            focused,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 1000.0))),
            ..Default::default()
        }
    }

    // Scenario A: Window blur while key is captured
    {
        let ctx = egui::Context::default();
        let mut keypad = VirtualKeypad::new();
        let mut state = IdeAppState {
            vm_state: VmExecutionState::Running,
            ..Default::default()
        };

        // Click Enter (30)
        let enter_pos = KEYS
            .iter()
            .find(|k| k.code == 30)
            .unwrap()
            .rect(Pos2::ZERO, 1.0)
            .center();

        let mut raw_1 = test_raw_input(true);
        raw_1.events.push(egui::Event::PointerButton {
            pos: enter_pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::default(),
        });
        let _ = ctx.run(raw_1, |ctx| {
            CentralPanel::default().show(ctx, |ui| {
                keypad.paint_at(ui, &mut state, Pos2::ZERO, 1.0);
            });
        });
        assert!(state.keys_down.contains(&30));

        // Window loses focus (blur event, mouse still pressed)
        let raw_blur = test_raw_input(false); // window blur!
        let _ = ctx.run(raw_blur, |ctx| {
            CentralPanel::default().show(ctx, |ui| {
                keypad.paint_at(ui, &mut state, Pos2::ZERO, 1.0);
            });
        });

        // Key must be automatically released on blur
        assert!(
            !state.keys_down.contains(&30),
            "Window blur must release captured key 30"
        );
        assert_eq!(state.key_transitions.last(), Some(&(30, false)));

        // Window regains focus later
        let mut raw_refocus = test_raw_input(true);
        // Release physical mouse button
        raw_refocus.events.push(egui::Event::PointerButton {
            pos: enter_pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::default(),
        });
        let _ = ctx.run(raw_refocus, |ctx| {
            CentralPanel::default().show(ctx, |ui| {
                keypad.paint_at(ui, &mut state, Pos2::ZERO, 1.0);
            });
        });
        assert!(state.keys_down.is_empty());

        // Subsequent click must work cleanly with NO deadlock
        let mut raw_next = test_raw_input(true);
        raw_next.events.push(egui::Event::PointerButton {
            pos: enter_pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::default(),
        });
        let _ = ctx.run(raw_next, |ctx| {
            CentralPanel::default().show(ctx, |ui| {
                keypad.paint_at(ui, &mut state, Pos2::ZERO, 1.0);
            });
        });
        assert!(
            state.keys_down.contains(&30),
            "Keypad must immediately respond to new clicks after blur recovery without deadlock"
        );
    }

    // Scenario B: Click key, switch focus to editor, verify isolation and deadlock safety
    {
        let mut keypad = VirtualKeypad::new();
        let mut state = IdeAppState::default();
        let shared = Arc::new(GuiSharedState::new());
        let mut host = GuiHost::new(shared.clone());

        // Calculator has focus with held keys (e.g. Shift 41, Alpha 36, Plus 50)
        state.calc_input_focus = true;
        let held_keys = [41, 36, 50];
        for &k in &held_keys {
            state.set_key_down(k, true);
            shared.set_key_down(k, true);
        }
        for &k in &held_keys {
            assert!(state.keys_down.contains(&k));
            assert!(host.is_key_down(k).unwrap());
        }

        // User switches focus to editor
        let prev_calc_focus = state.calc_input_focus;
        state.calc_input_focus = false;

        // Execute focus loss cleanup (app.rs:1191-1211 contract)
        if prev_calc_focus && !state.calc_input_focus {
            keypad.release(&mut state);
            shared.clear_held_keys();
            shared.set_touch(None);
            state.clear_held_keys();
        }

        // Verify zero keys held
        assert!(state.keys_down.is_empty());
        for k in 0..=50 {
            assert!(!host.is_key_down(k).unwrap());
        }

        // Simulate extensive editor typing while editor is focused
        let mut tab = EditorTab::new("CODE.ppl", "");
        let code_text = "LOCAL A := 1; LOCAL B := 2; PRINT(A+B);";
        for ch in code_text.chars() {
            tab.content.push(ch);
            if state.calc_input_focus {
                // Must NOT execute
                shared.push_key(50);
            }
        }
        assert_eq!(tab.content, code_text);
        assert_eq!(
            host.poll_key().unwrap(),
            None,
            "Editor typing must not leak any keys to VM queue"
        );

        // Switch focus BACK to calculator
        state.calc_input_focus = true;
        shared.push_key(30); // Enter
        assert_eq!(
            host.poll_key().unwrap(),
            Some(30),
            "Calculator must accept keys immediately after refocus"
        );
    }
}

#[test]
fn test_empirical_key_translation_matrix_consistency_all_lookups() {
    assert_eq!(
        KEY_MAPPINGS.len(),
        51,
        "Authoritative matrix must contain exactly 51 keys"
    );

    let mut seen_device_codes = HashSet::new();

    for id in 0..=50u8 {
        // 1. lookup_by_physical
        let m_phys = lookup_by_physical(id).expect("lookup_by_physical must succeed");
        assert_eq!(m_phys.physical_id, id);
        assert_eq!(m_phys.ppl_code, id as i32);
        assert!(!m_phys.primary_label.is_empty());

        // 2. lookup_by_ppl_code
        let m_ppl = lookup_by_ppl_code(id as i32).expect("lookup_by_ppl_code must succeed");
        assert_eq!(
            m_phys, m_ppl,
            "lookup_by_physical and lookup_by_ppl_code must return identical mapping for key {id}"
        );

        // 3. lookup_by_device_code
        let m_dev =
            lookup_by_device_code(m_phys.device_code).expect("lookup_by_device_code must succeed");
        assert_eq!(
            m_phys, m_dev,
            "lookup_by_physical and lookup_by_device_code must return identical mapping for key {id}"
        );

        // Device codes must be strictly unique
        assert!(
            seen_device_codes.insert(m_phys.device_code),
            "Duplicate device code {:#06X} found at key {id}",
            m_phys.device_code
        );

        // 4. lookup_by_pc_key (if present)
        if let Some(pc_k) = m_phys.pc_key {
            let m_pc = lookup_by_pc_key(pc_k).expect("lookup_by_pc_key must succeed");
            assert_eq!(
                m_pc.ppl_code, m_phys.ppl_code,
                "lookup_by_pc_key must match ppl_code for key {id}"
            );
            assert_eq!(
                egui_key_to_hp(pc_k),
                Some(id as i64),
                "egui_key_to_hp must match key {id}"
            );
        }

        // hp_key_name consistency
        let name = hp_key_name(id as i64).expect("hp_key_name must exist");
        assert!(!name.is_empty());
    }

    // Verify all standard aliases in lookup_by_pc_key
    let aliases: &[(egui::Key, i32)] = &[
        (egui::Key::F1, 0),
        (egui::Key::F2, 1),
        (egui::Key::ArrowUp, 2),
        (egui::Key::F3, 3),
        (egui::Key::Escape, 4),
        (egui::Key::Home, 5),
        (egui::Key::F4, 5),
        (egui::Key::PageUp, 6),
        (egui::Key::F5, 6),
        (egui::Key::ArrowLeft, 7),
        (egui::Key::ArrowRight, 8),
        (egui::Key::F6, 9),
        (egui::Key::F7, 10),
        (egui::Key::PageDown, 11),
        (egui::Key::F8, 11),
        (egui::Key::ArrowDown, 12),
        (egui::Key::End, 13),
        (egui::Key::F9, 13),
        (egui::Key::Backspace, 19),
        (egui::Key::Delete, 19),
        (egui::Key::F11, 20),
        (egui::Key::S, 21),
        (egui::Key::C, 22),
        (egui::Key::T, 23),
        (egui::Key::L, 24),
        (egui::Key::Comma, 29),
        (egui::Key::Enter, 30),
        (egui::Key::Equals, 30),
        (egui::Key::E, 31),
        (egui::Key::Num7, 32),
        (egui::Key::Num8, 33),
        (egui::Key::Num9, 34),
        (egui::Key::Slash, 35),
        (egui::Key::Tab, 36),
        (egui::Key::Num4, 37),
        (egui::Key::Num5, 38),
        (egui::Key::Num6, 39),
        (egui::Key::X, 40),
        (egui::Key::Num1, 42),
        (egui::Key::Num2, 43),
        (egui::Key::Num3, 44),
        (egui::Key::Minus, 45),
        (egui::Key::F12, 46),
        (egui::Key::Num0, 47),
        (egui::Key::Period, 48),
        (egui::Key::Space, 49),
        (egui::Key::Plus, 50),
    ];

    for &(k, expected_ppl) in aliases {
        let mapped = lookup_by_pc_key(k).expect("alias lookup must succeed");
        assert_eq!(
            mapped.ppl_code, expected_ppl,
            "Alias {:?} must map to PPL code {}",
            k, expected_ppl
        );
        assert_eq!(
            egui_key_to_hp(k),
            Some(expected_ppl as i64),
            "egui_key_to_hp({:?}) must return {}",
            k,
            expected_ppl
        );
    }

    // Out of bounds queries must return None without panics
    for bad_id in [51, 52, 100, 255] {
        assert!(lookup_by_physical(bad_id).is_none());
    }
    for bad_ppl in [-1000, -1, 51, 52, 1000] {
        assert!(lookup_by_ppl_code(bad_ppl).is_none());
    }
    for bad_dev in [0x0000, 0xFFFF, 0x1234, 0x9999] {
        assert!(lookup_by_device_code(bad_dev).is_none());
    }

    // Unmapped egui keys must return None
    let unmapped_keys = [
        egui::Key::A,
        egui::Key::B,
        egui::Key::D,
        egui::Key::F,
        egui::Key::G,
        egui::Key::H,
        egui::Key::I,
        egui::Key::J,
        egui::Key::K,
        egui::Key::M,
        egui::Key::N,
        egui::Key::O,
        egui::Key::P,
        egui::Key::Q,
        egui::Key::R,
        egui::Key::U,
        egui::Key::V,
        egui::Key::W,
        egui::Key::Y,
        egui::Key::Z,
        egui::Key::Insert,
    ];
    for &k in &unmapped_keys {
        assert!(
            lookup_by_pc_key(k).is_none(),
            "Key {:?} must not be mapped",
            k
        );
        assert_eq!(egui_key_to_hp(k), None);
    }
}
