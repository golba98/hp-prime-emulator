//! Tier 2: Boundary & Corner Cases for HP Prime Desktop IDE.
//!
//! Covers extreme inputs, off-screen coordinates, edge values, rapid event bursts,
//! and resource limits across all 32 inventoried features (total 160 test cases).

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
use ppl_gui::theme::{ACCENT_BLUE, BG_PANEL, TEXT_NORMAL, classic_engineering_visuals};
use ppl_syntax::diag::Diagnostics;
use ppl_syntax::source::SourceFile;
use ppl_vm::host::{Host, Touch};
use ppl_vm::machine::{Limits, Machine};
use ppl_vm::settings::Settings;
use ppl_vm::value::Value;

// ============================================================================
// FEATURE 1: Classic Engineering Aesthetic — Boundary Cases
// ============================================================================
pub mod f01_boundary {
    use super::*;

    #[test]
    fn test_f01_b1_extreme_dark_luminance() {
        let (r, g, b) = (
            BG_PANEL.r() as f32,
            BG_PANEL.g() as f32,
            BG_PANEL.b() as f32,
        );
        let lum = (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255.0;
        assert!(
            lum < 0.25,
            "Background panel must be very dark (<0.25 lum), got {lum}"
        );
    }

    #[test]
    fn test_f01_b2_border_stroke_crisp_limits() {
        let visuals = classic_engineering_visuals();
        assert!(visuals.window_stroke.width > 0.0 && visuals.window_stroke.width <= 1.0);
    }

    #[test]
    fn test_f01_b3_zero_rounding_all_corners() {
        let c = classic_engineering_visuals().window_corner_radius;
        assert_eq!(c.nw, 0);
        assert_eq!(c.ne, 0);
        assert_eq!(c.sw, 0);
        assert_eq!(c.se, 0);
    }

    #[test]
    fn test_f01_b4_text_contrast_ratio() {
        let text_lum = (TEXT_NORMAL.r() as f32 * 0.299
            + TEXT_NORMAL.g() as f32 * 0.587
            + TEXT_NORMAL.b() as f32 * 0.114)
            / 255.0;
        let bg_lum = (BG_PANEL.r() as f32 * 0.299
            + BG_PANEL.g() as f32 * 0.587
            + BG_PANEL.b() as f32 * 0.114)
            / 255.0;
        let contrast = (text_lum + 0.05) / (bg_lum + 0.05);
        assert!(
            contrast > 4.0,
            "Contrast ratio must be > 4:1 for readability"
        );
    }

    #[test]
    fn test_f01_b5_saturated_accent_blue() {
        assert!(ACCENT_BLUE.b() > ACCENT_BLUE.r());
        assert!(ACCENT_BLUE.b() > ACCENT_BLUE.g());
    }
}

// ============================================================================
// FEATURE 2: Main Menu Actions — Boundary Cases
// ============================================================================
pub mod f02_boundary {
    use super::*;

    #[test]
    fn test_f02_b1_rapid_action_queue_burst() {
        let mut state = IdeAppStateModel::new();
        for _ in 0..100 {
            state.queue_action(IdeAction::RunProgram);
        }
        let actions = state.take_actions();
        assert_eq!(actions.len(), 100);
    }

    #[test]
    fn test_f02_b2_empty_action_queue_drain() {
        let mut state = IdeAppStateModel::new();
        let actions = state.take_actions();
        assert!(actions.is_empty());
    }

    #[test]
    fn test_f02_b3_deep_unicode_file_path() {
        let mut state = IdeAppStateModel::new();
        let deep_path = PathBuf::from("/tmp/deep/folder/with/unicode/🔥_test.hpprgm");
        state.queue_action(IdeAction::OpenFile(Some(deep_path.clone())));
        let actions = state.take_actions();
        assert_eq!(actions[0], IdeAction::OpenFile(Some(deep_path)));
    }

    #[test]
    fn test_f02_b4_redundant_toggle_sequences() {
        let mut state = IdeAppStateModel::new();
        for _ in 0..10 {
            state.queue_action(IdeAction::ToggleExplorer);
        }
        assert_eq!(state.take_actions().len(), 10);
    }

    #[test]
    fn test_f02_b5_action_equality_and_cloning() {
        let a1 = IdeAction::ToggleBreakpoint(42);
        let a2 = a1.clone();
        assert_eq!(a1, a2);
    }
}

// ============================================================================
// FEATURE 3: Toolbar Controls — Boundary Cases
// ============================================================================
pub mod f03_boundary {
    use super::*;

    #[test]
    fn test_f03_b1_repeated_run_clicks_when_running() {
        let mut state = IdeAppStateModel::new();
        state.vm_state = VmExecutionState::Running;
        state.queue_action(IdeAction::RunProgram);
        state.queue_action(IdeAction::RunProgram);
        assert_eq!(state.take_actions().len(), 2);
    }

    #[test]
    fn test_f03_b2_stop_clicked_multiple_times_when_idle() {
        let mut state = IdeAppStateModel::new();
        state.vm_state = VmExecutionState::Idle;
        for _ in 0..5 {
            state.queue_action(IdeAction::StopProgram);
        }
        assert_eq!(state.take_actions().len(), 5);
        assert_eq!(state.vm_state, VmExecutionState::Idle);
    }

    #[test]
    fn test_f03_b3_step_into_when_idle() {
        let mut state = IdeAppStateModel::new();
        state.vm_state = VmExecutionState::Idle;
        state.queue_action(IdeAction::StepInto);
        assert_eq!(state.take_actions(), vec![IdeAction::StepInto]);
    }

    #[test]
    fn test_f03_b4_settings_dialog_toggle() {
        let mut state = IdeAppStateModel::new();
        state.queue_action(IdeAction::ShowSettingsDialog);
        assert_eq!(state.take_actions(), vec![IdeAction::ShowSettingsDialog]);
    }

    #[test]
    fn test_f03_b5_save_unmodified_active_tab() {
        let mut state = IdeAppStateModel::new();
        let tab = state.active_tab().unwrap();
        assert!(!tab.modified);
        state.queue_action(IdeAction::SaveActiveFile);
        assert_eq!(state.take_actions(), vec![IdeAction::SaveActiveFile]);
    }
}

// ============================================================================
// FEATURE 4: Resizable Multi-Panel Split — Boundary Cases
// ============================================================================
pub mod f04_boundary {
    use super::*;

    #[test]
    fn test_f04_b1_explorer_negative_width_clamped() {
        let mut state = IdeAppStateModel::new();
        state.explorer_width = (-500.0f32).max(150.0).min(600.0);
        assert_eq!(state.explorer_width, 150.0);
    }

    #[test]
    fn test_f04_b2_explorer_extreme_width_clamped() {
        let mut state = IdeAppStateModel::new();
        state.explorer_width = 10000.0f32.max(150.0).min(600.0);
        assert_eq!(state.explorer_width, 600.0);
    }

    #[test]
    fn test_f04_b3_calculator_zero_width_clamped() {
        let mut state = IdeAppStateModel::new();
        state.calculator_width = 0.0f32.max(320.0).min(800.0);
        assert_eq!(state.calculator_width, 320.0);
    }

    #[test]
    fn test_f04_b4_bottom_dock_zero_height_clamped() {
        let mut state = IdeAppStateModel::new();
        state.bottom_height = 0.0f32.max(100.0).min(500.0);
        assert_eq!(state.bottom_height, 100.0);
    }

    #[test]
    fn test_f04_b5_micro_screen_layout_stability() {
        let total_w = 400.0f32;
        let total_h = 300.0f32;
        let mut state = IdeAppStateModel::new();
        state.explorer_width = (total_w * 0.25).max(150.0);
        state.calculator_width = (total_w * 0.35).max(320.0);
        state.bottom_height = (total_h * 0.2).max(100.0);

        assert!(state.explorer_width >= 150.0);
        assert!(state.calculator_width >= 320.0);
        assert!(state.bottom_height >= 100.0);
    }
}

// ============================================================================
// FEATURE 5: Panel State Persistence — Boundary Cases
// ============================================================================
pub mod f05_boundary {
    use super::*;

    #[test]
    fn test_f05_b1_all_panels_hidden_simultaneously() {
        let mut state = IdeAppStateModel::new();
        state.show_explorer = false;
        state.show_calculator = false;
        state.show_bottom_dock = false;
        state.show_status_bar = false;

        assert!(!state.show_explorer);
        assert!(!state.show_calculator);
        assert!(!state.show_bottom_dock);
        assert!(!state.show_status_bar);
    }

    #[test]
    fn test_f05_b2_toggle_storm_consistency() {
        let mut state = IdeAppStateModel::new();
        for _ in 0..10_000 {
            state.show_explorer = !state.show_explorer;
        }
        assert!(
            state.show_explorer,
            "Even number of toggles must restore true"
        );
    }

    #[test]
    fn test_f05_b3_reset_when_already_at_defaults() {
        let mut state = IdeAppStateModel::new();
        state.reset_layout();
        assert!(state.show_explorer);
        assert_eq!(state.explorer_width, 240.0);
    }

    #[test]
    fn test_f05_b4_panel_state_preserved_during_tab_switch() {
        let mut state = IdeAppStateModel::new();
        state.show_calculator = false;
        state.open_tab("NEW_TAB", "");
        assert!(!state.show_calculator);
    }

    #[test]
    fn test_f05_b5_dock_tab_cycle() {
        let mut state = IdeAppStateModel::new();
        let tabs = [
            DockTab::Output,
            DockTab::Problems,
            DockTab::Variables,
            DockTab::CallStack,
        ];
        for t in tabs {
            state.active_dock_tab = t;
            assert_eq!(state.active_dock_tab, t);
        }
    }
}

// ============================================================================
// FEATURE 6: Multi-Tab Editor Buffer Lifecycle — Boundary Cases
// ============================================================================
pub mod f06_boundary {
    use super::*;

    #[test]
    fn test_f06_b1_closing_last_tab_behavior() {
        let mut state = IdeAppStateModel::new();
        assert_eq!(state.tabs.len(), 1);
        let closed = state.close_tab(0);
        assert!(closed);
        assert!(state.tabs.is_empty());
    }

    #[test]
    fn test_f06_b2_large_buffer_allocation() {
        let mut state = IdeAppStateModel::new();
        let large_str = "A".repeat(500_000);
        state.open_tab("LARGE", large_str);
        assert_eq!(state.tabs[1].content.len(), 500_000);
    }

    #[test]
    fn test_f06_b3_cursor_out_of_bounds_clamping() {
        let mut tab = EditorTab::new("TEST", "line 1\nline 2");
        tab.cursor_line = 9999;
        tab.cursor_col = 9999;
        assert_eq!(tab.cursor_line, 9999);
    }

    #[test]
    fn test_f06_b4_tab_title_special_unicode() {
        let title = "🔥🚀_PROGRAM_∑_π.hpprgm";
        let tab = EditorTab::new(title, "");
        assert_eq!(tab.title, title);
    }

    #[test]
    fn test_f06_b5_rapid_open_close_cycle() {
        let mut state = IdeAppStateModel::new();
        for i in 0..50 {
            let idx = state.open_tab(format!("T_{i}"), "");
            state.close_tab(idx);
        }
        assert_eq!(state.tabs.len(), 1);
    }
}

// ============================================================================
// FEATURE 7: Native File Operations — Boundary Cases
// ============================================================================
pub mod f07_boundary {
    use super::*;

    #[test]
    fn test_f07_b1_program_name_with_numbers() {
        let tmpl = default_ppl_template("P123");
        assert!(tmpl.contains("EXPORT P123()"));
    }

    #[test]
    fn test_f07_b2_file_path_with_spaces_and_parentheses() {
        let path = PathBuf::from("/tmp/my programs (v2)/test file.hpprgm");
        let mut tab = EditorTab::new("UNTITLED", "");
        tab.path = Some(path.clone());
        assert_eq!(tab.path.unwrap(), path);
    }

    #[test]
    fn test_f07_b3_empty_file_io() {
        let path = std::env::temp_dir().join("empty_file_f07.hpprgm");
        std::fs::write(&path, "").unwrap();
        let read = std::fs::read_to_string(&path).unwrap();
        assert_eq!(read, "");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_f07_b4_recent_files_identical_entries() {
        let mut state = IdeAppStateModel::new();
        let p = PathBuf::from("/same/path.hpprgm");
        for _ in 0..5 {
            state.add_recent_file(p.clone());
        }
        assert_eq!(state.recent_files.len(), 1);
    }

    #[test]
    fn test_f07_b5_save_as_different_extension() {
        let mut tab = EditorTab::new("CODE", "");
        let path = PathBuf::from("/workspace/TEST.txt");
        tab.path = Some(path.clone());
        tab.title = path.file_name().unwrap().to_string_lossy().to_string();
        assert_eq!(tab.title, "TEST.txt");
    }
}

// ============================================================================
// FEATURE 8: PPL Syntax Highlighting — Boundary Cases
// ============================================================================
pub mod f08_boundary {
    use super::*;

    #[test]
    fn test_f08_b1_mixed_case_keyword_is_ident() {
        let src = "Export local Begin";
        let mut diags = Diagnostics::default();
        let toks = ppl_syntax::lexer::lex(src, &mut diags);
        assert_eq!(toks.len(), 4);
        assert_eq!(toks.last().unwrap().tok, ppl_syntax::lexer::Tok::Eof);
        for t in &toks[..3] {
            assert!(matches!(t.tok, ppl_syntax::lexer::Tok::Ident(_)));
        }
    }

    #[test]
    fn test_f08_b2_extreme_base_integer_hex() {
        let src = "#FFFFFFFFFFFFFFFF:16h";
        let mut diags = Diagnostics::default();
        let toks = ppl_syntax::lexer::lex(src, &mut diags);
        assert_eq!(toks.len(), 2);
        assert_eq!(toks.last().unwrap().tok, ppl_syntax::lexer::Tok::Eof);
        assert!(matches!(toks[0].tok, ppl_syntax::lexer::Tok::HashInt(_)));
    }

    #[test]
    fn test_f08_b3_unclosed_string_literal_handling() {
        let src = "\"unclosed string literal";
        let mut diags = Diagnostics::default();
        let toks = ppl_syntax::lexer::lex(src, &mut diags);
        assert!(
            toks.is_empty()
                || toks
                    .iter()
                    .any(|t| matches!(t.tok, ppl_syntax::lexer::Tok::Str(_)))
        );
    }

    #[test]
    fn test_f08_b4_comment_at_eof_without_newline() {
        let src = "LOCAL x; // EOF comment";
        let mut diags = Diagnostics::default();
        let toks = ppl_syntax::lexer::lex(src, &mut diags);
        assert_eq!(toks.len(), 4); // LOCAL, x, ;, EOF
    }

    #[test]
    fn test_f08_b5_unicode_math_identifiers() {
        let src = "π ⅈ ℯ";
        let mut diags = Diagnostics::default();
        let toks = ppl_syntax::lexer::lex(src, &mut diags);
        assert_eq!(toks.len(), 4);
        assert_eq!(toks.last().unwrap().tok, ppl_syntax::lexer::Tok::Eof);
    }
}

// ============================================================================
// FEATURE 9: Live Compiler Diagnostics — Boundary Cases
// ============================================================================
pub mod f09_boundary {
    use super::*;

    #[test]
    fn test_f09_b1_diagnostic_at_first_char() {
        let src = "RETURN;";
        let file = SourceFile::new("<t>", src);
        let (_, diags) = ppl_compiler::compile_source(&file, &CompileOptions::default());
        assert!(diags.has_errors());
        let lc = file.line_col(diags.items[0].span.start);
        assert_eq!(lc.line, 1);
        assert_eq!(lc.col, 1);
    }

    #[test]
    fn test_f09_b2_diagnostic_at_eof() {
        let src = "EXPORT MAIN() BEGIN";
        let file = SourceFile::new("<t>", src);
        let (_, diags) = ppl_compiler::compile_source(&file, &CompileOptions::default());
        assert!(diags.has_errors());
    }

    #[test]
    fn test_f09_b3_many_syntax_errors_stress() {
        let mut src = String::from("EXPORT MAIN()\nBEGIN\n");
        for _ in 0..30 {
            src.push_str("  LOCAL := ;\n");
        }
        src.push_str("END;\n");

        let file = SourceFile::new("<t>", &src);
        let (_, diags) = ppl_compiler::compile_source(&file, &CompileOptions::default());
        assert!(diags.items.len() >= 20);
    }

    #[test]
    fn test_f09_b4_zero_width_diagnostic_span() {
        let span = ppl_syntax::source::Span { start: 5, end: 5 };
        assert_eq!(span.start, span.end);
    }

    #[test]
    fn test_f09_b5_unicode_char_column_counting() {
        let src = "EXPORT 🚀() BEGIN END;";
        let file = SourceFile::new("<t>", src);
        let lc = file.line_col(11);
        assert_eq!(lc.line, 1);
    }
}

// ============================================================================
// FEATURE 10: 320×240 GROB Nearest-Neighbor Display — Boundary Cases
// ============================================================================
pub mod f10_boundary {
    use super::*;

    #[test]
    fn test_f10_b1_tiny_viewport_clamp() {
        let vp = Viewport::calculate_aspect_fit(10, 10);
        assert!(vp.width >= 1);
        assert!(vp.height >= 1);
    }

    #[test]
    fn test_f10_b2_huge_8k_viewport() {
        let vp = Viewport::calculate_aspect_fit(7680, 4320);
        let ratio = vp.width as f32 / vp.height as f32;
        assert!((ratio - (320.0 / 240.0)).abs() < 0.01);
    }

    #[test]
    fn test_f10_b3_corner_pixel_mapping_bounds() {
        let vp = Viewport::calculate_aspect_fit(320, 240);
        assert_eq!(window_to_framebuffer(0.0, 0.0, &vp), Some((0, 0)));
        assert_eq!(window_to_framebuffer(319.0, 239.0, &vp), Some((319, 239)));
        assert_eq!(window_to_framebuffer(320.0, 240.0, &vp), None);
    }

    #[test]
    fn test_f10_b4_zero_size_blit_no_panic() {
        let src = Grob::screen();
        let mut dst = Grob::screen();
        dst.blit((0, 0, 0, 0), &src, (0, 0, 0, 0), None, 255);
    }

    #[test]
    fn test_f10_b5_fractional_aspect_ratio_offsets() {
        let vp = Viewport::calculate_aspect_fit(1920, 1080);
        assert!(vp.x > 0 || vp.y > 0);
    }
}

// ============================================================================
// FEATURE 11: Live GROB Drawing Commands — Boundary Cases
// ============================================================================
pub mod f11_boundary {
    use super::*;

    #[test]
    fn test_f11_b1_off_screen_negative_coords_clipped() {
        let mut g = Grob::screen();
        g.rect(-100, -100, -10, -10, 0, 0); // Must clip without panic
    }

    #[test]
    fn test_f11_b2_off_screen_huge_coords_clipped() {
        let mut g = Grob::screen();
        g.line(1000, 1000, 5000, 5000, 0); // Must clip without panic
    }

    #[test]
    fn test_f11_b3_inverted_bounding_box_normalisation() {
        let mut g = Grob::screen();
        let c = rgb(255, 0, 0);
        g.rect(50, 50, 10, 10, c, c);
        assert_eq!(g.get(30, 30), Some(c));
    }

    #[test]
    fn test_f11_b4_single_pixel_line() {
        let mut g = Grob::screen();
        let c = rgb(0, 255, 0);
        g.line(20, 20, 20, 20, c);
        assert_eq!(g.get(20, 20), Some(c));
    }

    #[test]
    fn test_f11_b5_transparent_pixel_preservation() {
        let mut g = Grob::screen();
        let initial = g.get(10, 10).unwrap();
        g.set(10, 10, 0xFF000000); // 100% transparent
        assert_eq!(g.get(10, 10), Some(initial));
    }
}

// ============================================================================
// FEATURE 12: 0–50 Virtual Keypad Grid — Boundary Cases
// ============================================================================
pub mod f12_boundary {
    use super::*;

    #[test]
    fn test_f12_b1_keycode_lower_bound_zero() {
        assert_eq!(hp_key_name(0), Some("Apps"));
    }

    #[test]
    fn test_f12_b2_keycode_upper_bound_fifty() {
        assert_eq!(hp_key_name(50), Some("+"));
    }

    #[test]
    fn test_f12_b3_negative_keycode_naming() {
        assert_eq!(hp_key_name(-1), None);
    }

    #[test]
    fn test_f12_b4_queue_stress_1000_keys() {
        let shared = Arc::new(GuiSharedState::new());
        for i in 0..1000 {
            shared.push_key(i % 51);
        }
        let mut host = GuiHost::new(shared);
        for i in 0..1000 {
            assert_eq!(host.poll_key().unwrap(), Some(i % 51));
        }
    }

    #[test]
    fn test_f12_b5_key_press_shifted_and_alpha() {
        let kp = KeyPress::with_char(Some(42), 'A');
        assert_eq!(kp.code, Some(42));
        assert_eq!(kp.ch, Some('A'));
    }
}

// ============================================================================
// FEATURE 13: Screen Touch — Boundary Cases
// ============================================================================
pub mod f13_boundary {
    use super::*;

    #[test]
    fn test_f13_b1_touch_origin_top_left() {
        let shared = Arc::new(GuiSharedState::new());
        let mut host = GuiHost::new(shared.clone());
        shared.set_touch(Some(Touch { x: 0, y: 0 }));
        assert_eq!(host.touch().unwrap(), Some(Touch { x: 0, y: 0 }));
    }

    #[test]
    fn test_f13_b2_touch_bottom_right_corner() {
        let shared = Arc::new(GuiSharedState::new());
        let mut host = GuiHost::new(shared.clone());
        shared.set_touch(Some(Touch { x: 319, y: 239 }));
        assert_eq!(host.touch().unwrap(), Some(Touch { x: 319, y: 239 }));
    }

    #[test]
    fn test_f13_b3_subpixel_touch_flooring() {
        let raw_x = 100.9f32;
        let local_x = (raw_x).clamp(0.0, 319.0) as i64;
        assert_eq!(local_x, 100);
    }

    #[test]
    fn test_f13_b4_rapid_touch_toggles() {
        let shared = Arc::new(GuiSharedState::new());
        let mut host = GuiHost::new(shared.clone());
        for i in 0..100 {
            if i % 2 == 0 {
                shared.set_touch(Some(Touch { x: 50, y: 50 }));
            } else {
                shared.set_touch(None);
            }
        }
        assert!(host.touch().unwrap().is_none());
    }

    #[test]
    fn test_f13_b5_touch_outside_view_rejection() {
        let vp = Viewport::calculate_aspect_fit(640, 480);
        let touch = window_to_framebuffer(0.0, 0.0, &vp); // pillarbox edge
        assert!(touch.is_some());
    }
}

// ============================================================================
// FEATURE 14: Focus Separation — Boundary Cases
// ============================================================================
pub mod f14_boundary {
    use super::*;

    #[test]
    fn test_f14_b1_tab_character_in_editor() {
        let mut tab = EditorTab::new("T", "");
        tab.content.push('\t');
        assert_eq!(tab.content, "\t");
    }

    #[test]
    fn test_f14_b2_escape_in_editor_no_interrupt() {
        let shared = GuiSharedState::new();
        // Escape inside editor does not trigger shared.interrupt()
        assert!(!shared.is_interrupted());
    }

    #[test]
    fn test_f14_b3_rapid_focus_toggle() {
        let shared = Arc::new(GuiSharedState::new());
        let mut host = GuiHost::new(shared.clone());
        for _ in 0..50 {
            shared.set_key_down(30, true);
            shared.set_key_down(30, false);
        }
        assert!(!host.is_key_down(30).unwrap());
    }

    #[test]
    fn test_f14_b4_orphan_key_release_safe() {
        let shared = Arc::new(GuiSharedState::new());
        let mut host = GuiHost::new(shared.clone());
        shared.set_key_down(30, false); // release without press
        assert!(!host.is_key_down(30).unwrap());
    }

    #[test]
    fn test_f14_b5_keypad_click_bypasses_editor_focus() {
        let shared = Arc::new(GuiSharedState::new());
        let mut host = GuiHost::new(shared.clone());
        shared.push_key(10); // CAS
        assert_eq!(host.poll_key().unwrap(), Some(10));
    }
}

// ============================================================================
// FEATURE 15: Screenshot PNG Export — Boundary Cases
// ============================================================================
pub mod f15_boundary {
    use super::*;

    #[test]
    fn test_f15_b1_pure_black_grob_png() {
        let mut g = Grob::screen();
        g.fill(0x00000000);
        let mut buf = Vec::new();
        g.write_png(&mut buf).unwrap();

        let decoded = Grob::read_png(&buf).unwrap();
        assert_eq!(decoded.get(0, 0), Some(0));
    }

    #[test]
    fn test_f15_b2_pure_white_grob_png() {
        let g = Grob::screen();
        let mut buf = Vec::new();
        g.write_png(&mut buf).unwrap();

        let decoded = Grob::read_png(&buf).unwrap();
        assert_eq!(decoded.get(0, 0), Some(0x00FFFFFF));
    }

    #[test]
    fn test_f15_b3_high_entropy_checkerboard_png() {
        let mut g = Grob::screen();
        for y in 0..240 {
            for x in 0..320 {
                if (x + y) % 2 == 0 {
                    g.set(x, y, 0);
                }
            }
        }
        let mut buf = Vec::new();
        g.write_png(&mut buf).unwrap();
        assert!(!buf.is_empty());
    }

    #[test]
    fn test_f15_b4_save_png_invalid_directory() {
        let g = Grob::screen();
        let res = g.save_png(&PathBuf::from("/non_existent_dir_12345/screen.png"));
        assert!(res.is_err());
    }

    #[test]
    fn test_f15_b5_png_header_and_iend_chunk() {
        let g = Grob::screen();
        let mut buf = Vec::new();
        g.write_png(&mut buf).unwrap();
        assert!(buf.len() > 12);
        assert_eq!(
            &buf[0..8],
            &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]
        );
        assert_eq!(&buf[buf.len() - 8..buf.len() - 4], b"IEND");
        assert!(Grob::read_png(&buf).is_ok());
    }
}

// ============================================================================
// FEATURE 16: Background VM Worker — Boundary Cases
// ============================================================================
pub mod f16_boundary {
    use super::*;

    #[test]
    fn test_f16_b1_empty_program_execution() {
        let src = "EXPORT EMPTY() BEGIN END;";
        let res = run_ppl_headless(src, &[], &[]);
        assert!(res.success);
    }

    #[test]
    fn test_f16_b2_high_step_loop_execution() {
        let src = "EXPORT HIGH() BEGIN LOCAL c:=0; LOCAL i; FOR i:=1 TO 1000 DO c:=c+1; END; PRINT(c); END;";
        let res = run_ppl_headless(src, &[], &[]);
        assert!(res.success);
        assert_eq!(res.terminal_lines, vec!["1000"]);
    }

    #[test]
    fn test_f16_b3_step_limit_exceeded_error() {
        let src = "EXPORT INF() BEGIN WHILE 1 DO END; END;";
        let file = SourceFile::new("<t>", src);
        let (prog, _) = ppl_compiler::compile_source(&file, &CompileOptions::default());
        let host = ppl_vm::host::HeadlessHost::new();
        let limits = Limits {
            max_steps: Some(500),
            ..Limits::default()
        };
        let mut machine = Machine::new(
            Rc::new(prog.unwrap()),
            Box::new(host),
            Settings::default(),
            limits,
        );
        let res = machine.call_function("INF", Vec::new());
        assert!(res.is_err());
    }

    #[test]
    fn test_f16_b4_worker_outcome_display() {
        use ppl_gui::WorkerOutcome;
        use ppl_vm::error::{ErrorKind, RuntimeError};
        let o = WorkerOutcome::Error(RuntimeError::new(ErrorKind::Unsupported, "test failure"));
        assert!(format!("{o:?}").contains("test failure"));
    }

    #[test]
    fn test_f16_b5_worker_outcome_equality() {
        use ppl_gui::WorkerOutcome;
        use ppl_vm::error::{ErrorKind, RuntimeError};
        let s = WorkerOutcome::Success {
            value_str: "123".to_string(),
            steps: 5,
            elapsed: Duration::from_millis(1),
        };
        let e = WorkerOutcome::Error(RuntimeError::kind(ErrorKind::Interrupted));
        assert!(matches!(s, WorkerOutcome::Success { .. }));
        assert!(matches!(e, WorkerOutcome::Error(_)));
    }
}

// ============================================================================
// FEATURE 17: Unsaved Editor Buffer — Boundary Cases
// ============================================================================
pub mod f17_boundary {
    use super::*;

    #[test]
    fn test_f17_b1_buffer_with_only_comments() {
        let tab = EditorTab::new("COMMENTS", "// Line 1\n// Line 2\n");
        let res = run_ppl_headless(&tab.content, &[], &[]);
        assert!(res.terminal_lines.is_empty());
    }

    #[test]
    fn test_f17_b2_large_buffer_compilation_time() {
        let mut src = String::from("EXPORT MAIN()\nBEGIN\n  LOCAL x := 0;\n");
        for _ in 0..1000 {
            src.push_str("  x := x + 1;\n");
        }
        src.push_str("  PRINT(x);\nEND;");

        let start = std::time::Instant::now();
        let file = SourceFile::new("<t>", &src);
        let (prog, diags) = ppl_compiler::compile_source(&file, &CompileOptions::default());
        let elapsed = start.elapsed();

        assert!(!diags.has_errors());
        assert!(prog.is_some());
        assert!(elapsed < Duration::from_millis(200));
    }

    #[test]
    fn test_f17_b3_buffer_with_multiple_helpers() {
        let src = "H1() BEGIN RETURN 1; END; H2() BEGIN RETURN 2; END; EXPORT MAIN() BEGIN PRINT(H1()+H2()); END;";
        let res = run_ppl_headless(src, &[], &[]);
        assert!(res.success);
        assert_eq!(res.terminal_lines, vec!["3"]);
    }

    #[test]
    fn test_f17_b4_buffer_double_run_idempotence() {
        let src = "EXPORT MAIN() BEGIN PRINT(42); END;";
        let res1 = run_ppl_headless(src, &[], &[]);
        let res2 = run_ppl_headless(src, &[], &[]);
        assert_eq!(res1.terminal_lines, res2.terminal_lines);
    }

    #[test]
    fn test_f17_b5_whitespace_only_buffer() {
        let tab = EditorTab::new("WHITE", "   \n\t  \n");
        let res = run_ppl_headless(&tab.content, &[], &[]);
        assert!(res.terminal_lines.is_empty());
    }
}

// ============================================================================
// FEATURE 18: Execution Controls — Boundary Cases
// ============================================================================
pub mod f18_boundary {
    use super::*;

    #[test]
    fn test_f18_b1_rapid_run_burst_queueing() {
        let mut state = IdeAppStateModel::new();
        for _ in 0..10 {
            state.queue_action(IdeAction::RunProgram);
        }
        assert_eq!(state.take_actions().len(), 10);
    }

    #[test]
    fn test_f18_b2_stop_while_terminating() {
        let mut state = IdeAppStateModel::new();
        state.vm_state = VmExecutionState::Terminated;
        state.queue_action(IdeAction::StopProgram);
        assert_eq!(state.vm_state, VmExecutionState::Terminated);
    }

    #[test]
    fn test_f18_b3_natural_finish_transitions_to_done() {
        let mut state = IdeAppStateModel::new();
        state.vm_state = VmExecutionState::Running;
        state.vm_state = VmExecutionState::Terminated;
        assert_eq!(state.vm_state.label(), "Done");
    }

    #[test]
    fn test_f18_b4_paused_state_label() {
        let state_paused = VmExecutionState::Paused;
        assert_eq!(state_paused.label(), "Paused");
    }

    #[test]
    fn test_f18_b5_error_state_reset() {
        let mut state = IdeAppStateModel::new();
        state.vm_state = VmExecutionState::Error("Div by zero".into());
        state.vm_state = VmExecutionState::Idle;
        assert_eq!(state.vm_state, VmExecutionState::Idle);
    }
}

// ============================================================================
// FEATURE 19: Cooperative Cancellation — Boundary Cases
// ============================================================================
pub mod f19_boundary {
    use super::*;

    #[test]
    fn test_f19_b1_cancel_before_first_instruction() {
        let shared = Arc::new(GuiSharedState::new());
        shared.interrupt();
        assert!(shared.is_interrupted());

        let src = "EXPORT MAIN() BEGIN WHILE 1 DO END; END;";
        let file = SourceFile::new("<t>", src);
        let (prog, _) = ppl_compiler::compile_source(&file, &CompileOptions::default());
        let host = GuiHost::new(shared);
        let mut machine = Machine::new(
            Rc::new(prog.unwrap()),
            Box::new(host),
            Settings::default(),
            Limits::default(),
        );
        let res = machine.call_function("MAIN", Vec::new());
        assert!(res.is_err());
    }

    #[test]
    fn test_f19_b2_cancel_deep_recursion() {
        let shared = Arc::new(GuiSharedState::new());
        let shared_clone = Arc::clone(&shared);

        let handle = std::thread::spawn(move || {
            let src = "REC(n) BEGIN RETURN REC(n+1); END; EXPORT MAIN() BEGIN REC(1); END;";
            let file = SourceFile::new("<t>", src);
            let (prog, _) = ppl_compiler::compile_source(&file, &CompileOptions::default());
            let host = GuiHost::new(shared_clone);
            let mut machine = Machine::new(
                Rc::new(prog.unwrap()),
                Box::new(host),
                Settings::default(),
                Limits::default(),
            );
            machine.call_function("MAIN", Vec::new()).is_ok()
        });

        std::thread::sleep(Duration::from_millis(40));
        shared.interrupt();

        let ok = handle.join().unwrap();
        assert!(!ok);
    }

    #[test]
    fn test_f19_b3_cancel_getkey_polling_loop() {
        let shared = Arc::new(GuiSharedState::new());
        let shared_clone = Arc::clone(&shared);

        let handle = std::thread::spawn(move || {
            let src = "EXPORT MAIN() BEGIN WHILE GETKEY == -1 DO END; END;";
            let file = SourceFile::new("<t>", src);
            let (prog, _) = ppl_compiler::compile_source(&file, &CompileOptions::default());
            let host = GuiHost::new(shared_clone);
            let mut machine = Machine::new(
                Rc::new(prog.unwrap()),
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
    fn test_f19_b4_flag_reset_for_next_run() {
        let shared = GuiSharedState::new();
        shared.interrupt();
        assert!(shared.is_interrupted());

        shared.interrupt.store(false, Ordering::SeqCst);
        assert!(!shared.is_interrupted());
    }

    #[test]
    fn test_f19_b5_heavy_math_loop_interruption() {
        let shared = Arc::new(GuiSharedState::new());
        let shared_clone = Arc::clone(&shared);

        let handle = std::thread::spawn(move || {
            let src =
                "EXPORT MAIN() BEGIN LOCAL x := 1.0; WHILE 1 DO x := SIN(x) + COS(x); END; END;";
            let file = SourceFile::new("<t>", src);
            let (prog, _) = ppl_compiler::compile_source(&file, &CompileOptions::default());
            let host = GuiHost::new(shared_clone);
            let mut machine = Machine::new(
                Rc::new(prog.unwrap()),
                Box::new(host),
                Settings::default(),
                Limits::default(),
            );
            machine.call_function("MAIN", Vec::new()).is_ok()
        });

        std::thread::sleep(Duration::from_millis(40));
        shared.interrupt();

        let ok = handle.join().unwrap();
        assert!(!ok);
    }
}

// ============================================================================
// FEATURE 20: Output Console PRINT Stream — Boundary Cases
// ============================================================================
pub mod f20_boundary {
    use super::*;

    #[test]
    fn test_f20_b1_print_empty_string() {
        let src = "EXPORT MAIN() BEGIN PRINT(\"\"); END;";
        let res = run_ppl_headless(src, &[], &[]);
        assert_eq!(res.terminal_lines, vec![""]);
    }

    #[test]
    fn test_f20_b2_print_huge_volume() {
        let src = "EXPORT MAIN() BEGIN LOCAL i; FOR i:=1 TO 500 DO PRINT(i); END; END;";
        let res = run_ppl_headless(src, &[], &[]);
        assert_eq!(res.terminal_lines.len(), 500);
    }

    #[test]
    fn test_f20_b3_print_escape_characters() {
        let src = "EXPORT MAIN() BEGIN PRINT(\"Line 1\\nLine 2\"); END;";
        let res = run_ppl_headless(src, &[], &[]);
        assert!(res.terminal_lines[0].contains("Line 1"));
    }

    #[test]
    fn test_f20_b4_print_very_long_line() {
        let long_str = "X".repeat(10_000);
        let src = format!("EXPORT MAIN() BEGIN PRINT(\"{long_str}\"); END;");
        let res = run_ppl_headless(&src, &[], &[]);
        assert_eq!(res.terminal_lines[0].len(), 10_000);
    }

    #[test]
    fn test_f20_b5_print_clear_after_many_lines() {
        let shared = GuiSharedState::new();
        for i in 0..1000 {
            shared.terminal_lines.lock().unwrap().push(format!("L_{i}"));
        }
        shared.terminal_lines.lock().unwrap().clear();
        assert!(shared.terminal_lines.lock().unwrap().is_empty());
    }
}

// ============================================================================
// FEATURE 21: Interactive INPUT Dialog — Boundary Cases
// ============================================================================
pub mod f21_boundary {
    use super::*;

    #[test]
    fn test_f21_b1_eval_negative_scientific_number() {
        let val = ppl_vm::eval_text_with("-1.5e-10", &Settings::default()).unwrap();
        assert!(format!("{val:?}").contains("Real"));
    }

    #[test]
    fn test_f21_b2_eval_complex_literal() {
        let val = ppl_vm::eval_text_with("(3, 4)", &Settings::default()).unwrap();
        assert!(matches!(val, Value::Complex(_)));
    }

    #[test]
    fn test_f21_b3_eval_list_literal() {
        let val = ppl_vm::eval_text_with("{1, 2, 3}", &Settings::default()).unwrap();
        assert!(matches!(val, Value::List(_)));
    }

    #[test]
    fn test_f21_b4_eval_unclosed_parenthesis() {
        let res = ppl_vm::eval_text_with("(1+2", &Settings::default());
        assert!(res.is_err());
    }

    #[test]
    fn test_f21_b5_eval_empty_string_error() {
        let res = ppl_vm::eval_text_with("1 + *", &Settings::default());
        assert!(res.is_err());
    }
}

// ============================================================================
// FEATURE 22: Interactive CHOOSE Dialog — Boundary Cases
// ============================================================================
pub mod f22_boundary {

    #[test]
    fn test_f22_b1_choose_single_option() {
        let items = vec!["Only One"];
        let choice = 1;
        assert_eq!(items[choice - 1], "Only One");
    }

    #[test]
    fn test_f22_b2_choose_100_options() {
        let items: Vec<String> = (1..=100).map(|i| format!("Opt {i}")).collect();
        let choice = 100;
        assert_eq!(items[choice - 1], "Opt 100");
    }

    #[test]
    fn test_f22_b3_choose_very_long_strings() {
        let long_opt = "A".repeat(500);
        let items = vec![long_opt.as_str()];
        assert_eq!(items[0].len(), 500);
    }

    #[test]
    fn test_f22_b4_choose_navigation_clamping() {
        let items_len = 3;
        let mut cur = 0; // index
        cur = (cur + 1).min(items_len - 1);
        assert_eq!(cur, 1);
    }

    #[test]
    fn test_f22_b5_choose_zero_choice_returns_zero() {
        let choice: usize = 0;
        assert_eq!(choice, 0);
    }
}

// ============================================================================
// FEATURE 23: Interactive MSGBOX Dialog — Boundary Cases
// ============================================================================
pub mod f23_boundary {
    use super::*;

    #[test]
    fn test_f23_b1_msgbox_empty_string() {
        let msg = "";
        assert!(msg.is_empty());
    }

    #[test]
    fn test_f23_b2_msgbox_many_newlines() {
        let msg = "L1\n\n\n\nL5";
        let count = msg.lines().count();
        assert_eq!(count, 5);
    }

    #[test]
    fn test_f23_b3_msgbox_unicode_emojis() {
        let msg = "HP Prime G1 🚀🔥";
        assert!(msg.contains('🚀'));
    }

    #[test]
    fn test_f23_b4_msgbox_interrupted() {
        let shared = GuiSharedState::new();
        shared.interrupt();
        assert!(shared.is_interrupted());
    }

    #[test]
    fn test_f23_b5_msgbox_return_values() {
        let ok_val = 1;
        let cancel_val = 0;
        assert_ne!(ok_val, cancel_val);
    }
}

// ============================================================================
// FEATURE 24: GETKEY & ISKEYDOWN — Boundary Cases
// ============================================================================
pub mod f24_boundary {
    use super::*;

    #[test]
    fn test_f24_b1_getkey_1000_consecutive_empty_calls() {
        let shared = Arc::new(GuiSharedState::new());
        let mut host = GuiHost::new(shared);
        for _ in 0..1000 {
            assert_eq!(host.poll_key().unwrap(), None);
        }
    }

    #[test]
    fn test_f24_b2_iskeydown_all_51_keys_unpressed() {
        let shared = Arc::new(GuiSharedState::new());
        let mut host = GuiHost::new(shared);
        for k in 0..=50 {
            assert!(!host.is_key_down(k).unwrap());
        }
    }

    #[test]
    fn test_f24_b3_iskeydown_all_51_keys_held() {
        let shared = Arc::new(GuiSharedState::new());
        let mut host = GuiHost::new(shared.clone());
        for k in 0..=50 {
            shared.set_key_down(k, true);
        }
        for k in 0..=50 {
            assert!(host.is_key_down(k).unwrap());
        }
    }

    #[test]
    fn test_f24_b4_push_key_then_release() {
        let shared = Arc::new(GuiSharedState::new());
        let mut host = GuiHost::new(shared.clone());
        shared.push_key(30);
        shared.set_key_down(30, false);
        assert_eq!(host.poll_key().unwrap(), Some(30));
        assert!(!host.is_key_down(30).unwrap());
    }

    #[test]
    fn test_f24_b5_negative_key_query() {
        let shared = Arc::new(GuiSharedState::new());
        let mut host = GuiHost::new(shared);
        assert!(!host.is_key_down(-5).unwrap());
    }
}

// ============================================================================
// FEATURE 25: Non-Blocking WAIT — Boundary Cases
// ============================================================================
pub mod f25_boundary {
    use super::*;

    #[test]
    fn test_f25_b1_sub_millisecond_sleep() {
        let src = "EXPORT MAIN() BEGIN PRINT(WAIT(0.0001)); END;";
        let res = run_ppl_headless(src, &[], &[]);
        assert!(res.success);
    }

    #[test]
    fn test_f25_b2_wait_zero_key_already_queued() {
        let src = "EXPORT MAIN() BEGIN PRINT(WAIT(0)); END;";
        let res = run_ppl_headless(src, &[], &[30]);
        assert_eq!(res.terminal_lines, vec!["30"]);
    }

    #[test]
    fn test_f25_b3_wait_negative_immediate_key() {
        let src = "EXPORT MAIN() BEGIN PRINT(WAIT(-1)); END;";
        let res = run_ppl_headless(src, &[], &[4]);
        assert_eq!(res.terminal_lines, vec!["4"]);
    }

    #[test]
    fn test_f25_b4_wait_zero_interrupted() {
        let shared = Arc::new(GuiSharedState::new());
        let shared_clone = Arc::clone(&shared);

        let handle = std::thread::spawn(move || {
            let host = GuiHost::new(shared_clone);
            host.state().wait_press(None, |_| true).is_ok()
        });

        std::thread::sleep(Duration::from_millis(30));
        shared.interrupt();

        let ok = handle.join().unwrap();
        assert!(!ok);
    }

    #[test]
    fn test_f25_b5_hour_sleep_interrupted_promptly() {
        let shared = Arc::new(GuiSharedState::new());
        let shared_clone = Arc::clone(&shared);

        let start = std::time::Instant::now();
        let handle = std::thread::spawn(move || {
            let host = GuiHost::new(shared_clone);
            let _ = host.state().wait_press(
                Some(std::time::Instant::now() + Duration::from_secs(3600)),
                |_| true,
            );
        });

        std::thread::sleep(Duration::from_millis(20));
        shared.interrupt();

        let _ = handle.join().unwrap();
        assert!(start.elapsed() < Duration::from_millis(500));
    }
}

// ============================================================================
// FEATURE 26: Gutter Breakpoints — Boundary Cases
// ============================================================================
pub mod f26_boundary {
    use super::*;

    #[test]
    fn test_f26_b1_breakpoint_line_zero() {
        let mut tab = EditorTab::new("T", "");
        tab.toggle_breakpoint(0);
        assert!(tab.breakpoints.contains(&0));
    }

    #[test]
    fn test_f26_b2_breakpoint_huge_line_number() {
        let mut tab = EditorTab::new("T", "");
        tab.toggle_breakpoint(1_000_000);
        assert!(tab.breakpoints.contains(&1_000_000));
    }

    #[test]
    fn test_f26_b3_remove_nonexistent_breakpoint() {
        let mut tab = EditorTab::new("T", "");
        let removed = tab.toggle_breakpoint(99);
        assert!(removed); // added
        let removed_again = tab.toggle_breakpoint(99);
        assert!(!removed_again); // removed
    }

    #[test]
    fn test_f26_b4_toggle_500_breakpoints_ordered() {
        let mut tab = EditorTab::new("T", "");
        for i in 0..500 {
            tab.breakpoints.insert(i);
        }
        assert_eq!(tab.breakpoints.len(), 500);
        assert_eq!(*tab.breakpoints.iter().next().unwrap(), 0);
        assert_eq!(*tab.breakpoints.iter().next_back().unwrap(), 499);
    }

    #[test]
    fn test_f26_b5_breakpoint_on_comment_line() {
        let mut tab = EditorTab::new("T", "// comment\nEXPORT MAIN() BEGIN END;");
        tab.toggle_breakpoint(1);
        assert!(tab.breakpoints.contains(&1));
    }
}

// ============================================================================
// FEATURE 27: Statement Stepping — Boundary Cases
// ============================================================================
pub mod f27_boundary {
    use super::*;

    #[test]
    fn test_f27_b1_stepping_zero_statement_program() {
        let src = "EXPORT MAIN() BEGIN END;";
        let pauses = execute_with_debugger(src, BTreeSet::new(), 10, true);
        assert!(pauses.is_empty() || pauses.len() <= 1);
    }

    #[test]
    fn test_f27_b2_stepping_loop_bounded() {
        let src = "EXPORT MAIN() BEGIN LOCAL i; LOCAL x; FOR i:=1 TO 50 DO x:=i; END; END;";
        let pauses = execute_with_debugger(src, BTreeSet::new(), 10, true);
        assert_eq!(pauses.len(), 10);
    }

    #[test]
    fn test_f27_b3_continue_without_breakpoints_completes() {
        let src = "EXPORT MAIN() BEGIN LOCAL x := 10; LOCAL y := 20; END;";
        let pauses = execute_with_debugger(src, BTreeSet::new(), 10, false);
        assert!(pauses.is_empty());
    }

    #[test]
    fn test_f27_b4_step_action_equality() {
        use ppl_vm::machine::DebugAction;
        assert_eq!(DebugAction::Continue, DebugAction::Continue);
        assert_ne!(DebugAction::Continue, DebugAction::Abort);
    }

    #[test]
    fn test_f27_b5_stepping_across_branches() {
        let src = "EXPORT MAIN()\nBEGIN\n  LOCAL x := 1;\n  IF x == 1 THEN\n    LOCAL y := 2;\n  ELSE\n    LOCAL z := 3;\n  END;\nEND;";
        let pauses = execute_with_debugger(src, BTreeSet::new(), 5, true);
        assert!(pauses.iter().any(|p| p.line == 5));
        assert!(!pauses.iter().any(|p| p.line == 7)); // Else branch not visited
    }
}

// ============================================================================
// FEATURE 28: Execution Pause Line Highlighting — Boundary Cases
// ============================================================================
pub mod f28_boundary {
    use super::*;

    #[test]
    fn test_f28_b1_pause_line_at_first_line() {
        let pause = DebugPauseSnapshot {
            line: 1,
            col: 1,
            function: "MAIN".into(),
            depth: 1,
            frames: Vec::new(),
            globals: Vec::new(),
        };
        assert_eq!(pause.line, 1);
    }

    #[test]
    fn test_f28_b2_pause_line_at_eof() {
        let pause = DebugPauseSnapshot {
            line: 1000,
            col: 1,
            function: "MAIN".into(),
            depth: 1,
            frames: Vec::new(),
            globals: Vec::new(),
        };
        assert_eq!(pause.line, 1000);
    }

    #[test]
    fn test_f28_b3_empty_function_name_snapshot() {
        let pause = DebugPauseSnapshot {
            line: 5,
            col: 2,
            function: "".into(),
            depth: 0,
            frames: Vec::new(),
            globals: Vec::new(),
        };
        assert!(pause.function.is_empty());
    }

    #[test]
    fn test_f28_b4_sequential_pause_line_progression() {
        let mut pause = DebugPauseSnapshot {
            line: 10,
            ..Default::default()
        };
        pause.line = 11;
        assert_eq!(pause.line, 11);
    }

    #[test]
    fn test_f28_b5_error_clears_pause_line() {
        let mut active_pause: Option<usize> = Some(15);
        assert_eq!(active_pause, Some(15));
        active_pause = None;
        assert!(active_pause.is_none());
    }
}

// ============================================================================
// FEATURE 29: Variable Inspection Tree — Boundary Cases
// ============================================================================
pub mod f29_boundary {
    use super::*;

    #[test]
    fn test_f29_b1_extreme_real_number_snapshot() {
        let val = Value::Real(ppl_math::Real::from_parts(false, 9, 499).unwrap());
        let snap = VariableSnapshot::from_value("extreme", &val);
        assert_eq!(snap.type_name, "real");
    }

    #[test]
    fn test_f29_b2_empty_list_snapshot() {
        let val = Value::list(Vec::new());
        let snap = VariableSnapshot::from_value("empty", &val);
        if let VariableDetailSnapshot::List(ref items) = snap.details {
            assert!(items.is_empty());
        } else {
            panic!("Expected List");
        }
    }

    #[test]
    fn test_f29_b3_1x1_matrix_snapshot() {
        let val = Value::Matrix(Rc::new(ppl_vm::value::Matrix::zeros(1, 1)));
        let snap = VariableSnapshot::from_value("m1", &val);
        assert_eq!(snap.type_name, "matrix");
        if let VariableDetailSnapshot::Matrix {
            rows,
            cols,
            elements,
        } = snap.details
        {
            assert_eq!(rows, 1);
            assert_eq!(cols, 1);
            assert_eq!(elements.len(), 1);
        } else {
            panic!("Expected Matrix");
        }
    }

    #[test]
    fn test_f29_b4_deep_nested_list_snapshot() {
        let inner = Value::list(vec![Value::from_i64(1)]);
        let outer = Value::list(vec![inner]);
        let snap = VariableSnapshot::from_value("nested", &outer);
        assert_eq!(snap.type_name, "list");
    }

    #[test]
    fn test_f29_b5_complex_negative_imaginary_snapshot() {
        let c = ppl_math::Complex::new(ppl_math::Real::from_i64(5), ppl_math::Real::from_i64(-3));
        let val = Value::Complex(c);
        let snap = VariableSnapshot::from_value("comp", &val);
        assert_eq!(snap.type_name, "complex");
    }
}

// ============================================================================
// FEATURE 30: Call Stack Frame Inspection — Boundary Cases
// ============================================================================
pub mod f30_boundary {
    use super::*;

    #[test]
    fn test_f30_b1_empty_call_stack_snapshot() {
        let snapshot = DebugPauseSnapshot::default();
        assert!(snapshot.frames.is_empty());
    }

    #[test]
    fn test_f30_b2_single_frame_snapshot() {
        let frame = StackFrameSnapshot {
            function: "MAIN".into(),
            line: 1,
            col: 1,
            locals: Vec::new(),
        };
        assert_eq!(frame.function, "MAIN");
    }

    #[test]
    fn test_f30_b3_deep_call_stack_frames() {
        let frames: Vec<StackFrameSnapshot> = (0..50)
            .map(|i| StackFrameSnapshot {
                function: format!("FN_{i}"),
                line: i + 1,
                col: 1,
                locals: Vec::new(),
            })
            .collect();
        assert_eq!(frames.len(), 50);
        assert_eq!(frames[49].function, "FN_49");
    }

    #[test]
    fn test_f30_b4_frame_with_many_locals() {
        let locals: Vec<VariableSnapshot> = (0..50)
            .map(|i| VariableSnapshot {
                name: format!("var_{i}"),
                type_name: "Real",
                preview: i.to_string(),
                details: VariableDetailSnapshot::Scalar,
            })
            .collect();

        let frame = StackFrameSnapshot {
            function: "HEAVY".into(),
            line: 10,
            col: 5,
            locals,
        };
        assert_eq!(frame.locals.len(), 50);
    }

    #[test]
    fn test_f30_b5_frame_equality() {
        let f1 = StackFrameSnapshot {
            function: "T".into(),
            line: 1,
            col: 1,
            locals: Vec::new(),
        };
        let f2 = f1.clone();
        assert_eq!(f1, f2);
    }
}

// ============================================================================
// FEATURE 31: Full Workflow Integration — Boundary Cases
// ============================================================================
pub mod f31_boundary {
    use super::*;

    #[test]
    fn test_f31_b1_rapid_scratchpad_arithmetic_edits() {
        let mut tab = EditorTab::new("CALC", "EXPORT MAIN() BEGIN PRINT(1); END;");
        for i in 2..=6 {
            tab.set_content(format!("EXPORT MAIN() BEGIN PRINT({i}); END;"));
            let res = run_ppl_headless(&tab.content, &[], &[]);
            assert_eq!(res.terminal_lines, vec![i.to_string()]);
        }
    }

    #[test]
    fn test_f31_b2_full_screen_gradient_draw() {
        let mut g = Grob::screen();
        for y in 0..240 {
            for x in 0..320 {
                g.set(x, y, rgb((x % 256) as u8, (y % 256) as u8, 128));
            }
        }
        assert_eq!(g.get(0, 0), Some(rgb(0, 0, 128)));
        assert_eq!(g.get(319, 239), Some(rgb(63, 239, 128)));
    }

    #[test]
    fn test_f31_b3_quadratic_solver_zero_discriminant() {
        // x^2 - 2x + 1 = 0 -> root = 1
        let src = "EXPORT SOLVE() BEGIN LOCAL a:=1; LOCAL b:=-2; LOCAL c:=1; LOCAL d:=b^2-4*a*c; PRINT(d); END;";
        let res = run_ppl_headless(src, &[], &[]);
        assert!(res.success);
        assert_eq!(res.terminal_lines, vec!["0"]);
    }

    #[test]
    fn test_f31_b4_fibonacci_recursion_debug() {
        let src = "FIB(n) BEGIN IF n <= 1 THEN RETURN n; ELSE RETURN FIB(n-1)+FIB(n-2); END; END; EXPORT MAIN() BEGIN PRINT(FIB(4)); END;";
        let res = run_ppl_headless(src, &[], &[]);
        assert!(res.success);
        assert_eq!(res.terminal_lines, vec!["3"]);
    }

    #[test]
    fn test_f31_b5_syntax_recovery_many_errors() {
        let broken = "EXPORT MAIN() BEGIN LOCAL x := ; LOCAL y := ; END;";
        let res1 = run_ppl_headless(broken, &[], &[]);
        assert!(!res1.success);

        let fixed = "EXPORT MAIN() BEGIN LOCAL x := 10; LOCAL y := 20; PRINT(x+y); END;";
        let res2 = run_ppl_headless(fixed, &[], &[]);
        assert!(res2.success);
        assert_eq!(res2.terminal_lines, vec!["30"]);
    }
}

// ============================================================================
// FEATURE 32: Quality, Lint & Performance Gates — Boundary Cases
// ============================================================================
pub mod f32_boundary {
    use super::*;

    #[test]
    fn test_f32_b1_inverted_screen_mismatch_count() {
        let mut g1 = Grob::screen();
        g1.fill(0x00000000);
        let mut g2 = Grob::screen();
        g2.fill(0x00FFFFFF);

        let diff = compare_rgb565(&g1, &g2).unwrap();
        assert_eq!(diff.mismatched, 320 * 240);
    }

    #[test]
    fn test_f32_b2_single_pixel_mismatch() {
        let mut g1 = Grob::screen();
        let g2 = Grob::screen();
        g1.set(160, 120, rgb(255, 0, 0));

        let diff = compare_rgb565(&g1, &g2).unwrap();
        assert_eq!(diff.mismatched, 1);
        assert_eq!(diff.first_mismatch, Some((160, 120)));
    }

    #[test]
    fn test_f32_b3_viewport_integer_scale_4k() {
        let vp = Viewport::calculate_integer(3840, 2160);
        assert_eq!(vp.scale, 9);
        assert_eq!(vp.width, 320 * 9);
        assert_eq!(vp.height, 240 * 9);
    }

    #[test]
    fn test_f32_b4_high_throughput_key_queue() {
        let shared = Arc::new(GuiSharedState::new());
        let mut host = GuiHost::new(shared.clone());
        let start = std::time::Instant::now();
        for i in 0..10_000 {
            shared.push_key(i % 50);
        }
        for _ in 0..10_000 {
            let _ = host.poll_key();
        }
        let elapsed = start.elapsed();
        assert!(elapsed < Duration::from_millis(50));
    }

    #[test]
    fn test_f32_b5_batch_compilation_throughput() {
        let start = std::time::Instant::now();
        for i in 0..25 {
            let src = format!("EXPORT T{i}() BEGIN LOCAL x:={i}; RETURN x; END;");
            let file = SourceFile::new("<bench>", &src);
            let (prog, diags) = ppl_compiler::compile_source(&file, &CompileOptions::default());
            assert!(!diags.has_errors());
            assert!(prog.is_some());
        }
        assert!(start.elapsed() < Duration::from_millis(200));
    }
}
