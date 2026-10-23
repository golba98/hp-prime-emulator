//! Milestone 3 Adversarial Stress & Verification Test Suite
//! Keypad Interaction, Focus Separation & Keyboard Routing Stress Testing
//!
//! Authored by Challenger 2 for Milestone 3: Emulated HP Prime G1 Display & Virtual Keypad (R4).

use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use ppl_compiler::CompileOptions;
use ppl_gui::host::{GuiHost, GuiSharedState};
use ppl_gui::ide::IdeAppState;
use ppl_gui::ide::editor::EditorTab;
use ppl_gui::ide::keypad::{
    KEY_ENTER_DEF, ROW_1, ROW_2, ROW_3, ROW_4, ROW_5, ROW_6, ROW_7, ROW_8, ROW_9, ROW_10,
};
use ppl_gui::keymap::{hp_key_name, map_char};
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

    let clusters = [
        ("Row 1", &ROW_1[..]),
        ("Row 2", &ROW_2[..]),
        ("Row 3", &ROW_3[..]),
        ("Row 4", &ROW_4[..]),
        ("Row 5", &ROW_5[..]),
        ("Row 6", &ROW_6[..]),
        ("Row 7", &ROW_7[..]),
        ("Row 8", &ROW_8[..]),
        ("Row 9", &ROW_9[..]),
        ("Row 10", &ROW_10[..]),
    ];

    for (name, row) in clusters {
        assert_eq!(row.len(), 5, "{name} must have exactly 5 keys");
        for key in row {
            assert!(!key.label.is_empty(), "Key label must not be empty");
            assert!(
                (0..=50).contains(&key.code),
                "Keycode {} out of range 0..=50",
                key.code
            );
            let inserted = observed_codes.insert(key.code);
            assert!(
                inserted,
                "Duplicate keycode {} detected in {}",
                key.code, name
            );
        }
    }

    // Check Enter Key
    assert_eq!(KEY_ENTER_DEF.code, 30);
    assert_eq!(KEY_ENTER_DEF.label, "Enter");
    let enter_inserted = observed_codes.insert(KEY_ENTER_DEF.code);
    assert!(enter_inserted, "Enter keycode 30 duplicate");

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
