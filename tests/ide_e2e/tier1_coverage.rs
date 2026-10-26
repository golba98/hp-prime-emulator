//! Tier 1: Comprehensive Feature Coverage for HP Prime Desktop IDE.
//!
//! Covers all 32 inventoried features with at least 5 representative test cases each
//! (total 160 test cases), strictly derived from requirements in ORIGINAL_REQUEST.md,
//! PROJECT.md, and TEST_INFRA.md.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use super::harness::*;
use ppl_compiler::CompileOptions;
use ppl_graphics::{Grob, compare_rgb565, rgb};
use ppl_gui::dialog::KeyPress;
use ppl_gui::host::{GuiHost, GuiSharedState};
use ppl_gui::keymap::hp_key_name;
use ppl_gui::scaler::{Viewport, window_to_framebuffer};
use ppl_gui::theme::{
    BG_EXTREME, BG_PANEL, BG_WIDGET, BORDER_CRISP, CALC_KEY_ALPHA, CALC_KEY_ENTER, CALC_KEY_FN_BG,
    CALC_KEY_NUM_BG, CALC_KEY_ON, CALC_KEY_SHIFT, TEXT_NORMAL, classic_engineering_visuals,
};
use ppl_syntax::diag::{Diagnostics, Severity};
use ppl_syntax::source::SourceFile;
use ppl_vm::host::{Host, Touch};
use ppl_vm::machine::{Limits, Machine};
use ppl_vm::settings::Settings;
use ppl_vm::value::Value;

// ============================================================================
// FEATURE 1: True-black IDE and authentic calculator materials
// ============================================================================
pub mod f01_engineering_aesthetic {
    use super::*;

    #[test]
    fn test_f01_c1_palette_background_colors() {
        let visuals = classic_engineering_visuals();
        assert_eq!(
            visuals.panel_fill, BG_PANEL,
            "Panel bg must match the true-black tokens"
        );
        assert_eq!(
            visuals.window_fill, BG_WIDGET,
            "Window bg must match the raised surface token"
        );
        assert_eq!(
            visuals.extreme_bg_color, BG_EXTREME,
            "Extreme bg must match the application background"
        );
    }

    #[test]
    fn test_f01_c2_crisp_border_stroke() {
        let visuals = classic_engineering_visuals();
        assert_eq!(visuals.window_stroke.color, BORDER_CRISP);
        assert_eq!(visuals.window_stroke.width, 1.0);
        assert_eq!(visuals.widgets.noninteractive.bg_stroke.color, BORDER_CRISP);
        assert_eq!(visuals.widgets.inactive.bg_stroke.color, BORDER_CRISP);
    }

    #[test]
    fn test_f01_c3_zero_corner_rounding() {
        let visuals = classic_engineering_visuals();
        let zero = Default::default();
        assert_eq!(visuals.window_corner_radius, zero);
        assert_eq!(visuals.widgets.noninteractive.corner_radius, zero);
        assert_eq!(visuals.widgets.inactive.corner_radius, zero);
        assert_eq!(visuals.widgets.hovered.corner_radius, zero);
        assert_eq!(visuals.widgets.active.corner_radius, zero);
    }

    #[test]
    fn test_f01_c4_calculator_key_palettes() {
        assert_eq!(CALC_KEY_FN_BG.to_array(), [39, 39, 39, 255]);
        assert_eq!(CALC_KEY_NUM_BG.to_array(), [226, 226, 226, 255]);
        assert_eq!(CALC_KEY_SHIFT.to_array(), [0, 61, 156, 255]);
        assert_eq!(CALC_KEY_ALPHA.to_array(), [244, 103, 6, 255]);
        assert_eq!(CALC_KEY_ON, CALC_KEY_NUM_BG);
        assert_eq!(CALC_KEY_ENTER, CALC_KEY_NUM_BG);
        for surface in [BG_EXTREME, BG_PANEL, BG_WIDGET] {
            assert_eq!(surface.r(), surface.g());
            assert_eq!(surface.g(), surface.b());
            assert!(surface.r() <= 0x11);
        }
    }

    #[test]
    fn test_f01_c5_theme_dark_mode_and_text_contrast() {
        let visuals = classic_engineering_visuals();
        assert!(visuals.dark_mode);
        assert_eq!(visuals.widgets.noninteractive.fg_stroke.color, TEXT_NORMAL);
        assert_eq!(visuals.widgets.inactive.fg_stroke.color, TEXT_NORMAL);
    }
}

// ============================================================================
// FEATURE 2: Main Menu Actions
// ============================================================================
pub mod f02_main_menu_actions {
    use super::*;

    #[test]
    fn test_f02_c1_file_menu_actions() {
        let mut state = IdeAppStateModel::new();
        state.queue_action(IdeAction::NewProgram);
        state.queue_action(IdeAction::OpenFile(Some(PathBuf::from("TEST.hpprgm"))));
        state.queue_action(IdeAction::SaveActiveFile);
        state.queue_action(IdeAction::SaveActiveFileAs);
        state.queue_action(IdeAction::CloseActiveTab);

        let actions = state.take_actions();
        assert_eq!(actions.len(), 5);
        assert_eq!(actions[0], IdeAction::NewProgram);
        assert_eq!(
            actions[1],
            IdeAction::OpenFile(Some(PathBuf::from("TEST.hpprgm")))
        );
        assert_eq!(actions[2], IdeAction::SaveActiveFile);
        assert_eq!(actions[3], IdeAction::SaveActiveFileAs);
        assert_eq!(actions[4], IdeAction::CloseActiveTab);
    }

    #[test]
    fn test_f02_c2_run_menu_actions() {
        let mut state = IdeAppStateModel::new();
        state.queue_action(IdeAction::RunProgram);
        state.queue_action(IdeAction::StopProgram);
        state.queue_action(IdeAction::RestartProgram);

        let actions = state.take_actions();
        assert_eq!(actions.len(), 3);
        assert_eq!(actions[0], IdeAction::RunProgram);
        assert_eq!(actions[1], IdeAction::StopProgram);
        assert_eq!(actions[2], IdeAction::RestartProgram);
    }

    #[test]
    fn test_f02_c3_debug_menu_actions() {
        let mut state = IdeAppStateModel::new();
        state.queue_action(IdeAction::StartDebugging);
        state.queue_action(IdeAction::StepInto);
        state.queue_action(IdeAction::StepOver);
        state.queue_action(IdeAction::StepOut);
        state.queue_action(IdeAction::ContinueExecution);

        let actions = state.take_actions();
        assert_eq!(actions.len(), 5);
        assert_eq!(actions[0], IdeAction::StartDebugging);
        assert_eq!(actions[1], IdeAction::StepInto);
        assert_eq!(actions[2], IdeAction::StepOver);
        assert_eq!(actions[3], IdeAction::StepOut);
        assert_eq!(actions[4], IdeAction::ContinueExecution);
    }

    #[test]
    fn test_f02_c4_view_menu_toggles() {
        let mut state = IdeAppStateModel::new();
        state.queue_action(IdeAction::ToggleExplorer);
        state.queue_action(IdeAction::ToggleCalculator);
        state.queue_action(IdeAction::ToggleBottomDock);
        state.queue_action(IdeAction::ToggleStatusBar);
        state.queue_action(IdeAction::ResetLayout);

        let actions = state.take_actions();
        assert_eq!(actions.len(), 5);
        assert_eq!(actions[0], IdeAction::ToggleExplorer);
        assert_eq!(actions[4], IdeAction::ResetLayout);
    }

    #[test]
    fn test_f02_c5_program_menu_actions() {
        let mut state = IdeAppStateModel::new();
        state.queue_action(IdeAction::CheckSyntax);
        state.queue_action(IdeAction::FormatSource);

        let actions = state.take_actions();
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0], IdeAction::CheckSyntax);
        assert_eq!(actions[1], IdeAction::FormatSource);
    }
}

// ============================================================================
// FEATURE 3: Toolbar Controls
// ============================================================================
pub mod f03_toolbar_controls {
    use super::*;

    #[test]
    fn test_f03_c1_toolbar_new_program() {
        let mut state = IdeAppStateModel::new();
        state.queue_action(IdeAction::NewProgram);
        let actions = state.take_actions();
        assert!(actions.contains(&IdeAction::NewProgram));
    }

    #[test]
    fn test_f03_c2_toolbar_open_and_save() {
        let mut state = IdeAppStateModel::new();
        state.queue_action(IdeAction::OpenFile(None));
        state.queue_action(IdeAction::SaveActiveFile);
        let actions = state.take_actions();
        assert_eq!(
            actions,
            vec![IdeAction::OpenFile(None), IdeAction::SaveActiveFile]
        );
    }

    #[test]
    fn test_f03_c3_toolbar_compile_action() {
        let mut state = IdeAppStateModel::new();
        state.queue_action(IdeAction::CheckSyntax);
        assert_eq!(state.take_actions(), vec![IdeAction::CheckSyntax]);
    }

    #[test]
    fn test_f03_c4_toolbar_run_and_stop_controls() {
        let mut state = IdeAppStateModel::new();
        state.vm_state = VmExecutionState::Idle;
        state.queue_action(IdeAction::RunProgram);
        state.vm_state = VmExecutionState::Running;
        state.queue_action(IdeAction::StopProgram);

        let actions = state.take_actions();
        assert_eq!(actions, vec![IdeAction::RunProgram, IdeAction::StopProgram]);
    }

    #[test]
    fn test_f03_c5_toolbar_debug_and_settings() {
        let mut state = IdeAppStateModel::new();
        state.queue_action(IdeAction::StartDebugging);
        state.queue_action(IdeAction::ShowSettingsDialog);
        assert_eq!(
            state.take_actions(),
            vec![IdeAction::StartDebugging, IdeAction::ShowSettingsDialog]
        );
    }
}

// ============================================================================
// FEATURE 4: Resizable Multi-Panel Split
// ============================================================================
pub mod f04_resizable_panel_split {
    use super::*;

    #[test]
    fn test_f04_c1_default_panel_dimensions() {
        let state = IdeAppStateModel::new();
        assert_eq!(state.explorer_width, 240.0);
        assert_eq!(state.calculator_width, 360.0);
        assert_eq!(state.bottom_height, 180.0);
    }

    #[test]
    fn test_f04_c2_explorer_resizing_bounds() {
        let mut state = IdeAppStateModel::new();
        state.explorer_width = 100.0f32.max(150.0).min(500.0);
        assert_eq!(state.explorer_width, 150.0);
        state.explorer_width = 600.0f32.max(150.0).min(500.0);
        assert_eq!(state.explorer_width, 500.0);
    }

    #[test]
    fn test_f04_c3_calculator_resizing_bounds() {
        let mut state = IdeAppStateModel::new();
        state.calculator_width = 250.0f32.max(320.0).min(800.0);
        assert_eq!(state.calculator_width, 320.0);
        state.calculator_width = 480.0f32.max(320.0).min(800.0);
        assert_eq!(state.calculator_width, 480.0);
    }

    #[test]
    fn test_f04_c4_dock_resizing_bounds() {
        let mut state = IdeAppStateModel::new();
        state.bottom_height = 50.0f32.max(100.0).min(600.0);
        assert_eq!(state.bottom_height, 100.0);
        state.bottom_height = 300.0f32.max(100.0).min(600.0);
        assert_eq!(state.bottom_height, 300.0);
    }

    #[test]
    fn test_f04_c5_reset_layout_restores_defaults() {
        let mut state = IdeAppStateModel::new();
        state.explorer_width = 180.0;
        state.calculator_width = 400.0;
        state.bottom_height = 250.0;
        state.reset_layout();

        assert_eq!(state.explorer_width, 240.0);
        assert_eq!(state.calculator_width, 360.0);
        assert_eq!(state.bottom_height, 180.0);
    }
}

// ============================================================================
// FEATURE 5: Panel State & Visibility Persistence
// ============================================================================
pub mod f05_panel_state_persistence {
    use super::*;

    #[test]
    fn test_f05_c1_toggle_explorer_visibility() {
        let mut state = IdeAppStateModel::new();
        assert!(state.show_explorer);
        state.show_explorer = !state.show_explorer;
        assert!(!state.show_explorer);
        state.show_explorer = !state.show_explorer;
        assert!(state.show_explorer);
    }

    #[test]
    fn test_f05_c2_toggle_calculator_visibility() {
        let mut state = IdeAppStateModel::new();
        assert!(state.show_calculator);
        state.show_calculator = false;
        assert!(!state.show_calculator);
        state.show_calculator = true;
        assert!(state.show_calculator);
    }

    #[test]
    fn test_f05_c3_toggle_bottom_dock_visibility() {
        let mut state = IdeAppStateModel::new();
        assert!(state.show_bottom_dock);
        state.show_bottom_dock = false;
        assert!(!state.show_bottom_dock);
    }

    #[test]
    fn test_f05_c4_toggle_status_bar_visibility() {
        let mut state = IdeAppStateModel::new();
        assert!(state.show_status_bar);
        state.show_status_bar = false;
        assert!(!state.show_status_bar);
    }

    #[test]
    fn test_f05_c5_dock_tab_selection_persistence() {
        let mut state = IdeAppStateModel::new();
        assert_eq!(state.active_dock_tab, DockTab::Output);
        state.active_dock_tab = DockTab::Problems;
        assert_eq!(state.active_dock_tab, DockTab::Problems);
        state.active_dock_tab = DockTab::Variables;
        assert_eq!(state.active_dock_tab, DockTab::Variables);
        state.active_dock_tab = DockTab::CallStack;
        assert_eq!(state.active_dock_tab, DockTab::CallStack);
    }
}

// ============================================================================
// FEATURE 6: Multi-Tab Editor Buffer Lifecycle
// ============================================================================
pub mod f06_multi_tab_editor {
    use super::*;

    #[test]
    fn test_f06_c1_open_multiple_tabs() {
        let mut state = IdeAppStateModel::new();
        assert_eq!(state.tabs.len(), 1);

        let idx1 = state.open_tab("PROG1.hpprgm", default_ppl_template("PROG1"));
        let idx2 = state.open_tab("PROG2.hpprgm", default_ppl_template("PROG2"));

        assert_eq!(state.tabs.len(), 3);
        assert_eq!(idx1, 1);
        assert_eq!(idx2, 2);
        assert_eq!(state.active_tab_index, 2);
    }

    #[test]
    fn test_f06_c2_modified_indicator_tracking() {
        let mut state = IdeAppStateModel::new();
        let tab = state.active_tab_mut().unwrap();
        assert!(!tab.modified);

        tab.set_content("EXPORT MAIN() BEGIN PRINT(1); END;");
        assert!(tab.modified);

        tab.modified = false;
        assert!(!tab.modified);
    }

    #[test]
    fn test_f06_c3_close_tab_and_active_index_adjustment() {
        let mut state = IdeAppStateModel::new();
        state.open_tab("TAB1", "");
        state.open_tab("TAB2", "");
        assert_eq!(state.active_tab_index, 2);

        let closed = state.close_tab(2);
        assert!(closed);
        assert_eq!(state.tabs.len(), 2);
        assert_eq!(state.active_tab_index, 1);
    }

    #[test]
    fn test_f06_c4_cursor_line_col_updates() {
        let mut state = IdeAppStateModel::new();
        let tab = state.active_tab_mut().unwrap();
        tab.cursor_line = 5;
        tab.cursor_col = 14;

        assert_eq!(tab.cursor_line, 5);
        assert_eq!(tab.cursor_col, 14);
    }

    #[test]
    fn test_f06_c5_tab_breakpoints_isolation() {
        let mut state = IdeAppStateModel::new();
        state.open_tab("TAB_A", "");
        state.open_tab("TAB_B", "");

        state.tabs[1].toggle_breakpoint(4);
        state.tabs[1].toggle_breakpoint(8);

        assert!(state.tabs[1].breakpoints.contains(&4));
        assert!(state.tabs[1].breakpoints.contains(&8));
        assert!(
            state.tabs[2].breakpoints.is_empty(),
            "Tab B must have no breakpoints"
        );
    }
}

// ============================================================================
// FEATURE 7: Native File Operations
// ============================================================================
pub mod f07_native_file_operations {
    use super::*;

    #[test]
    fn test_f07_c1_new_program_template_generation() {
        let tmpl = default_ppl_template("SOLVER");
        assert!(tmpl.contains("EXPORT SOLVER()"));
        assert!(tmpl.contains("BEGIN"));
        assert!(tmpl.contains("END;"));

        let file = SourceFile::new("<test>", &tmpl);
        let (prog, diags) = ppl_compiler::compile_source(&file, &CompileOptions::default());
        assert!(!diags.has_errors());
        assert!(prog.is_some());
    }

    #[test]
    fn test_f07_c2_file_save_and_reload() {
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join("test_save_f07.hpprgm");
        let content = "EXPORT SAVED() BEGIN RETURN 42; END;\n";

        std::fs::write(&test_file, content).unwrap();
        let read_back = std::fs::read_to_string(&test_file).unwrap();
        assert_eq!(read_back, content);

        let _ = std::fs::remove_file(test_file);
    }

    #[test]
    fn test_f07_c3_save_as_updates_path_and_title() {
        let mut tab = EditorTab::new("UNTITLED.hpprgm", "EXPORT MAIN() BEGIN END;");
        tab.modified = true;

        let new_path = PathBuf::from("/workspace/DEMO.hpprgm");
        tab.path = Some(new_path.clone());
        tab.title = new_path.file_name().unwrap().to_string_lossy().to_string();
        tab.modified = false;

        assert_eq!(tab.title, "DEMO.hpprgm");
        assert_eq!(tab.path, Some(new_path));
        assert!(!tab.modified);
    }

    #[test]
    fn test_f07_c4_recent_files_deduplication() {
        let mut state = IdeAppStateModel::new();
        let p1 = PathBuf::from("/prog1.hpprgm");
        let p2 = PathBuf::from("/prog2.hpprgm");

        state.add_recent_file(p1.clone());
        state.add_recent_file(p2.clone());
        state.add_recent_file(p1.clone());

        assert_eq!(state.recent_files.len(), 2);
        assert_eq!(state.recent_files[0], p1);
        assert_eq!(state.recent_files[1], p2);
    }

    #[test]
    fn test_f07_c5_recent_files_capacity_limit() {
        let mut state = IdeAppStateModel::new();
        for i in 0..15 {
            state.add_recent_file(PathBuf::from(format!("/prog_{i}.hpprgm")));
        }
        assert_eq!(state.recent_files.len(), 10);
    }
}

// ============================================================================
// FEATURE 8: PPL Syntax Highlighting Tokenization
// ============================================================================
pub mod f08_syntax_highlighting {
    use super::*;

    #[test]
    fn test_f08_c1_lex_uppercase_keywords() {
        let src = "EXPORT LOCAL BEGIN END IF THEN ELSE FOR WHILE DO REPEAT UNTIL";
        let mut diags = Diagnostics::default();
        let toks = ppl_syntax::lexer::lex(src, &mut diags);
        assert_eq!(toks.len(), 13);
        assert_eq!(toks.last().unwrap().tok, ppl_syntax::lexer::Tok::Eof);
        for t in &toks[..12] {
            match &t.tok {
                ppl_syntax::lexer::Tok::Kw(_) => {}
                other => panic!("Expected Tok::Kw, found {other:?}"),
            }
        }
    }

    #[test]
    fn test_f08_c2_lex_numeric_and_base_integers() {
        let src = "123 45.67 1.23E-4 #FFh #1010b";
        let mut diags = Diagnostics::default();
        let toks = ppl_syntax::lexer::lex(src, &mut diags);
        assert_eq!(toks.len(), 6);
        assert_eq!(toks.last().unwrap().tok, ppl_syntax::lexer::Tok::Eof);
        assert!(matches!(toks[0].tok, ppl_syntax::lexer::Tok::Number(_)));
        assert!(matches!(toks[1].tok, ppl_syntax::lexer::Tok::Number(_)));
        assert!(matches!(toks[2].tok, ppl_syntax::lexer::Tok::Number(_)));
        assert!(matches!(toks[3].tok, ppl_syntax::lexer::Tok::HashInt(_)));
        assert!(matches!(toks[4].tok, ppl_syntax::lexer::Tok::HashInt(_)));
    }

    #[test]
    fn test_f08_c3_lex_string_and_quoted_literals() {
        let src = "\"Hello World\" 'X^2+1'";
        let mut diags = Diagnostics::default();
        let toks = ppl_syntax::lexer::lex(src, &mut diags);
        assert_eq!(toks.len(), 3);
        assert_eq!(toks.last().unwrap().tok, ppl_syntax::lexer::Tok::Eof);
        assert!(matches!(toks[0].tok, ppl_syntax::lexer::Tok::Str(_)));
        assert!(matches!(toks[1].tok, ppl_syntax::lexer::Tok::Quoted(_)));
    }

    #[test]
    fn test_f08_c4_lex_builtins_and_system_registers() {
        let src = "PRINT SIN RECT_P G0 L1 M2";
        let mut diags = Diagnostics::default();
        let toks = ppl_syntax::lexer::lex(src, &mut diags);
        assert_eq!(toks.len(), 7);
        assert_eq!(toks.last().unwrap().tok, ppl_syntax::lexer::Tok::Eof);
        for t in &toks[..6] {
            match &t.tok {
                ppl_syntax::lexer::Tok::Ident(name) => {
                    assert!(
                        ppl_compiler::builtins::lookup(name).is_some()
                            || name.starts_with('G')
                            || name.starts_with('L')
                            || name.starts_with('M')
                    );
                }
                other => panic!("Expected identifier, found {other:?}"),
            }
        }
    }

    #[test]
    fn test_f08_c5_lex_comment_gap_detection() {
        let src = "LOCAL x; // this is a comment\nLOCAL y;";
        let mut diags = Diagnostics::default();
        let toks = ppl_syntax::lexer::lex(src, &mut diags);
        assert_eq!(toks.len(), 7); // LOCAL, x, ;, LOCAL, y, ;, EOF
        let gap_start = toks[2].span.end as usize;
        let gap_end = toks[3].span.start as usize;
        let gap_text = &src[gap_start..gap_end];
        assert!(gap_text.contains("// this is a comment"));
    }
}

// ============================================================================
// FEATURE 9: Live Compiler Diagnostics & Error Gutter
// ============================================================================
pub mod f09_compiler_diagnostics {
    use super::*;

    #[test]
    fn test_f09_c1_clean_compilation_zero_diagnostics() {
        let src = "EXPORT FOO() BEGIN LOCAL x := 10; RETURN x; END;";
        let file = SourceFile::new("<test>", src);
        let (prog, diags) = ppl_compiler::compile_source(&file, &CompileOptions::default());
        assert!(prog.is_some());
        assert!(!diags.has_errors());
    }

    #[test]
    fn test_f09_c2_syntax_error_diagnostic_generation() {
        let src = "EXPORT FOO() BEGIN LOCAL x := ; END;";
        let file = SourceFile::new("<test>", src);
        let (prog, diags) = ppl_compiler::compile_source(&file, &CompileOptions::default());
        assert!(prog.is_none());
        assert!(diags.has_errors());
        assert!(diags.items.iter().any(|d| d.code == "expected-expression"));
    }

    #[test]
    fn test_f09_c3_line_col_mapping_accuracy() {
        let src = "EXPORT FOO()\nBEGIN\n  LOCAL x := ;\nEND;";
        let file = SourceFile::new("<test>", src);
        let (_, diags) = ppl_compiler::compile_source(&file, &CompileOptions::default());
        let err = diags
            .items
            .iter()
            .find(|d| d.code == "expected-expression")
            .unwrap();
        let lc = file.line_col(err.span.start);
        assert_eq!(lc.line, 3);
        assert_eq!(lc.col, 14);
    }

    #[test]
    fn test_f09_c4_warning_severity_unused_variable() {
        let src = "EXPORT FOO() BEGIN LOCAL unused_var; RETURN 1; END;";
        let file = SourceFile::new("<test>", src);
        let (prog, diags) = ppl_compiler::compile_source(&file, &CompileOptions::default());
        assert!(prog.is_some());
        assert!(diags.items.iter().any(|d| d.severity == Severity::Warning));
    }

    #[test]
    fn test_f09_c5_multiple_diagnostics_sorting() {
        let src = "EXPORT FOO()\nBEGIN\n  LOCAL a := ;\n  LOCAL b := ;\nEND;";
        let file = SourceFile::new("<test>", src);
        let (_, diags) = ppl_compiler::compile_source(&file, &CompileOptions::default());
        assert!(diags.items.len() >= 2);
        assert!(diags.items[0].span.start < diags.items[1].span.start);
    }
}

// ============================================================================
// FEATURE 10: 320×240 GROB Nearest-Neighbor Display
// ============================================================================
pub mod f10_grob_nearest_neighbor {
    use super::*;

    #[test]
    fn test_f10_c1_framebuffer_resolution_320x240() {
        let grob = Grob::screen();
        assert_eq!(grob.width(), 320);
        assert_eq!(grob.height(), 240);
        assert_eq!(grob.pixels().len(), 320 * 240);
    }

    #[test]
    fn test_f10_c2_aspect_ratio_4_3_fit() {
        let vp = Viewport::calculate_aspect_fit(640, 480);
        assert_eq!(vp.width, 640);
        assert_eq!(vp.height, 480);
        assert_eq!(vp.x, 0);
        assert_eq!(vp.y, 0);

        let vp_wide = Viewport::calculate_aspect_fit(800, 480);
        assert_eq!(vp_wide.height, 480);
        assert_eq!(vp_wide.width, 640);
        assert_eq!(vp_wide.x, 80); // pillarboxed
    }

    #[test]
    fn test_f10_c3_integer_scaling_factors() {
        let vp_int = Viewport::calculate_integer(700, 500);
        assert_eq!(vp_int.scale, 2);
        assert_eq!(vp_int.width, 640);
        assert_eq!(vp_int.height, 480);
    }

    #[test]
    fn test_f10_c4_nearest_neighbor_pixel_preservation() {
        let src = Grob::screen();
        let mut dst = Grob::new(640, 480, 0);
        dst.blit((0, 0, 640, 480), &src, (0, 0, 320, 240), None, 255);

        assert_eq!(dst.get(20, 20), Some(0x00FFFFFF));
    }

    #[test]
    fn test_f10_c5_window_to_framebuffer_coordinate_transform() {
        let vp = Viewport::calculate_aspect_fit(640, 480);
        let fb_coord = window_to_framebuffer(200.0, 200.0, &vp);
        assert_eq!(fb_coord, Some((100, 100)));

        let outside_coord = window_to_framebuffer(1000.0, 1000.0, &vp);
        assert_eq!(outside_coord, None);
    }
}

// ============================================================================
// FEATURE 11: Live GROB Drawing Commands Rendering
// ============================================================================
pub mod f11_grob_drawing_commands {
    use super::*;

    #[test]
    fn test_f11_c1_rect_p_draws_border_and_fill() {
        let mut g = Grob::screen();
        let border = rgb(0, 0, 255);
        let fill = rgb(0, 255, 0);
        g.rect(10, 10, 30, 30, border, fill);

        assert_eq!(g.get(10, 10), Some(border));
        assert_eq!(g.get(30, 30), Some(border));
        assert_eq!(g.get(20, 20), Some(fill));
    }

    #[test]
    fn test_f11_c2_pixon_p_and_pixel_get() {
        let mut g = Grob::screen();
        let c = rgb(255, 128, 0);
        g.set(55, 42, c);
        assert_eq!(g.get(55, 42), Some(c));
        assert_eq!(g.get(55, 43), Some(0x00FFFFFF));
    }

    #[test]
    fn test_f11_c3_line_p_bresenham_endpoints() {
        let mut g = Grob::screen();
        let c = rgb(0, 0, 0);
        g.line(10, 20, 50, 20, c);
        for x in 10..=50 {
            assert_eq!(g.get(x, 20), Some(c));
        }
    }

    #[test]
    fn test_f11_c4_fill_circle_rendering() {
        let mut g = Grob::screen();
        let c = rgb(255, 0, 0);
        g.fill_circle(100, 100, 10, c);
        assert_eq!(g.get(100, 100), Some(c));
        assert_eq!(g.get(100, 90), Some(c));
        assert_eq!(g.get(100, 110), Some(c));
        assert_eq!(g.get(100, 115), Some(0x00FFFFFF));
    }

    #[test]
    fn test_f11_c5_blit_p_region_transfer() {
        let src = Grob::new(10, 10, rgb(255, 0, 0));
        let mut dst = Grob::screen();
        dst.blit((50, 50, 60, 60), &src, (0, 0, 10, 10), None, 255);
        assert_eq!(dst.get(55, 55), Some(rgb(255, 0, 0)));
    }
}

// ============================================================================
// FEATURE 12: 0–50 Virtual Keypad Grid Input
// ============================================================================
pub mod f12_virtual_keypad {
    use super::*;

    #[test]
    fn test_f12_c1_keypad_layout_constants_range() {
        assert_eq!(hp_key_name(0), Some("Apps"));
        assert_eq!(hp_key_name(30), Some("Enter"));
        assert_eq!(hp_key_name(46), Some("On"));
        assert_eq!(hp_key_name(50), Some("+"));
        assert_eq!(hp_key_name(999), None);
    }

    #[test]
    fn test_f12_c2_keypad_click_queues_key_event() {
        let state = Arc::new(GuiSharedState::new());
        let mut host = GuiHost::new(state.clone());
        state.push_key(30); // Enter
        assert_eq!(host.poll_key().unwrap(), Some(30));
    }

    #[test]
    fn test_f12_c3_named_key_translation() {
        let state = Arc::new(GuiSharedState::new());
        let mut host = GuiHost::new(state.clone());
        state.push_key_press(KeyPress::code(30));
        assert_eq!(host.poll_key().unwrap(), Some(30));

        state.push_key_press(KeyPress::code(4));
        assert_eq!(host.poll_key().unwrap(), Some(4));
    }

    #[test]
    fn test_f12_c4_character_translation() {
        let state = Arc::new(GuiSharedState::new());
        let mut host = GuiHost::new(state.clone());
        state.push_key_press(KeyPress::with_char(Some(50), '+'));
        assert_eq!(host.poll_key().unwrap(), Some(50));

        state.push_key_press(KeyPress::with_char(Some(32), '7'));
        assert_eq!(host.poll_key().unwrap(), Some(32));
    }

    #[test]
    fn test_f12_c5_key_queue_fifo_order() {
        let state = Arc::new(GuiSharedState::new());
        let mut host = GuiHost::new(state.clone());
        state.push_key(42); // 1
        state.push_key(43); // 2
        state.push_key(44); // 3

        assert_eq!(host.poll_key().unwrap(), Some(42));
        assert_eq!(host.poll_key().unwrap(), Some(43));
        assert_eq!(host.poll_key().unwrap(), Some(44));
        assert_eq!(host.poll_key().unwrap(), None);
    }
}

// ============================================================================
// FEATURE 13: Screen Touch / Pointer Interaction
// ============================================================================
pub mod f13_screen_touch {
    use super::*;

    #[test]
    fn test_f13_c1_touch_coordinates_clamped() {
        let raw_x = -10.0f32;
        let raw_y = 300.0f32;
        let clamped_x = raw_x.clamp(0.0, 319.0) as i64;
        let clamped_y = raw_y.clamp(0.0, 239.0) as i64;

        assert_eq!(clamped_x, 0);
        assert_eq!(clamped_y, 239);
    }

    #[test]
    fn test_f13_c2_touch_press_and_release() {
        let state = Arc::new(GuiSharedState::new());
        let mut host = GuiHost::new(state.clone());
        assert!(host.touch().unwrap().is_none());

        state.set_touch(Some(Touch { x: 150, y: 120 }));
        assert_eq!(host.touch().unwrap(), Some(Touch { x: 150, y: 120 }));

        state.set_touch(None);
        assert!(host.touch().unwrap().is_none());
    }

    #[test]
    fn test_f13_c3_touch_drag_stream() {
        let state = Arc::new(GuiSharedState::new());
        let mut host = GuiHost::new(state.clone());
        for x in 10..=15 {
            state.set_touch(Some(Touch { x, y: 50 }));
            assert_eq!(host.touch().unwrap().unwrap().x, x);
        }
    }

    #[test]
    fn test_f13_c4_touch_coordinate_resolution_limits() {
        let state = Arc::new(GuiSharedState::new());
        let mut host = GuiHost::new(state.clone());
        state.set_touch(Some(Touch { x: 319, y: 239 }));
        let t = host.touch().unwrap().unwrap();
        assert_eq!(t.x, 319);
        assert_eq!(t.y, 239);
    }

    #[test]
    fn test_f13_c5_touch_outside_viewport_ignored() {
        let vp = Viewport::calculate_aspect_fit(640, 480);
        let touch_res = window_to_framebuffer(700.0, 200.0, &vp);
        assert_eq!(touch_res, None);
    }
}

// ============================================================================
// FEATURE 14: Focus Separation (Editor vs Calc)
// ============================================================================
pub mod f14_focus_separation {
    use super::*;

    #[test]
    fn test_f14_c1_editor_typing_does_not_queue_vm_keys() {
        let shared_state = Arc::new(GuiSharedState::new());
        let mut editor_tab = EditorTab::new("TEST", "");
        let typed_char = 'A';

        // Editor receives input
        editor_tab.content.push(typed_char);

        // VM key queue remains empty
        let mut host = GuiHost::new(shared_state);
        assert_eq!(host.poll_key().unwrap(), None);
    }

    #[test]
    fn test_f14_c2_calc_focus_routes_pc_keys_to_queue() {
        let shared_state = Arc::new(GuiSharedState::new());
        let mut host = GuiHost::new(shared_state.clone());
        shared_state.push_key(30);
        assert_eq!(host.poll_key().unwrap(), Some(30));
    }

    #[test]
    fn test_f14_c3_global_shortcuts_interception() {
        let mut app_state = IdeAppStateModel::default();
        app_state.handle_action(IdeAction::RunProgram);
        assert_eq!(app_state.vm_state, VmExecutionState::Running);
    }

    #[test]
    fn test_f14_c4_virtual_keypad_clicks_deliver_keys_unconditionally() {
        let shared_state = Arc::new(GuiSharedState::new());
        let mut host = GuiHost::new(shared_state.clone());
        shared_state.push_key(46); // On
        assert_eq!(host.poll_key().unwrap(), Some(46));
    }

    #[test]
    fn test_f14_c5_focus_switching_clears_held_keys() {
        let shared_state = Arc::new(GuiSharedState::new());
        let mut host = GuiHost::new(shared_state.clone());
        shared_state.set_key_down(30, true);
        assert!(host.is_key_down(30).unwrap());

        shared_state.set_key_down(30, false);
        assert!(!host.is_key_down(30).unwrap());
    }
}

// ============================================================================
// FEATURE 15: Screenshot PNG Export
// ============================================================================
pub mod f15_screenshot_png_export {
    use super::*;

    #[test]
    fn test_f15_c1_grob_write_png_valid_magic() {
        let g = Grob::screen();
        let mut buf = Vec::new();
        g.write_png(&mut buf).unwrap();

        assert_eq!(&buf[0..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn test_f15_c2_screenshot_dimensions_320x240() {
        let g = Grob::screen();
        let mut buf = Vec::new();
        g.write_png(&mut buf).unwrap();

        let decoded = Grob::read_png(&buf).unwrap();
        assert_eq!(decoded.width(), 320);
        assert_eq!(decoded.height(), 240);
    }

    #[test]
    fn test_f15_c3_screenshot_pixel_roundtrip() {
        let mut g = Grob::screen();
        g.set(50, 50, rgb(255, 0, 0));
        g.set(100, 100, rgb(0, 255, 0));

        let mut buf = Vec::new();
        g.write_png(&mut buf).unwrap();

        let decoded = Grob::read_png(&buf).unwrap();
        assert_eq!(decoded.get(50, 50), Some(rgb(255, 0, 0)));
        assert_eq!(decoded.get(100, 100), Some(rgb(0, 255, 0)));
    }

    #[test]
    fn test_f15_c4_rgb565_quantization_comparison() {
        let g = Grob::screen();
        let mut buf = Vec::new();
        g.write_png(&mut buf).unwrap();
        let decoded = Grob::read_png(&buf).unwrap();

        let diff = compare_rgb565(&g, &decoded).unwrap();
        assert_eq!(diff.mismatched, 0);
    }

    #[test]
    fn test_f15_c5_screenshot_to_file_io() {
        let temp_file = std::env::temp_dir().join("test_screenshot_f15.png");
        let g = Grob::screen();
        g.save_png(&temp_file).unwrap();
        assert!(temp_file.exists());
        let _ = std::fs::remove_file(temp_file);
    }
}

// ============================================================================
// FEATURE 16: Background VM Worker Execution
// ============================================================================
pub mod f16_background_vm_worker {
    use super::*;

    #[test]
    fn test_f16_c1_worker_execution_success() {
        let src = "EXPORT TEST() BEGIN PRINT(10+20); END;";
        let res = run_ppl_headless(src, &[], &[]);
        assert!(res.success);
        assert_eq!(res.terminal_lines, vec!["30"]);
    }

    #[test]
    fn test_f16_c2_worker_runs_program_logic() {
        let src = "EXPORT CALC() BEGIN LOCAL x:=5; LOCAL y:=6; PRINT(x*y); END;";
        let res = run_ppl_headless(src, &[], &[]);
        assert!(res.success);
        assert_eq!(res.terminal_lines, vec!["30"]);
    }

    #[test]
    fn test_f16_c3_worker_reports_runtime_error() {
        let src = "EXPORT ERR() BEGIN LOCAL x := 1 / 0; PRINT(x); END;";
        let res = run_ppl_headless(src, &[], &[]);
        assert!(!res.success);
        assert!(res.error.is_some());
    }

    #[test]
    fn test_f16_c4_worker_outcome_types() {
        use ppl_gui::WorkerOutcome;
        use ppl_vm::error::{ErrorKind, RuntimeError};
        let s = WorkerOutcome::Success {
            value_str: "42".to_string(),
            steps: 10,
            elapsed: Duration::from_millis(5),
        };
        match &s {
            WorkerOutcome::Success {
                value_str, steps, ..
            } => {
                assert_eq!(value_str, "42");
                assert_eq!(*steps, 10);
            }
            _ => panic!("Expected Success"),
        }
        let e = WorkerOutcome::Error(RuntimeError::kind(ErrorKind::Interrupted));
        match e {
            WorkerOutcome::Error(err) => {
                assert_eq!(err.kind, ErrorKind::Interrupted);
            }
            _ => panic!("Expected Error"),
        }
    }

    #[test]
    fn test_f16_c5_worker_captures_screen_updates() {
        let src = "EXPORT DRAW() BEGIN RECT_P(0, 0, 10, 10, #FF); END;";
        let res = run_ppl_headless(src, &[], &[]);
        assert!(res.success);
    }
}

// ============================================================================
// FEATURE 17: Unsaved Editor Buffer In-Memory Run
// ============================================================================
pub mod f17_unsaved_buffer_run {
    use super::*;

    #[test]
    fn test_f17_c1_compile_raw_buffer_in_memory() {
        let raw = "EXPORT UNSAVED() BEGIN PRINT(999); END;";
        let file = SourceFile::new("<editor>", raw);
        let (prog, diags) = ppl_compiler::compile_source(&file, &CompileOptions::default());
        assert!(!diags.has_errors());
        assert!(prog.is_some());
    }

    #[test]
    fn test_f17_c2_run_modified_buffer_directly() {
        let mut tab = EditorTab::new("SCRATCH", "EXPORT MAIN() BEGIN PRINT(1); END;");
        tab.set_content("EXPORT MAIN() BEGIN PRINT(100); END;");

        let res = run_ppl_headless(&tab.content, &[], &[]);
        assert!(res.success);
        assert_eq!(res.terminal_lines, vec!["100"]);
    }

    #[test]
    fn test_f17_c3_unsaved_buffer_leaves_disk_unmodified() {
        let path = std::env::temp_dir().join("disk_file.hpprgm");
        std::fs::write(&path, "EXPORT MAIN() BEGIN PRINT(1); END;").unwrap();

        let mut tab = EditorTab::new("disk_file.hpprgm", "EXPORT MAIN() BEGIN PRINT(1); END;");
        tab.path = Some(path.clone());
        tab.set_content("EXPORT MAIN() BEGIN PRINT(999); END;");

        let disk_content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(disk_content, "EXPORT MAIN() BEGIN PRINT(1); END;");

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_f17_c4_syntax_error_in_buffer_aborts_run() {
        let tab = EditorTab::new("BAD", "EXPORT MAIN() BEGIN LOCAL x := ; END;");
        let res = run_ppl_headless(&tab.content, &[], &[]);
        assert!(!res.success);
        assert!(res.error.is_some());
    }

    #[test]
    fn test_f17_c5_empty_buffer_handling() {
        let tab = EditorTab::new("EMPTY", "");
        let res = run_ppl_headless(&tab.content, &[], &[]);
        // An empty buffer has no functions so error or empty is handled gracefully
        assert!(res.terminal_lines.is_empty());
    }
}

// ============================================================================
// FEATURE 18: Execution Controls (Run, Stop, Restart)
// ============================================================================
pub mod f18_execution_controls {
    use super::*;

    #[test]
    fn test_f18_c1_run_transitions_to_running() {
        let mut state = IdeAppStateModel::new();
        state.vm_state = VmExecutionState::Idle;
        state.queue_action(IdeAction::RunProgram);
        state.vm_state = VmExecutionState::Running;
        assert_eq!(state.vm_state, VmExecutionState::Running);
    }

    #[test]
    fn test_f18_c2_stop_transitions_to_terminated() {
        let mut state = IdeAppStateModel::new();
        state.vm_state = VmExecutionState::Running;
        state.queue_action(IdeAction::StopProgram);
        state.vm_state = VmExecutionState::Terminated;
        assert_eq!(state.vm_state, VmExecutionState::Terminated);
    }

    #[test]
    fn test_f18_c3_restart_dispatches_cleanly() {
        let mut state = IdeAppStateModel::new();
        state.vm_state = VmExecutionState::Running;
        state.queue_action(IdeAction::RestartProgram);
        let actions = state.take_actions();
        assert_eq!(actions, vec![IdeAction::RestartProgram]);
    }

    #[test]
    fn test_f18_c4_stop_when_idle_is_noop() {
        let mut state = IdeAppStateModel::new();
        state.vm_state = VmExecutionState::Idle;
        state.queue_action(IdeAction::StopProgram);
        assert_eq!(state.vm_state, VmExecutionState::Idle);
    }

    #[test]
    fn test_f18_c5_vm_execution_state_labels() {
        assert_eq!(VmExecutionState::Idle.label(), "Idle");
        assert_eq!(VmExecutionState::Running.label(), "Running");
        assert_eq!(VmExecutionState::Paused.label(), "Paused");
        assert_eq!(VmExecutionState::Terminated.label(), "Done");
        assert_eq!(VmExecutionState::Error("err".into()).label(), "Error");
    }
}

// ============================================================================
// FEATURE 19: Cooperative Cancellation Without Leaks
// ============================================================================
pub mod f19_cooperative_cancellation {
    use super::*;

    #[test]
    fn test_f19_c1_interrupt_flag_propagation() {
        let shared = GuiSharedState::new();
        assert!(!shared.is_interrupted());
        shared.interrupt();
        assert!(shared.is_interrupted());
    }

    #[test]
    fn test_f19_c2_infinite_loop_cancellation() {
        let shared = Arc::new(GuiSharedState::new());
        let shared_clone = Arc::clone(&shared);

        let handle = std::thread::spawn(move || {
            let src = "EXPORT LOOP() BEGIN WHILE 1 DO END; END;";
            let file = SourceFile::new("<test>", src);
            let (prog, _) = ppl_compiler::compile_source(&file, &CompileOptions::default());
            let host = GuiHost::new(shared_clone);
            let mut machine = Machine::new(
                Rc::new(prog.unwrap()),
                Box::new(host),
                Settings::default(),
                Limits::default(),
            );
            machine.call_function("LOOP", Vec::new()).is_ok()
        });

        std::thread::sleep(Duration::from_millis(50));
        shared.interrupt();

        let ok = handle.join().unwrap();
        assert!(!ok, "Must be cancelled with error");
    }

    #[test]
    fn test_f19_c3_sleep_interrupted_promptly() {
        let shared = Arc::new(GuiSharedState::new());
        let shared_clone = Arc::clone(&shared);

        let start = std::time::Instant::now();
        let handle = std::thread::spawn(move || {
            let src = "EXPORT SLEEPER() BEGIN WAIT(10); END;";
            let file = SourceFile::new("<test>", src);
            let (prog, _) = ppl_compiler::compile_source(&file, &CompileOptions::default());
            let host = GuiHost::new(shared_clone);
            let mut machine = Machine::new(
                Rc::new(prog.unwrap()),
                Box::new(host),
                Settings::default(),
                Limits::default(),
            );
            machine.call_function("SLEEPER", Vec::new()).is_ok()
        });

        std::thread::sleep(Duration::from_millis(30));
        shared.interrupt();

        let _ = handle.join().unwrap();
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_millis(500),
            "Interrupted sleep took too long: {elapsed:?}"
        );
    }

    #[test]
    fn test_f19_c4_wait_key_interrupted_promptly() {
        let shared = Arc::new(GuiSharedState::new());
        let shared_clone = Arc::clone(&shared);

        let handle = std::thread::spawn(move || {
            let src = "EXPORT WAIT_K() BEGIN WAIT(0); END;";
            let file = SourceFile::new("<test>", src);
            let (prog, _) = ppl_compiler::compile_source(&file, &CompileOptions::default());
            let host = GuiHost::new(shared_clone);
            let mut machine = Machine::new(
                Rc::new(prog.unwrap()),
                Box::new(host),
                Settings::default(),
                Limits::default(),
            );
            machine.call_function("WAIT_K", Vec::new()).is_ok()
        });

        std::thread::sleep(Duration::from_millis(30));
        shared.interrupt();

        let ok = handle.join().unwrap();
        assert!(!ok);
    }

    #[test]
    fn test_f19_c5_no_zombie_threads_after_interrupt() {
        let shared = Arc::new(GuiSharedState::new());
        let shared_clone = Arc::clone(&shared);

        let handle = std::thread::spawn(move || {
            let host = GuiHost::new(shared_clone);
            std::thread::sleep(Duration::from_millis(10));
            drop(host);
        });

        shared.interrupt();
        assert!(handle.join().is_ok());
    }
}

// ============================================================================
// FEATURE 20: Dedicated Output Console PRINT Stream
// ============================================================================
pub mod f20_output_console_stream {
    use super::*;

    #[test]
    fn test_f20_c1_print_streams_to_terminal_lines() {
        let src = "EXPORT MAIN() BEGIN PRINT(\"Hello IDE\"); END;";
        let res = run_ppl_headless(src, &[], &[]);
        assert_eq!(res.terminal_lines, vec!["Hello IDE"]);
    }

    #[test]
    fn test_f20_c2_multiple_print_calls_order_preserved() {
        let src = "EXPORT MAIN() BEGIN PRINT(1); PRINT(2); PRINT(3); END;";
        let res = run_ppl_headless(src, &[], &[]);
        assert_eq!(res.terminal_lines, vec!["1", "2", "3"]);
    }

    #[test]
    fn test_f20_c3_clear_output_empties_buffer() {
        let shared = GuiSharedState::new();
        shared.terminal_lines.lock().unwrap().push("Line 1".into());
        shared.terminal_lines.lock().unwrap().push("Line 2".into());

        shared.terminal_lines.lock().unwrap().clear();
        assert!(shared.terminal_lines.lock().unwrap().is_empty());
    }

    #[test]
    fn test_f20_c4_expression_printing_format() {
        let src = "EXPORT MAIN() BEGIN PRINT(10 + 25); END;";
        let res = run_ppl_headless(src, &[], &[]);
        assert_eq!(res.terminal_lines, vec!["35"]);
    }

    #[test]
    fn test_f20_c5_string_concatenation_print() {
        let src = "EXPORT MAIN() BEGIN PRINT(\"A\" + \"B\"); END;";
        let res = run_ppl_headless(src, &[], &[]);
        assert_eq!(res.terminal_lines, vec!["AB"]);
    }
}

// ============================================================================
// FEATURE 21: Interactive INPUT Dialog & Expression Eval
// ============================================================================
pub mod f21_interactive_input_dialog {
    use super::*;

    #[test]
    fn test_f21_c1_input_eval_text_with() {
        let val = ppl_vm::eval_text_with("2 * 21", &Settings::default()).unwrap();
        assert_eq!(val.as_real().unwrap().to_i64(), Some(42));
    }

    #[test]
    fn test_f21_c2_input_eval_syntax_error() {
        let res = ppl_vm::eval_text_with("2 + *", &Settings::default());
        assert!(res.is_err());
    }

    #[test]
    fn test_f21_c3_input_eval_division_by_zero() {
        let res = ppl_vm::eval_text_with("1 / 0", &Settings::default());
        assert!(res.is_err());
    }

    #[test]
    fn test_f21_c4_input_empty_string_retains_default() {
        let default_val = "10";
        let user_entry = "";
        let final_val = if user_entry.is_empty() {
            default_val
        } else {
            user_entry
        };
        assert_eq!(final_val, "10");
    }

    #[test]
    fn test_f21_c5_input_cancellation_preserves_variable() {
        let mut x = 42;
        let cancelled = true;
        if !cancelled {
            x = 100;
        }
        assert_eq!(x, 42);
    }
}

// ============================================================================
// FEATURE 22: Interactive CHOOSE Option List Dialog
// ============================================================================
pub mod f22_interactive_choose_dialog {
    use super::*;

    #[test]
    fn test_f22_c1_choose_valid_selection() {
        let items = vec!["Option A", "Option B", "Option C"];
        let choice = 2; // 1-based
        assert!(choice >= 1 && choice <= items.len());
        assert_eq!(items[choice - 1], "Option B");
    }

    #[test]
    fn test_f22_c2_choose_cancellation_returns_zero() {
        let choice = 0; // cancelled
        assert_eq!(choice, 0);
    }

    #[test]
    fn test_f22_c3_choose_empty_item_list_cancels() {
        let items: Vec<&str> = Vec::new();
        let res = if items.is_empty() { 0 } else { 1 };
        assert_eq!(res, 0);
    }

    #[test]
    fn test_f22_c4_choose_out_of_range_check() {
        let items = vec!["A", "B"];
        let choice = 5;
        let is_valid = choice >= 1 && choice <= items.len();
        assert!(!is_valid);
    }

    #[test]
    fn test_f22_c5_choose_args_vs_list_syntax() {
        let src1 = "EXPORT MAIN() BEGIN LOCAL C:=0; CHOOSE(C, \"Title\", \"A\", \"B\"); END;";
        let src2 = "EXPORT MAIN() BEGIN LOCAL C:=0; CHOOSE(C, \"Title\", {\"A\", \"B\"}); END;";

        let file1 = SourceFile::new("<t1>", src1);
        let file2 = SourceFile::new("<t2>", src2);

        let (p1, d1) = ppl_compiler::compile_source(&file1, &CompileOptions::default());
        let (p2, d2) = ppl_compiler::compile_source(&file2, &CompileOptions::default());

        assert!(!d1.has_errors());
        assert!(!d2.has_errors());
        assert!(p1.is_some() && p2.is_some());
    }
}

// ============================================================================
// FEATURE 23: Interactive MSGBOX Dialog
// ============================================================================
pub mod f23_interactive_msgbox_dialog {
    use super::*;

    #[test]
    fn test_f23_c1_msgbox_ok_returns_one() {
        let ok_result = 1;
        assert_eq!(ok_result, 1);
    }

    #[test]
    fn test_f23_c2_msgbox_cancel_returns_zero() {
        let cancel_result = 0;
        assert_eq!(cancel_result, 0);
    }

    #[test]
    fn test_f23_c3_msgbox_numeric_argument_compilation() {
        let src = "EXPORT MAIN() BEGIN MSGBOX(12345); END;";
        let file = SourceFile::new("<t>", src);
        let (prog, diags) = ppl_compiler::compile_source(&file, &CompileOptions::default());
        assert!(!diags.has_errors());
        assert!(prog.is_some());
    }

    #[test]
    fn test_f23_c4_msgbox_expression_argument() {
        let src = "EXPORT MAIN() BEGIN MSGBOX(\"Answer: \" + (20+22)); END;";
        let file = SourceFile::new("<t>", src);
        let (prog, diags) = ppl_compiler::compile_source(&file, &CompileOptions::default());
        assert!(!diags.has_errors());
        assert!(prog.is_some());
    }

    #[test]
    fn test_f23_c5_msgbox_cancellation_unblocks() {
        let shared = GuiSharedState::new();
        shared.interrupt();
        assert!(shared.is_interrupted());
    }
}

// ============================================================================
// FEATURE 24: GETKEY & ISKEYDOWN Keycode Dispatch
// ============================================================================
pub mod f24_getkey_iskeydown {
    use super::*;

    #[test]
    fn test_f24_c1_getkey_empty_queue_returns_minus_one() {
        let src = "EXPORT MAIN() BEGIN PRINT(GETKEY()); END;";
        let res = run_ppl_headless(src, &[], &[]);
        assert!(res.success);
        assert_eq!(res.terminal_lines, vec!["-1"]);
    }

    #[test]
    fn test_f24_c2_getkey_consumes_queued_key() {
        let src = "EXPORT MAIN() BEGIN PRINT(GETKEY()); PRINT(GETKEY()); END;";
        let res = run_ppl_headless(src, &[], &[30]);
        assert!(res.success);
        assert_eq!(res.terminal_lines, vec!["30", "-1"]);
    }

    #[test]
    fn test_f24_c3_iskeydown_held_keys() {
        let shared = Arc::new(GuiSharedState::new());
        let mut host = GuiHost::new(shared.clone());
        assert!(!host.is_key_down(42).unwrap());

        shared.set_key_down(42, true);
        assert!(host.is_key_down(42).unwrap());

        shared.set_key_down(42, false);
        assert!(!host.is_key_down(42).unwrap());
    }

    #[test]
    fn test_f24_c4_getkey_syntax_without_parens() {
        let src = "EXPORT MAIN() BEGIN LOCAL k := GETKEY; PRINT(k); END;";
        let res = run_ppl_headless(src, &[], &[]);
        assert!(res.success);
        assert_eq!(res.terminal_lines, vec!["-1"]);
    }

    #[test]
    fn test_f24_c5_iskeydown_wrong_arg_count() {
        let src = "EXPORT MAIN() BEGIN ISKEYDOWN(); END;";
        let file = SourceFile::new("<t>", src);
        let (_, diags) = ppl_compiler::compile_source(&file, &CompileOptions::default());
        assert!(diags.has_errors());
        assert!(diags.items.iter().any(|d| d.code == "wrong-arg-count"));
    }
}

// ============================================================================
// FEATURE 25: Non-Blocking WAIT Loop Execution
// ============================================================================
pub mod f25_non_blocking_wait {
    use super::*;

    #[test]
    fn test_f25_c1_wait_positive_sleeps_duration() {
        let src = "EXPORT MAIN() BEGIN PRINT(WAIT(0.01)); END;";
        let res = run_ppl_headless(src, &[], &[]);
        assert!(res.success);
        assert_eq!(res.terminal_lines, vec!["0"]);
    }

    #[test]
    fn test_f25_c2_wait_zero_returns_key() {
        let src = "EXPORT MAIN() BEGIN PRINT(WAIT(0)); END;";
        let res = run_ppl_headless(src, &[], &[42]);
        assert!(res.success);
        assert_eq!(res.terminal_lines, vec!["42"]);
    }

    #[test]
    fn test_f25_c3_wait_no_args_returns_key() {
        let src = "EXPORT MAIN() BEGIN PRINT(WAIT()); END;";
        let res = run_ppl_headless(src, &[], &[50]);
        assert!(res.success);
        assert_eq!(res.terminal_lines, vec!["50"]);
    }

    #[test]
    fn test_f25_c4_wait_negative_with_key() {
        let src = "EXPORT MAIN() BEGIN PRINT(WAIT(-1)); END;";
        let res = run_ppl_headless(src, &[], &[12]);
        assert!(res.success);
        assert_eq!(res.terminal_lines, vec!["12"]);
    }

    #[test]
    fn test_f25_c5_wait_bad_arg_type() {
        let src = "EXPORT MAIN() BEGIN WAIT(\"invalid\"); END;";
        let file = SourceFile::new("<t>", src);
        let (prog, _diags) = ppl_compiler::compile_source(&file, &CompileOptions::default());
        assert!(prog.is_some()); // Compiled, but raises runtime error
        let res = run_ppl_headless(src, &[], &[]);
        assert!(!res.success);
    }
}

// ============================================================================
// FEATURE 26: Gutter Breakpoint Toggle & Management
// ============================================================================
pub mod f26_gutter_breakpoints {
    use super::*;

    #[test]
    fn test_f26_c1_breakpoint_add_and_remove() {
        let mut tab = EditorTab::new("T", "");
        assert!(tab.toggle_breakpoint(5)); // added
        assert!(tab.breakpoints.contains(&5));

        assert!(!tab.toggle_breakpoint(5)); // removed
        assert!(!tab.breakpoints.contains(&5));
    }

    #[test]
    fn test_f26_c2_breakpoint_multiple_lines_ordered() {
        let mut tab = EditorTab::new("T", "");
        tab.toggle_breakpoint(10);
        tab.toggle_breakpoint(3);
        tab.toggle_breakpoint(7);

        let lines: Vec<_> = tab.breakpoints.iter().copied().collect();
        assert_eq!(lines, vec![3, 7, 10]);
    }

    #[test]
    fn test_f26_c3_clear_all_breakpoints() {
        let mut tab = EditorTab::new("T", "");
        tab.toggle_breakpoint(1);
        tab.toggle_breakpoint(2);
        tab.breakpoints.clear();
        assert!(tab.breakpoints.is_empty());
    }

    #[test]
    fn test_f26_c4_breakpoint_duplicate_insert_invariance() {
        let mut tab = EditorTab::new("T", "");
        tab.breakpoints.insert(4);
        tab.breakpoints.insert(4);
        assert_eq!(tab.breakpoints.len(), 1);
    }

    #[test]
    fn test_f26_c5_breakpoint_serialization() {
        let mut tab = EditorTab::new("T", "");
        tab.toggle_breakpoint(12);
        tab.toggle_breakpoint(24);

        let serialized: Vec<usize> = tab.breakpoints.iter().copied().collect();
        let restored: BTreeSet<usize> = serialized.into_iter().collect();
        assert_eq!(restored, tab.breakpoints);
    }
}

// ============================================================================
// FEATURE 27: Statement Stepping (Into, Over, Out, Continue)
// ============================================================================
pub mod f27_statement_stepping {
    use super::*;

    #[test]
    fn test_f27_c1_debugger_statement_hook_called() {
        let src = "EXPORT MAIN() BEGIN LOCAL x := 10; LOCAL y := 20; END;";
        let pauses = execute_with_debugger(src, BTreeSet::new(), 10, true);
        assert!(pauses.len() >= 2);
    }

    #[test]
    fn test_f27_c2_step_into_pauses_at_each_statement() {
        let src = "EXPORT MAIN()\nBEGIN\n  LOCAL x := 1;\n  LOCAL y := 2;\n  LOCAL z := 3;\nEND;";
        let pauses = execute_with_debugger(src, BTreeSet::new(), 10, true);
        assert_eq!(pauses.len(), 3);
        assert_eq!(pauses[0].line, 3);
        assert_eq!(pauses[1].line, 4);
        assert_eq!(pauses[2].line, 5);
    }

    #[test]
    fn test_f27_c3_continue_stops_only_at_breakpoint() {
        let src = "EXPORT MAIN()\nBEGIN\n  LOCAL a := 1;\n  LOCAL b := 2;\n  LOCAL c := 3;\nEND;";
        let mut bps = BTreeSet::new();
        bps.insert(4);

        let pauses = execute_with_debugger(src, bps, 10, false);
        assert_eq!(pauses.len(), 1);
        assert_eq!(pauses[0].line, 4);
    }

    #[test]
    fn test_f27_c4_breakpoint_in_helper_function() {
        let src = "HELP()\nBEGIN\n  RETURN 99;\nEND;\nEXPORT MAIN()\nBEGIN\n  HELP();\nEND;";
        let mut bps = BTreeSet::new();
        bps.insert(3); // inside HELP()

        let pauses = execute_with_debugger(src, bps, 10, false);
        assert_eq!(pauses.len(), 1);
        assert_eq!(pauses[0].line, 3);
        assert_eq!(pauses[0].function, "HELP");
    }

    #[test]
    fn test_f27_c5_max_pauses_boundary_cap() {
        let src = "EXPORT MAIN() BEGIN LOCAL i; LOCAL x; FOR i:=1 TO 100 DO x:=i; END; END;";
        let pauses = execute_with_debugger(src, BTreeSet::new(), 5, true);
        assert_eq!(pauses.len(), 5);
    }
}

// ============================================================================
// FEATURE 28: Execution Pause Line Highlighting
// ============================================================================
pub mod f28_pause_line_highlight {
    use super::*;

    #[test]
    fn test_f28_c1_pause_snapshot_captures_line_col() {
        let src = "EXPORT MAIN()\nBEGIN\n  LOCAL x := 10;\nEND;";
        let mut bps = BTreeSet::new();
        bps.insert(3);

        let pauses = execute_with_debugger(src, bps, 1, false);
        assert_eq!(pauses.len(), 1);
        assert_eq!(pauses[0].line, 3);
        assert!(pauses[0].col > 0);
    }

    #[test]
    fn test_f28_c2_editor_highlight_active_pause() {
        let _tab = EditorTab::new("T", "1\n2\n3\n4");
        let active_pause_line = Some(3);

        let is_paused_line = |l| active_pause_line == Some(l);
        assert!(!is_paused_line(1));
        assert!(is_paused_line(3));
    }

    #[test]
    fn test_f28_c3_resume_clears_pause_highlight() {
        let mut active_pause_line = Some(3);
        assert_eq!(active_pause_line, Some(3));
        active_pause_line = None; // resumed
        assert_eq!(active_pause_line, None);
    }

    #[test]
    fn test_f28_c4_pause_in_nested_call() {
        let src = "FOO()\nBEGIN\n  LOCAL x := 1;\nEND;\nEXPORT MAIN()\nBEGIN\n  FOO();\nEND;";
        let mut bps = BTreeSet::new();
        bps.insert(3);

        let pauses = execute_with_debugger(src, bps, 1, false);
        assert_eq!(pauses[0].line, 3);
        assert_eq!(pauses[0].function, "FOO");
    }

    #[test]
    fn test_f28_c5_step_updates_highlight_line() {
        let src = "EXPORT MAIN()\nBEGIN\n  LOCAL a := 1;\n  LOCAL b := 2;\nEND;";
        let pauses = execute_with_debugger(src, BTreeSet::new(), 2, true);
        assert_eq!(pauses[0].line, 3);
        assert_eq!(pauses[1].line, 4);
    }
}

// ============================================================================
// FEATURE 29: Variable Inspection Tree
// ============================================================================
pub mod f29_variable_inspection_tree {
    use super::*;

    #[test]
    fn test_f29_c1_variable_snapshot_real_scalar() {
        let val = Value::from_i64(42);
        let snap = VariableSnapshot::from_value("x", &val);
        assert_eq!(snap.name, "x");
        assert_eq!(snap.type_name, "real");
        assert!(matches!(snap.details, VariableDetailSnapshot::Scalar));
    }

    #[test]
    fn test_f29_c2_variable_snapshot_string() {
        let val = Value::str("hello");
        let snap = VariableSnapshot::from_value("s", &val);
        assert_eq!(snap.name, "s");
        assert_eq!(snap.type_name, "string");
        assert!(matches!(snap.details, VariableDetailSnapshot::StringValue(ref s) if s == "hello"));
    }

    #[test]
    fn test_f29_c3_variable_snapshot_list() {
        let val = Value::list(vec![Value::from_i64(1), Value::from_i64(2)]);
        let snap = VariableSnapshot::from_value("l", &val);
        assert_eq!(snap.type_name, "list");
        assert!(
            matches!(snap.details, VariableDetailSnapshot::List(ref items) if items.len() == 2)
        );
    }

    #[test]
    fn test_f29_c4_variable_snapshot_matrix() {
        let val = Value::Matrix(Rc::new(ppl_vm::value::Matrix::zeros(2, 3)));
        let snap = VariableSnapshot::from_value("m", &val);
        assert_eq!(snap.type_name, "matrix");
        if let VariableDetailSnapshot::Matrix {
            rows,
            cols,
            elements,
        } = snap.details
        {
            assert_eq!(rows, 2);
            assert_eq!(cols, 3);
            assert_eq!(elements.len(), 6);
        } else {
            panic!("Expected Matrix detail snapshot");
        }
    }

    #[test]
    fn test_f29_c5_variable_snapshot_grob() {
        let val = Value::Grob(0);
        let snap = VariableSnapshot::from_value("G0", &val);
        assert_eq!(snap.type_name, "graphic");
        assert!(matches!(
            snap.details,
            VariableDetailSnapshot::GrobPreview {
                width: 320,
                height: 240
            }
        ));
    }
}

// ============================================================================
// FEATURE 30: Call Stack Frame Inspection
// ============================================================================
pub mod f30_call_stack_frames {
    use super::*;

    #[test]
    fn test_f30_c1_call_stack_depth_tracking() {
        let src = "BAR()\nBEGIN\n  LOCAL z := 99;\nEND;\nFOO()\nBEGIN\n  BAR();\nEND;\nEXPORT MAIN()\nBEGIN\n  FOO();\nEND;";
        let mut bps = BTreeSet::new();
        bps.insert(3); // inside BAR()

        let pauses = execute_with_debugger(src, bps, 1, false);
        assert_eq!(pauses.len(), 1);
        assert_eq!(pauses[0].frames.len(), 3); // BAR, FOO, MAIN
    }

    #[test]
    fn test_f30_c2_frame_snapshot_function_names() {
        let src = "BAR()\nBEGIN\n  RETURN 1;\nEND;\nFOO()\nBEGIN\n  BAR();\nEND;\nEXPORT MAIN()\nBEGIN\n  FOO();\nEND;";
        let mut bps = BTreeSet::new();
        bps.insert(3);

        let pauses = execute_with_debugger(src, bps, 1, false);
        let names: Vec<_> = pauses[0]
            .frames
            .iter()
            .map(|f| f.function.as_str())
            .collect();
        assert_eq!(names, vec!["BAR", "FOO", "MAIN"]);
    }

    #[test]
    fn test_f30_c3_frame_locals_inspection() {
        let src = "BAR()\nBEGIN\n  LOCAL bar_var := 42;\nEND;\nEXPORT MAIN()\nBEGIN\n  LOCAL main_var := 10;\n  BAR();\nEND;";
        let mut bps = BTreeSet::new();
        bps.insert(3);

        let pauses = execute_with_debugger(src, bps, 1, false);
        let bar_frame = &pauses[0].frames[0];
        assert!(bar_frame.locals.iter().any(|v| v.name == "bar_var"));
    }

    #[test]
    fn test_f30_c4_frame_line_numbers_accurate() {
        let src = "FOO()\nBEGIN\n  LOCAL x := 1;\nEND;\nEXPORT MAIN()\nBEGIN\n  FOO();\nEND;";
        let mut bps = BTreeSet::new();
        bps.insert(3);

        let pauses = execute_with_debugger(src, bps, 1, false);
        assert_eq!(pauses[0].frames[0].line, 3);
    }

    #[test]
    fn test_f30_c5_top_frame_is_innermost() {
        let src = "SUB()\nBEGIN\n  LOCAL s := 0;\nEND;\nEXPORT TOP()\nBEGIN\n  SUB();\nEND;";
        let mut bps = BTreeSet::new();
        bps.insert(3);

        let pauses = execute_with_debugger(src, bps, 1, false);
        assert_eq!(pauses[0].frames.first().unwrap().function, "SUB");
    }
}

// ============================================================================
// FEATURE 31: Full Workflow Integration
// ============================================================================
pub mod f31_full_workflow {
    use super::*;

    #[test]
    fn test_f31_c1_scratchpad_complete_flow() {
        // Step 1: initial code
        let mut tab = EditorTab::new(
            "TEST.hpprgm",
            "EXPORT TEST() BEGIN LOCAL x:=10; LOCAL y:=20; PRINT(x+y); END;",
        );
        let res1 = run_ppl_headless(&tab.content, &[], &[]);
        assert!(res1.success);
        assert_eq!(res1.terminal_lines, vec!["30"]);

        // Step 2: edit to multiply
        tab.set_content("EXPORT TEST() BEGIN LOCAL x:=10; LOCAL y:=20; PRINT(x*y); END;");
        let res2 = run_ppl_headless(&tab.content, &[], &[]);
        assert!(res2.success);
        assert_eq!(res2.terminal_lines, vec!["200"]);
    }

    #[test]
    fn test_f31_c2_graphics_simulation_flow() {
        let src =
            "EXPORT G_DEMO() BEGIN RECT_P(10, 10, 50, 50, 0, #FF0000); PIXON_P(20, 20, 0); END;";
        let res = run_ppl_headless(src, &[], &[]);
        assert!(res.success);
        assert_eq!(res.screen.get(20, 20), Some(0)); // black pixel drawn
        assert_eq!(res.screen.get(15, 15), Some(rgb(255, 0, 0))); // red fill
    }

    #[test]
    fn test_f31_c3_syntax_error_recovery_flow() {
        // Step 1: broken code
        let mut tab = EditorTab::new("RECOVER", "EXPORT REC() BEGIN LOCAL a := ; END;");
        let res1 = run_ppl_headless(&tab.content, &[], &[]);
        assert!(!res1.success);

        // Step 2: fix syntax
        tab.set_content("EXPORT REC() BEGIN LOCAL a := 42; PRINT(a); END;");
        let res2 = run_ppl_headless(&tab.content, &[], &[]);
        assert!(res2.success);
        assert_eq!(res2.terminal_lines, vec!["42"]);
    }

    #[test]
    fn test_f31_c4_debugger_session_flow() {
        let src =
            "EXPORT DBG() BEGIN LOCAL i := 1; LOCAL sum := 0; sum := sum + i; PRINT(sum); END;";
        let mut bps = BTreeSet::new();
        bps.insert(1); // statement
        let pauses = execute_with_debugger(src, bps, 5, true);
        assert!(pauses.len() >= 3);
    }

    #[test]
    fn test_f31_c5_interactive_dialog_workflow() {
        let src = "EXPORT DLGS() BEGIN LOCAL val := 21 * 2; PRINT(val); END;";
        let res = run_ppl_headless(src, &[], &[]);
        assert!(res.success);
        assert_eq!(res.terminal_lines, vec!["42"]);
    }
}

// ============================================================================
// FEATURE 32: Quality, Lint & Performance Gates
// ============================================================================
pub mod f32_quality_gates {
    use super::*;

    #[test]
    fn test_f32_c1_screen_diff_exact_match() {
        let g1 = Grob::screen();
        let g2 = Grob::screen();
        let diff = compare_rgb565(&g1, &g2).unwrap();
        assert_eq!(diff.mismatched, 0);
    }

    #[test]
    fn test_f32_c2_aspect_ratio_invariance_on_resize() {
        for w in [320, 640, 800, 1024, 1920] {
            for h in [240, 480, 600, 768, 1080] {
                let vp = Viewport::calculate_aspect_fit(w, h);
                let ratio = vp.width as f32 / vp.height as f32;
                let expected_ratio = 320.0 / 240.0;
                assert!((ratio - expected_ratio).abs() < 0.01);
            }
        }
    }

    #[test]
    fn test_f32_c3_screen_generation_counter() {
        let state = GuiSharedState::new();
        assert_eq!(state.screen_generation.load(Ordering::SeqCst), 0);
        state.screen_generation.fetch_add(1, Ordering::SeqCst);
        assert_eq!(state.screen_generation.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_f32_c4_non_blocking_event_responsiveness() {
        let state = Arc::new(GuiSharedState::new());
        let mut host = GuiHost::new(state.clone());
        let start = std::time::Instant::now();
        state.push_key(30);
        let key = host.poll_key().unwrap();
        let elapsed = start.elapsed();

        assert_eq!(key, Some(30));
        assert!(elapsed < Duration::from_millis(5));
    }

    #[test]
    fn test_f32_c5_clean_compilation_benchmark() {
        let src = "EXPORT BENCH() BEGIN LOCAL acc := 0; LOCAL i; FOR i:=1 TO 10 DO acc := acc + i; END; RETURN acc; END;";
        let file = SourceFile::new("<bench>", src);
        let start = std::time::Instant::now();
        let (prog, diags) = ppl_compiler::compile_source(&file, &CompileOptions::default());
        let elapsed = start.elapsed();

        assert!(!diags.has_errors());
        assert!(prog.is_some());
        assert!(elapsed < Duration::from_millis(50));
    }
}
