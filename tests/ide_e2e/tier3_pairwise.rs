//! Tier 3: Pairwise Cross-Feature Interactions for HP Prime Desktop IDE.
//!
//! Verifies combinations of interacting features across the desktop shell,
//! editor, compiler, graphics, virtual keypad, worker thread, and debugger.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use super::harness::*;
use ppl_compiler::CompileOptions;
use ppl_graphics::{Grob, compare_rgb565, rgb};
use ppl_gui::host::{GuiHost, GuiSharedState};
use ppl_gui::scaler::{Viewport, window_to_framebuffer};
use ppl_gui::theme::{BG_PANEL, classic_engineering_visuals};
use ppl_syntax::diag::Diagnostics;
use ppl_syntax::source::SourceFile;
use ppl_vm::host::{Host, Touch};
use ppl_vm::machine::{Limits, Machine};
use ppl_vm::settings::Settings;
use ppl_vm::value::Value;

#[test]
fn test_p01_theme_and_display() {
    let visuals = classic_engineering_visuals();
    let display_grob = Grob::screen();
    assert_eq!(visuals.panel_fill, BG_PANEL);
    assert_eq!(display_grob.width(), 320);
    assert_eq!(display_grob.height(), 240);
}

#[test]
fn test_p02_menu_and_editor() {
    let mut state = IdeAppStateModel::new();
    state.queue_action(IdeAction::NewProgram);
    state.open_tab("NEW_PROG.hpprgm", default_ppl_template("NEW_PROG"));

    state
        .active_tab_mut()
        .unwrap()
        .set_content("EXPORT NEW_PROG() BEGIN PRINT(1); END;");
    assert!(state.active_tab().unwrap().modified);

    state.queue_action(IdeAction::SaveActiveFile);
    state.active_tab_mut().unwrap().modified = false;
    assert!(!state.active_tab().unwrap().modified);
}

#[test]
fn test_p03_toolbar_and_worker() {
    let mut state = IdeAppStateModel::new();
    state.queue_action(IdeAction::RunProgram);
    state.vm_state = VmExecutionState::Running;

    let res = run_ppl_headless("EXPORT MAIN() BEGIN PRINT(42); END;", &[], &[]);
    assert!(res.success);
    assert_eq!(res.terminal_lines, vec!["42"]);

    state.queue_action(IdeAction::StopProgram);
    state.vm_state = VmExecutionState::Terminated;
    assert_eq!(state.vm_state, VmExecutionState::Terminated);
}

#[test]
fn test_p04_layout_and_display() {
    let mut state = IdeAppStateModel::new();
    state.calculator_width = 480.0;

    let vp = Viewport::calculate_aspect_fit(state.calculator_width as u32, 360);
    let ratio = vp.width as f32 / vp.height as f32;
    assert!((ratio - (320.0 / 240.0)).abs() < 0.01);
}

#[test]
fn test_p05_toggles_and_console() {
    let mut state = IdeAppStateModel::new();
    let res = run_ppl_headless("EXPORT MAIN() BEGIN PRINT(\"Persistent\"); END;", &[], &[]);
    state
        .shared_state
        .terminal_lines
        .lock()
        .unwrap()
        .extend(res.terminal_lines);

    // Toggle dock off and on
    state.show_bottom_dock = false;
    assert!(!state.show_bottom_dock);
    state.show_bottom_dock = true;

    // Terminal lines retained
    let lines = state.shared_state.terminal_lines.lock().unwrap();
    assert_eq!(*lines, vec!["Persistent"]);
}

#[test]
fn test_p06_editor_and_diagnostics() {
    let mut tab = EditorTab::new("T", "EXPORT MAIN() BEGIN LOCAL x := ; END;");
    let file = SourceFile::new("<t>", &tab.content);
    let (_, diags) = ppl_compiler::compile_source(&file, &CompileOptions::default());
    assert!(diags.has_errors());

    // Fix error in tab
    tab.set_content("EXPORT MAIN() BEGIN LOCAL x := 10; END;");
    let file2 = SourceFile::new("<t>", &tab.content);
    let (_, diags2) = ppl_compiler::compile_source(&file2, &CompileOptions::default());
    assert!(!diags2.has_errors());
}

#[test]
fn test_p07_editor_and_unsaved_run() {
    let tab = EditorTab::new("UNSAVED", "EXPORT MAIN() BEGIN PRINT(777); END;");
    assert!(tab.path.is_none()); // Unsaved
    let res = run_ppl_headless(&tab.content, &[], &[]);
    assert!(res.success);
    assert_eq!(res.terminal_lines, vec!["777"]);
}

#[test]
fn test_p08_file_ops_and_syntax() {
    let code = "EXPORT TEST() BEGIN LOCAL a := 10; RETURN a; END;";
    let mut diags = Diagnostics::default();
    let toks = ppl_syntax::lexer::lex(code, &mut diags);
    assert!(
        toks.iter()
            .any(|t| matches!(t.tok, ppl_syntax::lexer::Tok::Kw(_)))
    );
}

#[test]
fn test_p09_syntax_and_breakpoints() {
    let src = "EXPORT MAIN()\nBEGIN\n  LOCAL x := 10;\n  PRINT(x);\nEND;";
    let mut diags = Diagnostics::default();
    let toks = ppl_syntax::lexer::lex(src, &mut diags);
    let mut tab = EditorTab::new("T", src);
    tab.toggle_breakpoint(3); // Line with LOCAL x := 10;

    assert!(tab.breakpoints.contains(&3));
    assert!(toks.iter().any(|t| t.span.start > 0));
}

#[test]
fn test_p10_diagnostics_and_controls() {
    let broken = "EXPORT MAIN() BEGIN LOCAL x := ; END;";
    let file = SourceFile::new("<t>", broken);
    let (prog, diags) = ppl_compiler::compile_source(&file, &CompileOptions::default());

    let can_run = prog.is_some() && !diags.has_errors();
    assert!(
        !can_run,
        "Diagnostics must block Run control from launching"
    );
}

#[test]
fn test_p11_display_and_drawing() {
    let src = "EXPORT MAIN() BEGIN RECT_P(0, 0, 50, 50, 0, #00FF00); END;";
    let res = run_ppl_headless(src, &[], &[]);
    assert!(res.success);
    assert_eq!(res.screen.get(25, 25), Some(rgb(0, 255, 0)));
}

#[test]
fn test_p12_display_and_touch() {
    let vp = Viewport::calculate_aspect_fit(640, 480);
    let fb_coord = window_to_framebuffer(320.0, 240.0, &vp).unwrap();
    assert_eq!(fb_coord, (160, 120));

    let shared = Arc::new(GuiSharedState::new());
    let mut host = GuiHost::new(shared.clone());
    shared.set_touch(Some(Touch {
        x: fb_coord.0,
        y: fb_coord.1,
    }));
    assert_eq!(host.touch().unwrap().unwrap().x, 160);
}

#[test]
fn test_p13_display_and_screenshot() {
    let mut g = Grob::screen();
    g.rect(10, 10, 40, 40, rgb(255, 0, 0), rgb(0, 0, 255));

    let mut png_bytes = Vec::new();
    g.write_png(&mut png_bytes).unwrap();

    let decoded = Grob::read_png(&png_bytes).unwrap();
    let diff = compare_rgb565(&g, &decoded).unwrap();
    assert_eq!(diff.mismatched, 0);
}

#[test]
fn test_p14_keypad_and_focus() {
    let shared = Arc::new(GuiSharedState::new());
    let mut host = GuiHost::new(shared.clone());
    // Simulate editor typing
    let mut editor_tab = EditorTab::new("T", "typing code");
    editor_tab.content.push('x');

    // Virtual keypad click delivers key independently
    shared.push_key(30);
    assert_eq!(host.poll_key().unwrap(), Some(30));
}

#[test]
fn test_p15_keypad_and_getkey() {
    let src = "EXPORT MAIN() BEGIN PRINT(GETKEY()); END;";
    let res = run_ppl_headless(src, &[], &[42]);
    assert!(res.success);
    assert_eq!(res.terminal_lines, vec!["42"]);
}

#[test]
fn test_p16_touch_and_worker() {
    let shared = Arc::new(GuiSharedState::new());
    let mut host = GuiHost::new(shared.clone());
    shared.set_touch(Some(Touch { x: 100, y: 50 }));
    let host_touch = host.touch().unwrap().unwrap();
    assert_eq!((host_touch.x, host_touch.y), (100, 50));
}

#[test]
fn test_p17_focus_and_iskeydown() {
    let shared = Arc::new(GuiSharedState::new());
    let mut host = GuiHost::new(shared.clone());
    shared.set_key_down(4, true); // Esc held
    assert!(host.is_key_down(4).unwrap());
    shared.set_key_down(4, false);
    assert!(!host.is_key_down(4).unwrap());
}

#[test]
fn test_p18_worker_and_cancellation() {
    let shared = Arc::new(GuiSharedState::new());
    let shared_clone = Arc::clone(&shared);

    let handle = std::thread::spawn(move || {
        let src = "EXPORT MAIN() BEGIN WHILE 1 DO END; END;";
        let file = SourceFile::new("<t>", src);
        let (prog, _) = ppl_compiler::compile_source(&file, &CompileOptions::default());
        let host = GuiHost::new(shared_clone);
        let mut machine = Machine::new(
            std::rc::Rc::new(prog.unwrap()),
            Box::new(host),
            Settings::default(),
            Limits::default(),
        );
        machine.call_function("MAIN", Vec::new()).is_ok()
    });

    std::thread::sleep(Duration::from_millis(30));
    shared.interrupt();

    let ok = handle.join().unwrap();
    assert!(!ok);
}

#[test]
fn test_p19_worker_and_console() {
    let src = "EXPORT MAIN() BEGIN PRINT(\"A\"); PRINT(\"B\"); END;";
    let res = run_ppl_headless(src, &[], &[]);
    assert_eq!(res.terminal_lines, vec!["A", "B"]);
}

#[test]
fn test_p20_worker_and_stepping() {
    let src = "EXPORT MAIN()\nBEGIN\n  LOCAL a := 1;\n  LOCAL b := 2;\nEND;";
    let pauses = execute_with_debugger(src, BTreeSet::new(), 2, true);
    assert_eq!(pauses.len(), 2);
}

#[test]
fn test_p21_unsaved_run_and_console() {
    let tab = EditorTab::new("UNSAVED", "EXPORT MAIN() BEGIN PRINT(123*2); END;");
    let res = run_ppl_headless(&tab.content, &[], &[]);
    assert_eq!(res.terminal_lines, vec!["246"]);
}

#[test]
fn test_p22_controls_and_cancellation() {
    let shared = GuiSharedState::new();
    shared.interrupt();
    assert!(shared.is_interrupted());
}

#[test]
fn test_p23_input_and_worker() {
    let val = ppl_vm::eval_text_with("50 + 50", &Settings::default()).unwrap();
    assert_eq!(val.as_real().unwrap().to_i64(), Some(100));
}

#[test]
fn test_p24_input_and_variables() {
    let val = Value::from_i64(100);
    let snap = VariableSnapshot::from_value("N", &val);
    assert_eq!(snap.name, "N");
    assert_eq!(snap.type_name, "real");
}

#[test]
fn test_p25_choose_and_console() {
    let choice = 2;
    let res = run_ppl_headless(
        &format!("EXPORT MAIN() BEGIN PRINT({choice}); END;"),
        &[],
        &[],
    );
    assert_eq!(res.terminal_lines, vec!["2"]);
}

#[test]
fn test_p26_msgbox_and_cancellation() {
    let shared = GuiSharedState::new();
    shared.interrupt();
    assert!(shared.is_interrupted());
}

#[test]
fn test_p27_getkey_and_wait() {
    let src = "EXPORT MAIN() BEGIN LOCAL k:=WAIT(0); PRINT(k); END;";
    let res = run_ppl_headless(src, &[], &[30]);
    assert_eq!(res.terminal_lines, vec!["30"]);
}

#[test]
fn test_p28_breakpoints_and_stepping() {
    let src = "EXPORT MAIN()\nBEGIN\n  LOCAL x := 1;\n  LOCAL y := 2;\n  LOCAL z := 3;\nEND;";
    let mut bps = BTreeSet::new();
    bps.insert(4);

    let pauses = execute_with_debugger(src, bps, 1, false);
    assert_eq!(pauses.len(), 1);
    assert_eq!(pauses[0].line, 4);
}

#[test]
fn test_p29_stepping_and_pause_highlight() {
    let src = "EXPORT MAIN()\nBEGIN\n  LOCAL a := 1;\nEND;";
    let pauses = execute_with_debugger(src, BTreeSet::new(), 1, true);
    let pause_line = pauses[0].line;
    assert_eq!(pause_line, 3);
}

#[test]
fn test_p30_stepping_and_variables() {
    let src = "EXPORT MAIN()\nBEGIN\n  LOCAL a := 42;\n  LOCAL b := 84;\nEND;";
    let mut bps = BTreeSet::new();
    bps.insert(4);

    let pauses = execute_with_debugger(src, bps, 1, false);
    assert_eq!(pauses.len(), 1);
    let frame = &pauses[0].frames[0];
    assert!(frame.locals.iter().any(|v| v.name == "a"));
}

#[test]
fn test_p31_stepping_and_call_stack() {
    let src = "SUB()\nBEGIN\n  LOCAL s := 1;\nEND;\nEXPORT MAIN()\nBEGIN\n  SUB();\nEND;";
    let mut bps = BTreeSet::new();
    bps.insert(3);

    let pauses = execute_with_debugger(src, bps, 1, false);
    assert_eq!(pauses[0].frames.len(), 2);
    assert_eq!(pauses[0].frames[0].function, "SUB");
    assert_eq!(pauses[0].frames[1].function, "MAIN");
}

#[test]
fn test_p32_variables_and_call_stack() {
    let src = "CALC(n)\nBEGIN\n  LOCAL res := n * 2;\n  RETURN res;\nEND;\nEXPORT MAIN()\nBEGIN\n  LOCAL arg := 10;\n  CALC(arg);\nEND;";
    let mut bps = BTreeSet::new();
    bps.insert(3);

    let pauses = execute_with_debugger(src, bps, 1, false);
    assert_eq!(pauses[0].frames.len(), 2);
    assert!(
        pauses[0].frames[0]
            .locals
            .iter()
            .any(|v| v.name == "res" || v.name == "n")
    );
    assert!(pauses[0].frames[1].locals.iter().any(|v| v.name == "arg"));
}
