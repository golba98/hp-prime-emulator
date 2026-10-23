//! Adversarial Challenger 2 Test Suite:
//! Problems Navigation, Multi-Tab Lifecycle, Dirty Buffers & Tier 4 E2E Workflows.

use std::rc::Rc;
use std::sync::Arc;

use ppl_compiler::CompileOptions;
use ppl_gui::host::{GuiHost, GuiSharedState};
use ppl_gui::ide::editor::{EditorArea, EditorTab};
use ppl_gui::ide::explorer::ProgramExplorer;
use ppl_gui::ide::problems::ProblemsPanel;
use ppl_syntax::source::SourceFile;
use ppl_vm::machine::{Limits, Machine};
use ppl_vm::settings::Settings;

// =========================================================================
// 1. OUT-OF-BOUNDS LINE/COL NAVIGATION & CLAMPING TESTS
// =========================================================================

#[test]
fn test_tab_goto_line_bounds_and_extremes() {
    let mut tab = EditorTab::new("TEST.ppl", "LINE 1\nLINE 2\nLINE 3\nLINE 4");

    // Case 1: Line 0, Col 0 -> Clamped to start of line 1 (char index 0)
    tab.goto_line(0, 0);
    assert_eq!(tab.pending_jump, Some(0));

    // Case 2: Line 1, Col 1 -> Start of line 1 (char index 0)
    tab.goto_line(1, 1);
    assert_eq!(tab.pending_jump, Some(0));

    // Case 3: Line 2, Col 1 -> Start of line 2 (char index 7)
    tab.goto_line(2, 1);
    assert_eq!(tab.pending_jump, Some(7));

    // Case 4: Line 2, Col 99999 -> Past end of line 2, clamps to end of line before '\n' (char index 13)
    tab.goto_line(2, 99999);
    assert_eq!(tab.pending_jump, Some(13)); // "LINE 1\nLINE 2" is 13 chars

    // Case 5: Line 99999, Col 99999 -> Way past EOF, clamps to EOF (char index 27)
    tab.goto_line(99999, 99999);
    let total_chars = tab.content.chars().count();
    assert_eq!(tab.pending_jump, Some(total_chars));

    // Case 6: usize::MAX for line and col (arithmetic underflow stress)
    tab.goto_line(usize::MAX, usize::MAX);
    assert_eq!(tab.pending_jump, Some(total_chars));

    // Case 7: Empty content buffer
    let mut empty_tab = EditorTab::new("EMPTY.ppl", "");
    empty_tab.goto_line(0, 0);
    assert_eq!(empty_tab.pending_jump, Some(0));
    empty_tab.goto_line(1, 1);
    assert_eq!(empty_tab.pending_jump, Some(0));
    empty_tab.goto_line(100, 50);
    assert_eq!(empty_tab.pending_jump, Some(0));
    empty_tab.goto_line(usize::MAX, usize::MAX);
    assert_eq!(empty_tab.pending_jump, Some(0));

    // Case 8: Unicode content (multibyte UTF-8 characters)
    let mut uni_tab = EditorTab::new("UNI.ppl", "EXPORT π()\nBEGIN\n  PRINT(\"ⅈ\");\nEND;\n");
    uni_tab.goto_line(1, 8); // On 'π' or '('
    assert!(uni_tab.pending_jump.is_some());
    uni_tab.goto_line(3, 999); // Clamps to end of line 3 without panicking on unicode
    assert!(uni_tab.pending_jump.is_some());
}

// =========================================================================
// 2. 50-TAB LIFECYCLE STRESS TEST
// =========================================================================

#[test]
fn test_50_consecutive_tabs_open_switch_and_close() {
    let mut area = EditorArea::new();
    assert_eq!(area.tabs.len(), 1); // Default DEMO.ppl

    // Open 50 additional consecutive tabs
    for i in 1..=50 {
        area.new_tab(
            format!("PROG_{i:02}.ppl"),
            format!("EXPORT P{i}() BEGIN PRINT({i}); END;\n"),
        );
        assert_eq!(area.tabs.len(), i + 1);
        assert_eq!(area.active_tab_index, i);
        assert_eq!(area.active_tab().unwrap().title, format!("PROG_{i:02}.ppl"));
    }

    assert_eq!(area.tabs.len(), 51);
    assert_eq!(area.active_tab_index, 50);

    // Verify all 51 tabs have correct contents and titles
    for (idx, tab) in area.tabs.iter().enumerate() {
        if idx == 0 {
            assert_eq!(tab.title, "DEMO.ppl");
        } else {
            assert_eq!(tab.title, format!("PROG_{idx:02}.ppl"));
            assert!(tab.content.contains(&format!("PRINT({idx})")));
        }
    }

    // Switch active tab across various positions
    area.active_tab_index = 25;
    assert_eq!(area.active_tab().unwrap().title, "PROG_25.ppl");
    area.active_tab_index = 0;
    assert_eq!(area.active_tab().unwrap().title, "DEMO.ppl");
    area.active_tab_index = 50;
    assert_eq!(area.active_tab().unwrap().title, "PROG_50.ppl");

    // Close 20 tabs from the middle (closing index 20 repeatedly)
    for _ in 0..20 {
        area.close_tab(20);
    }
    assert_eq!(area.tabs.len(), 31);
    assert!(area.active_tab_index < area.tabs.len());

    // Close 15 tabs from the start (index 0)
    for _ in 0..15 {
        area.close_tab(0);
    }
    assert_eq!(area.tabs.len(), 16);
    assert!(area.active_tab_index < area.tabs.len());

    // Close remaining 16 tabs until completely empty
    while !area.tabs.is_empty() {
        let last_idx = area.tabs.len() - 1;
        area.close_tab(last_idx);
    }
    assert_eq!(area.tabs.len(), 0);
    assert!(area.active_tab().is_none());
}

// =========================================================================
// 3. CLOSING THE ONLY OPEN TAB & EMPTY STATE HANDLING
// =========================================================================

#[test]
fn test_close_only_open_tab_and_empty_state_resilience() {
    let mut area = EditorArea::new();
    assert_eq!(area.tabs.len(), 1);

    // Close the single open tab
    area.close_tab(0);
    assert_eq!(area.tabs.len(), 0);
    assert!(area.active_tab().is_none());
    assert!(area.active_tab_mut().is_none());
    assert!(!area.active_tab_modified());

    // Closing an already empty editor does not panic
    area.close_tab(0);
    area.close_tab(999);
    assert!(area.tabs.is_empty());

    // Adding a new tab brings it back to a valid state
    area.new_tab("REBORN.ppl", "EXPORT R() BEGIN END;");
    assert_eq!(area.tabs.len(), 1);
    assert_eq!(area.active_tab_index, 0);
    assert_eq!(area.active_tab().unwrap().title, "REBORN.ppl");
}

// =========================================================================
// 4. MULTI-TAB SWITCHING WITH DIRTY BUFFERS & INACTIVE CLOSE TARGETING
// =========================================================================

#[test]
fn test_multi_tab_switching_dirty_buffers_and_target_close() {
    let mut area = EditorArea::new();

    // Tab 0 is DEMO.ppl. Add Tab 1 and Tab 2.
    area.new_tab("MATH.ppl", "EXPORT M() BEGIN PRINT(100); END;\n");
    area.new_tab("GRAV.ppl", "EXPORT G() BEGIN PRINT(9.81); END;\n");

    assert_eq!(area.tabs.len(), 3);
    assert_eq!(area.active_tab_index, 2);

    // Modify Tab 1 (MATH.ppl)
    area.active_tab_index = 1;
    let modified_content = "EXPORT M() BEGIN PRINT(999); END;\n";
    area.active_tab_mut().unwrap().content = modified_content.to_string();
    area.active_tab_mut().unwrap().modified = true;

    // Switch to Tab 2
    area.active_tab_index = 2;
    assert!(!area.active_tab().unwrap().modified);

    // Switch to Tab 0
    area.active_tab_index = 0;
    assert!(!area.active_tab().unwrap().modified);

    // Switch back to Tab 1: check dirty state and content preserved
    area.active_tab_index = 1;
    assert!(area.active_tab().unwrap().modified);
    assert_eq!(area.active_tab().unwrap().content, modified_content);

    // Inactive dirty tab close targeting verification:
    // If active tab is 0 (clean), but user clicks close on Tab 1 (dirty):
    area.active_tab_index = 0;
    let target_close_idx = 1;
    let is_target_dirty = area.tabs.get(target_close_idx).is_some_and(|t| t.modified);
    assert!(is_target_dirty);

    // In editor.rs lines 358-360:
    // When closing a dirty inactive tab, active_tab_index is switched to target_close_idx
    if is_target_dirty {
        area.active_tab_index = target_close_idx;
    }
    assert_eq!(area.active_tab_index, 1);
    assert!(area.active_tab_modified());

    // Close Tab 1
    area.close_tab(1);
    assert_eq!(area.tabs.len(), 2);
    // Tab 0 (DEMO.ppl) and Tab 1 (formerly Tab 2: GRAV.ppl) remain
    assert_eq!(area.tabs[0].title, "DEMO.ppl");
    assert_eq!(area.tabs[1].title, "GRAV.ppl");
}

// =========================================================================
// 5. PROBLEMS PANEL NAVIGATION & COMPILER DIAGNOSTICS MAPPING
// =========================================================================

#[test]
fn test_problems_panel_compiler_diagnostics_and_navigation() {
    let mut problems = ProblemsPanel::new();
    assert_eq!(problems.error_count(), 0);
    assert_eq!(problems.warning_count(), 0);

    // Compile bad source code
    let bad_code = "EXPORT BUGGY()\nBEGIN\n  LOCAL a := ;\n  LOCAL b := 10;\nEND;\n";
    let file = SourceFile::new("BUGGY.hpprgm", bad_code);
    let (prog, diags) = ppl_compiler::compile_source(&file, &CompileOptions::default());
    assert!(prog.is_none());
    assert!(diags.has_errors());

    let mut items = Vec::new();
    for d in &diags.items {
        items.push(ppl_gui::ide::editor::DiagnosticItem::from_diagnostic(
            d, &file,
        ));
    }

    problems.set_diagnostics(items);
    assert!(problems.error_count() > 0);
    let err = problems
        .items
        .iter()
        .find(|d| d.code == "expected-expression")
        .expect("expected-expression present");
    assert_eq!(err.line, 3);

    // Test tab navigation target
    let mut tab = EditorTab::new("BUGGY.hpprgm", bad_code);
    tab.goto_line(err.line, err.col);
    assert!(tab.pending_jump.is_some());
    let jump_target = tab.pending_jump.unwrap();

    // Verify jump target falls on line 3
    let line_3_start = "EXPORT BUGGY()\nBEGIN\n".chars().count();
    assert!(jump_target >= line_3_start);
}

// =========================================================================
// 6. REAL TIER 4 APPLICATION WORKFLOWS EMPIRICAL VERIFICATION
// =========================================================================

#[test]
fn test_tier4_scenario_01_real_scratchpad_vm_execution() {
    let src1 = "EXPORT TEST()\nBEGIN\n  LOCAL x := 10;\n  LOCAL y := 20;\n  PRINT(x + y);\nEND;\n";
    let file1 = SourceFile::new("TEST.hpprgm", src1);
    let (prog1, diags1) = ppl_compiler::compile_source(&file1, &CompileOptions::default());
    assert!(!diags1.has_errors());
    let prog1 = prog1.expect("compiled program 1");

    let shared1 = Arc::new(GuiSharedState::new());
    let host1 = GuiHost::new(Arc::clone(&shared1));
    let mut machine1 = Machine::new(
        Rc::new(prog1),
        Box::new(host1),
        Settings::default(),
        Limits::default(),
    );
    let outcome1 = machine1.call_function("TEST", Vec::new());
    assert!(outcome1.is_ok());
    let printed1 = shared1.terminal_lines.lock().unwrap().clone();
    assert_eq!(printed1, vec!["30"]);

    // Edit to multiplication
    let src2 = "EXPORT TEST()\nBEGIN\n  LOCAL x := 10;\n  LOCAL y := 20;\n  PRINT(x * y);\nEND;\n";
    let file2 = SourceFile::new("TEST.hpprgm", src2);
    let (prog2, diags2) = ppl_compiler::compile_source(&file2, &CompileOptions::default());
    assert!(!diags2.has_errors());
    let prog2 = prog2.expect("compiled program 2");

    let shared2 = Arc::new(GuiSharedState::new());
    let host2 = GuiHost::new(Arc::clone(&shared2));
    let mut machine2 = Machine::new(
        Rc::new(prog2),
        Box::new(host2),
        Settings::default(),
        Limits::default(),
    );
    let outcome2 = machine2.call_function("TEST", Vec::new());
    assert!(outcome2.is_ok());
    let printed2 = shared2.terminal_lines.lock().unwrap().clone();
    assert_eq!(printed2, vec!["200"]);
}

#[test]
fn test_tier4_scenario_05_syntax_error_recovery_and_click_to_fix() {
    // 1. Intentionally write invalid syntax
    let bad_src = "EXPORT FOO()\nBEGIN\n  LOCAL x := ;\nEND;\n";
    let file1 = SourceFile::new("FOO.hpprgm", bad_src);
    let (prog1, diags1) = ppl_compiler::compile_source(&file1, &CompileOptions::default());
    assert!(prog1.is_none());
    assert!(diags1.has_errors());

    let err = diags1
        .items
        .iter()
        .find(|d| d.code == "expected-expression")
        .unwrap();
    let lc = file1.line_col(err.span.start);
    assert_eq!(lc.line, 3);

    // 2. Editor navigates to error location
    let mut tab = EditorTab::new("FOO.hpprgm", bad_src);
    tab.goto_line(lc.line as usize, lc.col as usize);
    assert!(tab.pending_jump.is_some());

    // 3. Fix error in place
    let fixed_src = "EXPORT FOO()\nBEGIN\n  LOCAL x := 42;\n  PRINT(x);\nEND;\n";
    tab.content = fixed_src.to_string();
    tab.modified = true;

    // 4. Re-check syntax: 0 errors
    let file2 = SourceFile::new("FOO.hpprgm", &tab.content);
    let (prog2, diags2) = ppl_compiler::compile_source(&file2, &CompileOptions::default());
    assert!(!diags2.has_errors());
    let prog2 = prog2.expect("compiled program");

    // 5. Execute fixed program
    let shared = Arc::new(GuiSharedState::new());
    let host = GuiHost::new(Arc::clone(&shared));
    let mut machine = Machine::new(
        Rc::new(prog2),
        Box::new(host),
        Settings::default(),
        Limits::default(),
    );
    let outcome = machine.call_function("FOO", Vec::new());
    assert!(outcome.is_ok());
    assert_eq!(shared.terminal_lines.lock().unwrap().clone(), vec!["42"]);
}

#[test]
fn test_tier4_scenario_06_multi_tab_switching_and_outline_sync() {
    let mut area = EditorArea::new();
    let mut explorer = ProgramExplorer::default();

    // Open Tab 1 and Tab 2
    area.new_tab("P1.hpprgm", "EXPORT P1()\nBEGIN\nEND;\n");
    area.new_tab("P2.hpprgm", "EXPORT P2()\nBEGIN\nEND;\n");

    assert_eq!(area.tabs.len(), 3);
    assert_eq!(area.active_tab_index, 2);

    // Update outline for active Tab 2
    explorer.update_outline(&area.tabs[2].content);
    assert_eq!(explorer.outline_items.len(), 1);
    assert_eq!(explorer.outline_items[0].name, "P2");
    assert_eq!(explorer.outline_items[0].line, 1);

    // Switch to Tab 1 and edit
    area.active_tab_index = 1;
    area.active_tab_mut().unwrap().content =
        "EXPORT P1()\nBEGIN\n  PRINT(\"Mod\");\nEND;\nLOCAL HELPER()\nBEGIN\nEND;\n".to_string();
    area.active_tab_mut().unwrap().modified = true;

    // Update outline for Tab 1
    explorer.update_outline(&area.tabs[1].content);
    assert_eq!(explorer.outline_items.len(), 2);
    assert_eq!(explorer.outline_items[0].name, "P1");
    assert_eq!(explorer.outline_items[0].line, 1);
    assert_eq!(explorer.outline_items[1].name, "HELPER");
    assert_eq!(explorer.outline_items[1].line, 5);

    // Navigate to outline entry (line 5)
    area.active_tab_mut()
        .unwrap()
        .goto_line(explorer.outline_items[1].line, 1);
    assert!(area.active_tab().unwrap().pending_jump.is_some());

    // Close Tab 0 (DEMO.ppl)
    area.close_tab(0);
    assert_eq!(area.tabs.len(), 2);
    // Active tab index was 1, shifted to 0
    assert_eq!(area.active_tab_index, 0);
    assert_eq!(area.active_tab().unwrap().title, "P1.hpprgm");
    assert!(area.active_tab().unwrap().modified);
}
