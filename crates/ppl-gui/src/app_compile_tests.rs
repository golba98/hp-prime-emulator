//! The compiler as the IDE's source of truth: live analysis state, stale
//! result rejection, the Problems panel, and the Run gate. Every test
//! drives the real [`IdeDesktopApp`] (and real egui frames where widgets
//! are involved).

use std::time::{Duration, Instant};

use super::*;
use crate::ide::AnalysisState;
use ppl_syntax::{DiagnosticClass, Severity};

const VALID: &str = "EXPORT DEMO()\nBEGIN\n  PRINT(\"Hello\");\nEND;\n";
const UNCLOSED_IF: &str = "EXPORT DEMO()\nBEGIN\n  LOCAL a;\n  IF a == 1 THEN\n";
const MISSING_VALUE: &str = "EXPORT DEMO()\nBEGIN\n  LOCAL a;\n  a := ;\nEND;\n";

fn app_with(src: &str) -> IdeDesktopApp {
    let mut app = IdeDesktopApp::default();
    app.editor.active_tab_mut().unwrap().content = src.to_string();
    app
}

/// One UI frame of live analysis.
fn frame(app: &mut IdeDesktopApp, ctx: &egui::Context) {
    let _ = ctx.run(Default::default(), |ctx| app.schedule_diagnostics(ctx));
}

/// Runs frames until every open tab has a verdict for its current text.
fn settle(app: &mut IdeDesktopApp, ctx: &egui::Context) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        frame(app, ctx);
        if app.problems.is_settled() || !app.problems.failures.is_empty() {
            let all = (0..app.editor.tabs.len()).all(|i| app.analysis_state(i).is_settled());
            if all {
                return;
            }
        }
        assert!(Instant::now() < deadline, "analysis never settled");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Polls the worker until the run ends.
fn finish_run(app: &mut IdeDesktopApp) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.controller.worker_alive() {
        app.poll_execution();
        assert!(Instant::now() < deadline, "run never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
    app.poll_execution();
}

fn console(app: &IdeDesktopApp) -> String {
    app.console.plain_text()
}

#[test]
fn valid_source_is_reported_valid_only_after_analysis() {
    let ctx = egui::Context::default();
    let mut app = app_with(VALID);
    // Nothing analysed yet: no "0 problems" claim.
    app.refresh_analysis_status();
    assert!(!app.problems.is_settled());
    assert!(
        app.problems.title().contains('…'),
        "{}",
        app.problems.title()
    );
    settle(&mut app, &ctx);
    assert_eq!(app.state.analysis, AnalysisState::Valid);
    assert!(app.problems.items.is_empty());
    assert_eq!(app.problems.title(), "Problems (0)");
}

#[test]
fn invalid_sources_report_errors_at_their_positions() {
    let ctx = egui::Context::default();
    // (source, every error as (code, line, col)) — the same findings as
    // `ppl check` on the same text.
    for (src, want) in [
        (
            UNCLOSED_IF,
            vec![("unclosed-block", 2, 1), ("unclosed-block", 4, 3)],
        ),
        (MISSING_VALUE, vec![("expected-expression", 4, 8)]),
    ] {
        let mut app = app_with(src);
        settle(&mut app, &ctx);
        assert!(
            matches!(app.state.analysis, AnalysisState::Invalid { .. }),
            "{:?}",
            app.state.analysis
        );
        let errors: Vec<_> = app
            .problems
            .items
            .iter()
            .filter(|p| p.severity == Severity::Error)
            .map(|p| (p.code.as_str(), p.line, p.col))
            .collect();
        assert_eq!(errors, want, "{src}");
        assert!(
            app.problems
                .items
                .iter()
                .all(|p| p.class == DiagnosticClass::Syntax),
            "{:#?}",
            app.problems.items
        );
    }
}

#[test]
fn an_edit_hides_old_diagnostics_until_reanalysed() {
    let ctx = egui::Context::default();
    let mut app = app_with(MISSING_VALUE);
    settle(&mut app, &ctx);
    assert!(app.problems.error_count() > 0);

    // Fix the source. Before the new analysis finishes, nothing from the
    // old revision may be shown, and the panel says it is analysing.
    app.editor.active_tab_mut().unwrap().content = VALID.to_string();
    frame(&mut app, &ctx);
    assert!(app.problems.items.is_empty(), "{:#?}", app.problems.items);
    assert!(!app.problems.is_settled());
    assert!(!app.state.analysis.is_settled(), "{:?}", app.state.analysis);
    assert!(
        app.editor
            .active_tab_mut()
            .unwrap()
            .current_diagnostics()
            .is_empty()
    );

    settle(&mut app, &ctx);
    assert_eq!(app.state.analysis, AnalysisState::Valid);
    assert!(app.problems.items.is_empty());

    // Break it again: the error comes back.
    app.editor.active_tab_mut().unwrap().content = MISSING_VALUE.to_string();
    settle(&mut app, &ctx);
    assert!(app.problems.error_count() > 0);
}

#[test]
fn each_tab_keeps_its_own_diagnostics() {
    let ctx = egui::Context::default();
    let mut app = app_with(VALID);
    app.editor.new_tab("BROKEN.ppl", MISSING_VALUE);
    settle(&mut app, &ctx);
    let broken = app.editor.tabs[1].id;
    assert!(app.problems.items.iter().all(|p| p.tab == Some(broken)));
    assert!(matches!(app.state.analysis, AnalysisState::Invalid { .. }));
    // Switching tabs changes the verdict shown, not the diagnostics.
    app.editor.active_tab_index = 0;
    settle(&mut app, &ctx);
    assert_eq!(app.state.analysis, AnalysisState::Valid);
    assert!(app.problems.items.iter().all(|p| p.tab == Some(broken)));
    assert!(app.editor.tabs[0].current_diagnostics().is_empty());
    assert!(!app.editor.tabs[1].current_diagnostics().is_empty());
}

#[test]
fn results_for_old_revisions_or_closed_tabs_are_rejected() {
    let ctx = egui::Context::default();
    let mut app = app_with(VALID);
    let tab = &mut app.editor.tabs[0];
    let old = tab.revision();
    tab.content = MISSING_VALUE.to_string();
    let item = diagnostics::check_source("DEMO.ppl", MISSING_VALUE);
    // A result computed for the previous text never applies.
    assert!(!IdeDesktopApp::set_analysis(tab, old, item, None));
    assert!(tab.analysis.is_none());

    // Rapid edits: many revisions, one final verdict for the final text.
    for i in 0..100 {
        app.editor.tabs[0].content = format!("EXPORT DEMO()\nBEGIN\n  RETURN {i};\nEND;\n");
        frame(&mut app, &ctx);
    }
    app.editor.tabs[0].content = MISSING_VALUE.to_string();
    settle(&mut app, &ctx);
    assert!(app.problems.error_count() > 0);
    let compiled = app
        .diagnostics
        .stats
        .compiled
        .load(std::sync::atomic::Ordering::Relaxed);
    assert!(
        compiled < 20,
        "typing must not compile every revision: {compiled}"
    );

    // A tab closed while its analysis is in flight leaves nothing behind.
    app.editor.new_tab("GONE.ppl", UNCLOSED_IF);
    app.editor.active_tab_index = 0;
    frame(&mut app, &ctx); // queues GONE.ppl at once (inactive tab)
    app.editor.close_tab(1);
    settle(&mut app, &ctx);
    assert!(app.problems.items.iter().all(|p| p.file != "GONE.ppl"));
}

#[test]
fn worker_coalesces_requests_and_never_reorders_a_tab() {
    let w = diagnostics::DiagnosticsWorker::new(None);
    let tab = crate::ide::editor::TabId(424_242);
    for rev in 1..=300u64 {
        w.request(
            tab,
            rev,
            "P.ppl",
            &format!("EXPORT P()\nBEGIN\nRETURN {rev};\nEND;\n"),
        );
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut seen = Vec::new();
    while seen.last() != Some(&300) {
        seen.extend(w.poll().into_iter().map(|r| r.revision));
        assert!(Instant::now() < deadline, "{seen:?}");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(seen.windows(2).all(|p| p[0] < p[1]), "{seen:?}");
    let compiled = w.stats.compiled.load(std::sync::atomic::Ordering::Relaxed);
    assert!(compiled < 300, "requests were not coalesced");
}

#[test]
fn compiler_panic_is_reported_as_analysis_failure() {
    let ctx = egui::Context::default();
    let mut app = app_with(&format!(
        "EXPORT P()\nBEGIN\nEND;\n// {}",
        diagnostics::PANIC_MARKER
    ));
    settle(&mut app, &ctx);
    assert!(
        matches!(app.state.analysis, AnalysisState::AnalysisFailed(_)),
        "{:?}",
        app.state.analysis
    );
    assert_eq!(app.problems.failures.len(), 1);
    assert!(!app.problems.title().ends_with("(0)"));
    // Run refuses to execute an unchecked program.
    app.handle_action(IdeAction::RunProgram, &ctx);
    assert!(!app.controller.worker_alive());
    assert!(console(&app).contains("Run blocked: internal compiler failure"));
    // The worker survived: the next revision is analysed normally.
    app.editor.active_tab_mut().unwrap().content = VALID.to_string();
    settle(&mut app, &ctx);
    assert_eq!(app.state.analysis, AnalysisState::Valid);
}

#[test]
fn run_never_executes_an_old_program_when_the_source_no_longer_compiles() {
    let ctx = egui::Context::default();
    let mut app = app_with("EXPORT GATE()\nBEGIN\n  PRINT(\"first\");\n  RETURN 1;\nEND;\n");
    app.handle_action(IdeAction::RunProgram, &ctx);
    finish_run(&mut app);
    assert!(console(&app).contains("first"));
    assert!(console(&app).contains("returned 1"));

    // Break the source (unsaved, no analysis frame in between) and Run.
    app.editor.active_tab_mut().unwrap().content =
        "EXPORT GATE()\nBEGIN\n  PRINT(\"second\");\n  a := ;\nEND;\n".to_string();
    let before = console(&app);
    app.handle_action(IdeAction::RunProgram, &ctx);
    assert!(!app.controller.worker_alive(), "a program was started");
    for _ in 0..20 {
        app.poll_execution();
        std::thread::sleep(Duration::from_millis(5));
    }
    let after = console(&app);
    let new_output = &after[before.len()..];
    assert!(new_output.contains("Run blocked"), "{new_output}");
    assert!(!new_output.contains("second"), "{new_output}");
    assert!(
        !new_output.contains("returned"),
        "old outcome re-reported: {new_output}"
    );
    assert_eq!(app.state.vm_state, crate::ide::VmExecutionState::Idle);
    assert!(matches!(app.state.analysis, AnalysisState::Invalid { .. }));
    assert_eq!(app.state.active_dock_tab, DockTab::Problems);
    // The cursor goes to the first error: undeclared `a` (line 4, col 3;
    // the missing value at col 8 is the second error).
    let errors: Vec<_> = app
        .problems
        .items
        .iter()
        .filter(|p| p.severity == Severity::Error)
        .map(|p| (p.code.clone(), p.line, p.col))
        .collect();
    assert_eq!(
        errors,
        vec![
            ("undeclared-identifier".to_string(), 4, 3),
            ("expected-expression".to_string(), 4, 8)
        ]
    );
    let tab = app.editor.active_tab().unwrap();
    let jump = tab.pending_jump.expect("cursor moved to the error");
    assert_eq!(
        &tab.content.chars().skip(jump).take(4).collect::<String>(),
        "a :="
    );

    // Fixed source runs, and it is the new program that runs.
    app.editor.active_tab_mut().unwrap().content =
        "EXPORT GATE()\nBEGIN\n  PRINT(\"third\");\n  RETURN 3;\nEND;\n".to_string();
    app.handle_action(IdeAction::RunProgram, &ctx);
    finish_run(&mut app);
    let tail = console(&app)[after.len()..].to_string();
    assert!(
        tail.contains("third") && tail.contains("returned 3"),
        "{tail}"
    );
    assert_eq!(app.state.analysis, AnalysisState::Valid);
}

#[test]
fn run_compiles_once_and_the_background_does_not_repeat_it() {
    let ctx = egui::Context::default();
    let mut app = app_with("EXPORT ONCE()\nBEGIN\n  RETURN 7;\nEND;\n");
    app.handle_action(IdeAction::RunProgram, &ctx);
    finish_run(&mut app);
    for _ in 0..10 {
        frame(&mut app, &ctx);
    }
    assert_eq!(app.state.analysis, AnalysisState::Valid);
    let compiled = app
        .diagnostics
        .stats
        .compiled
        .load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(compiled, 0, "the revision Run compiled was compiled again");
}

#[test]
fn runtime_errors_are_runtime_problems_until_the_source_changes() {
    let ctx = egui::Context::default();
    let mut app = app_with("EXPORT RT()\nBEGIN\n  LOCAL z := 0;\n  RETURN 1/z;\nEND;\n");
    app.handle_action(IdeAction::RunProgram, &ctx);
    finish_run(&mut app);
    let runtime: Vec<_> = app
        .problems
        .items
        .iter()
        .filter(|p| p.class == DiagnosticClass::Runtime)
        .collect();
    assert_eq!(runtime.len(), 1, "{:#?}", app.problems.items);
    assert_eq!(runtime[0].line, 4);
    assert_eq!(app.state.analysis, AnalysisState::Valid, "compiled fine");

    // Editing the source retires the runtime problem (its location
    // belonged to the text that ran).
    app.editor.active_tab_mut().unwrap().content =
        "EXPORT RT()\nBEGIN\n  LOCAL z := 0;\n  RETURN 1/z; // changed\nEND;\n".to_string();
    frame(&mut app, &ctx);
    assert!(
        app.problems
            .items
            .iter()
            .all(|p| p.class != DiagnosticClass::Runtime),
        "{:#?}",
        app.problems.items
    );
}

#[test]
fn compatibility_warnings_allow_running_and_stay_distinct() {
    let ctx = egui::Context::default();
    let mut app = app_with("EXPORT EV()\nBEGIN\n  RETURN EVAL(\"1+1\");\nEND;\n");
    settle(&mut app, &ctx);
    assert_eq!(app.state.analysis, AnalysisState::ValidWithWarnings(1));
    assert_eq!(app.problems.error_count(), 0);
    assert!(
        app.problems
            .items
            .iter()
            .all(|p| p.class == DiagnosticClass::Compatibility)
    );
    app.handle_action(IdeAction::RunProgram, &ctx);
    finish_run(&mut app);
    // It ran (and failed at run time on the unemulated command).
    assert!(
        app.problems
            .items
            .iter()
            .any(|p| p.class == DiagnosticClass::Runtime)
    );
    assert!(
        app.problems
            .items
            .iter()
            .any(|p| p.class == DiagnosticClass::Compatibility)
    );
}

#[test]
fn clicking_a_problem_moves_the_cursor_there() {
    let ctx = egui::Context::default();
    let mut app = app_with(VALID);
    app.editor.new_tab("BROKEN.ppl", MISSING_VALUE);
    app.editor.active_tab_index = 0;
    settle(&mut app, &ctx);
    let broken = app.editor.tabs[1].id;

    // Real frames of the Problems panel; then a pointer click on row 0.
    let mut state = std::mem::take(&mut app.state);
    let mut render = |app: &mut IdeDesktopApp, events: Vec<egui::Event>| {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 400.0),
            )),
            events,
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.problems.render(ui, &mut state));
        });
    };
    render(&mut app, vec![]);
    let row = *app.problems.row_rects.first().expect("a rendered row");
    let at = row.center();
    render(&mut app, vec![egui::Event::PointerMoved(at)]);
    let button = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    render(&mut app, vec![button(true)]);
    render(&mut app, vec![button(false)]);
    let actions = state.take_actions();
    app.state = state;
    let nav = actions
        .into_iter()
        .find(|a| matches!(a, IdeAction::NavigateToLocation { .. }))
        .expect("click navigates");
    assert_eq!(
        nav,
        IdeAction::NavigateToLocation {
            tab: Some(broken),
            line: 4,
            col: 8
        }
    );
    app.handle_action(nav, &ctx);
    assert_eq!(app.editor.active_tab_index, 1);
    // Line 4 col 8 of MISSING_VALUE: the `;` after `a := `.
    let jump = app.editor.tabs[1].pending_jump.expect("jump");
    assert_eq!(MISSING_VALUE.chars().nth(jump), Some(';'));
}

#[test]
fn problems_are_grouped_by_file_and_closed_files_drop_out() {
    let ctx = egui::Context::default();
    let mut app = app_with(MISSING_VALUE);
    app.editor.new_tab("SECOND.ppl", UNCLOSED_IF);
    settle(&mut app, &ctx);
    let files: Vec<&str> = app.problems.items.iter().map(|p| p.file.as_str()).collect();
    let first_second = files.iter().position(|f| *f == "SECOND.ppl").unwrap();
    assert!(files[..first_second].iter().all(|f| *f != "SECOND.ppl"));
    assert!(files[first_second..].iter().all(|f| *f == "SECOND.ppl"));
    app.handle_action(IdeAction::CloseActiveTab, &ctx);
    settle(&mut app, &ctx);
    assert!(app.problems.items.iter().all(|p| p.file != "SECOND.ppl"));
}

#[test]
fn baseconv_runs_through_the_ide_with_typed_input() {
    use crate::dialog::KeyPress;
    use crate::keymap::{KEY_ENTER, KEY_ESC};
    let ctx = egui::Context::default();
    let src = include_str!("../../../tests/fixtures/baseconv/Baseconv_fixed.hpprgm");
    let mut app = app_with(src);
    settle(&mut app, &ctx);
    assert_eq!(app.state.analysis, AnalysisState::Valid);

    let type_answers = |app: &IdeDesktopApp, answers: &[&str]| {
        for a in answers {
            for c in a.chars() {
                app.controller
                    .shared
                    .push_key_press(KeyPress::with_char(None, c));
            }
            app.controller.shared.push_key(KEY_ENTER);
        }
    };
    // Run 1: a typed hex string.
    app.handle_action(IdeAction::RunProgram, &ctx);
    type_answers(&app, &["FF", "16", "10", "10"]);
    finish_run(&mut app);
    assert!(
        console(&app).contains("Final Result: 255"),
        "{}",
        console(&app)
    );

    // Run 2: Esc in the second dialog ends the program normally.
    let before = console(&app).len();
    app.handle_action(IdeAction::RunProgram, &ctx);
    type_answers(&app, &["7B.8"]);
    app.controller.shared.push_key(KEY_ESC);
    finish_run(&mut app);
    let tail = console(&app)[before..].to_string();
    assert!(tail.contains("returned 0"), "{tail}");
    assert!(!tail.contains("Final Result"), "{tail}");

    // Run 3: the program runs again from a clean state.
    let before = console(&app).len();
    app.handle_action(IdeAction::RunProgram, &ctx);
    type_answers(&app, &["7B.8", "16", "2", "10"]);
    finish_run(&mut app);
    let tail = console(&app)[before..].to_string();
    assert!(tail.contains("Final Result: 01111011.1000"), "{tail}");
}

#[test]
fn baseconv_original_source_defect_is_a_runtime_problem() {
    use crate::dialog::KeyPress;
    use crate::keymap::KEY_ENTER;
    let ctx = egui::Context::default();
    let src = include_str!("../../../tests/fixtures/baseconv/Baseconv_original.hpprgm");
    let mut app = app_with(src);
    settle(&mut app, &ctx);
    // The original compiles: its defects are run-time ones.
    assert_eq!(app.state.analysis, AnalysisState::Valid);
    app.handle_action(IdeAction::RunProgram, &ctx);
    // Typing "FF" (with quotes) stores a string; temp(1) is then a
    // character code and UPPER rejects it.
    for c in "\"FF\"".chars() {
        app.controller
            .shared
            .push_key_press(KeyPress::with_char(None, c));
    }
    app.controller.shared.push_key(KEY_ENTER);
    finish_run(&mut app);
    let runtime: Vec<_> = app
        .problems
        .items
        .iter()
        .filter(|p| p.class == DiagnosticClass::Runtime)
        .collect();
    assert_eq!(runtime.len(), 1, "{:#?}", app.problems.items);
    assert_eq!(runtime[0].line, 16);
    assert!(
        runtime[0].message.contains("UPPER"),
        "{}",
        runtime[0].message
    );
}

#[test]
fn error_recovery_never_hangs_on_partial_source() {
    // Every prefix of a real program (as if typed or deleted from the end)
    // analyses quickly, without panicking, and the full text is clean.
    let src = include_str!("../../../tests/fixtures/baseconv/Baseconv_fixed.hpprgm");
    let started = Instant::now();
    let mut longest = Duration::ZERO;
    let ends: Vec<usize> = src
        .char_indices()
        .map(|(i, _)| i)
        .chain([src.len()])
        .step_by(3)
        .chain([src.len()])
        .collect();
    for end in ends {
        let t = Instant::now();
        let a = diagnostics::analyze("B.ppl", &src[..end], 1);
        longest = longest.max(t.elapsed());
        assert!(a.failure.is_none(), "panic at prefix {end}");
    }
    assert!(diagnostics::analyze("B.ppl", src, 1).items.is_empty());
    assert!(
        longest < Duration::from_secs(1),
        "slowest prefix took {longest:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(60));
}

/// Generated program of about `funcs * 18` lines.
fn big_program(funcs: usize) -> String {
    let mut s = String::from("EXPORT MAIN()\nBEGIN\n  RETURN 0;\nEND;\n");
    for i in 0..funcs {
        s.push_str(&format!(
            "F{i}(a)\nBEGIN\n  LOCAL i, t := \"x\";\n  FOR i FROM 1 TO 10 DO\n    \
             IF a > i THEN t := t + STRING(i); ELSE t := UPPER(t); END;\n  END;\n  \
             WHILE a > 0 DO a := a - 1; END;\n  RETURN DIM(t);\nEND;\n"
        ));
    }
    s
}

/// Measurements for the report: `cargo test --release -p ppl-gui
/// measure_analysis_latency -- --ignored --nocapture`.
#[test]
#[ignore]
fn measure_analysis_latency() {
    let baseconv = include_str!("../../../tests/fixtures/baseconv/Baseconv_fixed.hpprgm");
    for (name, src) in [
        ("Baseconv", baseconv.to_string()),
        ("500 functions", big_program(500)),
        ("2000 functions", big_program(2000)),
    ] {
        let mut best = Duration::MAX;
        for _ in 0..5 {
            let t = Instant::now();
            let a = diagnostics::analyze("P.ppl", &src, 1);
            best = best.min(t.elapsed());
            assert!(a.failure.is_none());
        }
        println!(
            "analyze {name}: {} lines, {:.2} ms",
            src.lines().count(),
            best.as_secs_f64() * 1e3
        );
    }

    // Edit → Problems updated (includes the 250 ms typing pause).
    let ctx = egui::Context::default();
    let mut app = app_with(VALID);
    settle(&mut app, &ctx);
    app.editor.active_tab_mut().unwrap().content = MISSING_VALUE.to_string();
    let t = Instant::now();
    settle(&mut app, &ctx);
    println!(
        "edit -> diagnostics shown: {:.0} ms (debounce {} ms)",
        t.elapsed().as_secs_f64() * 1e3,
        DIAGNOSTICS_DEBOUNCE.as_millis()
    );

    // UI-thread cost per frame while an 18k-line analysis is in flight.
    // The first frame also builds the editor's highlight cache for the
    // new buffer (done on open regardless of compilation); later frames
    // are pure scheduling while the worker compiles.
    let mut app = app_with(VALID);
    app.editor.new_tab("BIG.ppl", big_program(2000));
    app.editor.active_tab_index = 0;
    let mut worst = Duration::ZERO;
    let mut first = Duration::ZERO;
    let mut frames = 0;
    let t = Instant::now();
    while !app.problems.is_settled() || frames == 0 {
        let f = Instant::now();
        frame(&mut app, &ctx);
        if frames == 0 {
            first = f.elapsed();
        } else {
            worst = worst.max(f.elapsed());
        }
        frames += 1;
        std::thread::sleep(Duration::from_millis(1));
        assert!(t.elapsed() < Duration::from_secs(20));
    }
    println!(
        "UI frame during an 18k-line analysis: first (highlight cache) {:.2} ms, then worst {:.2} ms over {frames} frames; analysis done after {:.0} ms",
        first.as_secs_f64() * 1e3,
        worst.as_secs_f64() * 1e3,
        t.elapsed().as_secs_f64() * 1e3
    );

    // Rapid typing: compilations per revision.
    let stats = &app.diagnostics.stats;
    let before = stats.compiled.load(std::sync::atomic::Ordering::Relaxed);
    for i in 0..200 {
        app.editor.tabs[0].content = format!("EXPORT DEMO()\nBEGIN\n  RETURN {i};\nEND;\n");
        frame(&mut app, &ctx);
        std::thread::sleep(Duration::from_millis(2));
    }
    settle(&mut app, &ctx);
    let after = app
        .diagnostics
        .stats
        .compiled
        .load(std::sync::atomic::Ordering::Relaxed);
    println!(
        "200 edits at ~2 ms intervals: {} background compilations",
        after - before
    );
    println!(
        "bookkeeping after closing a tab: {} request entries for {} tabs",
        {
            app.editor.close_tab(1);
            settle(&mut app, &ctx);
            app.diag_requests.len()
        },
        app.editor.tabs.len()
    );
}
