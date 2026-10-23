//! Editor behavior through real egui frames: synthetic keyboard, pointer,
//! window-size and scale-factor input drives the production
//! [`EditorArea`] exactly as eframe would, without a window.

use egui::text::{CCursor, CCursorRange};
use egui::text_edit::TextEditState;
use egui::{Event, Key, Modifiers, Pos2, Rect, Vec2, pos2, vec2};

use super::view::edit_id;
use super::{EditorArea, FindCommand, TabId};
use crate::ide::{IdeAction, IdeAppState, IdePanel};

struct Harness {
    ctx: egui::Context,
    area: EditorArea,
    state: IdeAppState,
    size: Vec2,
    ppp: f32,
    time: f64,
    actions: Vec<IdeAction>,
    last: Option<egui::FullOutput>,
}

impl Harness {
    fn new(src: &str) -> Self {
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let mut area = EditorArea::new();
        area.tabs.clear();
        area.new_tab("TEST.ppl", src);
        let mut h = Harness {
            ctx,
            area,
            state: IdeAppState::default(),
            size: vec2(1280.0, 800.0),
            ppp: 1.0,
            time: 0.0,
            actions: Vec::new(),
            last: None,
        };
        h.frame(vec![]);
        h
    }

    fn frame(&mut self, events: Vec<Event>) {
        let mut raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            events,
            time: Some(self.time),
            focused: true,
            ..Default::default()
        };
        raw.viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .native_pixels_per_point = Some(self.ppp);
        self.time += 1.0 / 60.0;
        let (area, state) = (&mut self.area, &mut self.state);
        let out = self.ctx.run(raw, |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| area.render(ui, state));
        });
        self.actions.extend(self.state.take_actions());
        self.last = Some(out);
    }

    /// Runs half a second of frames (egui animates scrolling).
    fn settle(&mut self) {
        for _ in 0..30 {
            self.frame(vec![]);
        }
    }

    fn tab_id(&self) -> TabId {
        self.area.active_tab().unwrap().id
    }

    fn focus(&mut self) {
        let id = edit_id(self.tab_id());
        self.ctx.memory_mut(|m| m.request_focus(id));
        self.frame(vec![]);
        self.frame(vec![]);
    }

    fn key(&mut self, key: Key, modifiers: Modifiers) {
        let ev = |pressed| Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        };
        self.frame(vec![ev(true)]);
        self.frame(vec![ev(false)]);
    }

    fn type_text(&mut self, s: &str) {
        for ch in s.chars() {
            match ch {
                '\n' => self.key(Key::Enter, Modifiers::NONE),
                '\t' => self.key(Key::Tab, Modifiers::NONE),
                c => self.frame(vec![Event::Text(c.to_string())]),
            }
        }
    }

    fn text(&self) -> &str {
        &self.area.active_tab().unwrap().content
    }

    fn selection(&self) -> (usize, usize) {
        let st = TextEditState::load(&self.ctx, edit_id(self.tab_id())).unwrap();
        let r = st.cursor.char_range().unwrap();
        let [a, b] = r.sorted_cursors();
        (a.index, b.index)
    }

    fn select(&mut self, a: usize, b: usize) {
        let id = edit_id(self.tab_id());
        let mut st = TextEditState::load(&self.ctx, id).unwrap_or_default();
        st.cursor
            .set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
        st.store(&self.ctx, id);
        self.frame(vec![]);
    }

    fn undo(&mut self) {
        self.key(Key::Z, Modifiers::COMMAND);
    }

    fn redo(&mut self) {
        self.key(Key::Y, Modifiers::COMMAND);
    }

    fn info(&self) -> super::view::FrameInfo {
        self.area
            .view
            .last_frame
            .clone()
            .expect("editor drew a frame")
    }

    fn click(&mut self, at: Pos2) {
        let btn = |pressed| Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        self.frame(vec![Event::PointerMoved(at)]);
        self.frame(vec![btn(true)]);
        self.frame(vec![btn(false)]);
    }
}

fn program(lines: usize) -> String {
    let mut s = String::from("EXPORT BIG()\nBEGIN\n");
    for i in 0..lines {
        s.push_str(&format!("  PRINT({i});\n"));
    }
    s.push_str("END;\n");
    s
}

#[test]
fn tab_indents_a_multi_line_selection_without_losing_text() {
    // Regression: egui's TextEdit replaced the selection with "\t" before
    // the IDE's own indent ran on the stale range.
    let mut h = Harness::new("AAA\nBBB\nCCC");
    h.focus();
    h.select(1, 5);
    h.key(Key::Tab, Modifiers::NONE);
    assert_eq!(h.text(), "  AAA\n  BBB\nCCC");
    assert_eq!(h.selection(), (3, 9));
    h.key(Key::Tab, Modifiers::SHIFT);
    assert_eq!(h.text(), "AAA\nBBB\nCCC");
    // Each operation is exactly one undo step.
    h.undo();
    assert_eq!(h.text(), "  AAA\n  BBB\nCCC");
    h.undo();
    assert_eq!(h.text(), "AAA\nBBB\nCCC");
    h.redo();
    assert_eq!(h.text(), "  AAA\n  BBB\nCCC");
}

#[test]
fn enter_indents_blocks_and_aligns_closers() {
    let mut h = Harness::new("");
    h.focus();
    h.type_text("EXPORT F()\nBEGIN\nIF 1 THEN\nPRINT(1);\nEND;\nEND;");
    // Each END was re-aligned to its block when Enter finished its line;
    // the last one is still on the line being typed.
    assert_eq!(
        h.text(),
        "EXPORT F()\nBEGIN\n  IF 1 THEN\n    PRINT(1);\n  END;\n  END;"
    );
    h.key(Key::Enter, Modifiers::NONE);
    assert_eq!(
        h.text(),
        "EXPORT F()\nBEGIN\n  IF 1 THEN\n    PRINT(1);\n  END;\nEND;\n"
    );
}

#[test]
fn brackets_and_quotes_pair_and_step_over() {
    let mut h = Harness::new("");
    h.focus();
    h.type_text("PRINT(");
    assert_eq!(h.text(), "PRINT()");
    assert_eq!(h.selection(), (6, 6));
    h.type_text("\"hi");
    assert_eq!(h.text(), "PRINT(\"hi\")");
    h.type_text("\")");
    assert_eq!(h.text(), "PRINT(\"hi\")", "closers are stepped over");
    assert_eq!(h.selection(), (11, 11));
    // Backspace inside an empty pair removes both characters.
    h.type_text("{");
    assert_eq!(h.text(), "PRINT(\"hi\"){}");
    h.key(Key::Backspace, Modifiers::NONE);
    assert_eq!(h.text(), "PRINT(\"hi\")");
}

#[test]
fn toggle_comment_with_ctrl_slash() {
    let mut h = Harness::new("BEGIN\n  x := 1;\nEND;");
    h.focus();
    h.select(6, 15);
    h.key(Key::Slash, Modifiers::COMMAND);
    assert_eq!(h.text(), "BEGIN\n  // x := 1;\nEND;");
    h.undo();
    assert_eq!(h.text(), "BEGIN\n  x := 1;\nEND;");
}

#[test]
fn completion_offers_documented_names_and_accepts_with_enter() {
    let mut h = Harness::new("EXPORT MYFUNC()\nBEGIN\n  \nEND;");
    h.focus();
    h.select(24, 24);
    h.type_text("PRI");
    let c = &h.area.view.completion;
    assert!(c.open, "popup opens after two identifier characters");
    assert_eq!(c.items[0].label, "PRINT");
    h.key(Key::Enter, Modifiers::NONE);
    assert_eq!(h.text(), "EXPORT MYFUNC()\nBEGIN\n  PRINT\nEND;");
    assert!(!h.area.view.completion.open);
    // User functions are offered too.
    h.type_text(";\n  MYF");
    assert_eq!(h.area.view.completion.items[0].label, "MYFUNC");
    h.key(Key::Escape, Modifiers::NONE);
    assert!(!h.area.view.completion.open);
}

#[test]
fn each_tab_keeps_its_own_cursor_and_undo_history() {
    let mut h = Harness::new("AAAA");
    h.area.new_tab("B.ppl", "BBBB");
    h.frame(vec![]);
    h.focus();
    h.select(4, 4);
    h.type_text("b");
    assert_eq!(h.text(), "BBBBb");

    h.area.active_tab_index = 0;
    h.frame(vec![]);
    h.focus();
    h.select(2, 2);
    h.type_text("a");
    assert_eq!(h.text(), "AAaAA");

    // Back to B: its cursor and history are its own.
    h.area.active_tab_index = 1;
    h.frame(vec![]);
    h.focus();
    assert_eq!(h.selection(), (5, 5));
    h.undo();
    assert_eq!(h.text(), "BBBB");
    assert_eq!(h.area.tabs[0].content, "AAaAA", "undo never crosses tabs");

    // Closing the first tab must not hand its state to the second.
    h.area.close_tab(0);
    h.frame(vec![]);
    h.focus();
    assert_eq!(h.text(), "BBBB");
    assert_eq!(h.selection(), (4, 4));
}

#[test]
fn resizing_and_rescaling_keep_cursor_scroll_and_layout_cache() {
    let src = program(1000);
    let mut h = Harness::new(&src);
    h.focus();
    h.area.active_tab_mut().unwrap().goto_line(600, 3);
    h.settle();
    let cursor = h.selection();
    let line_of = |h: &Harness| {
        let i = h.info();
        i.visible_lines.clone()
    };
    assert!(
        line_of(&h).contains(&599),
        "jump scrolled line 600 into view"
    );
    let layouts_before = h.area.tabs[0].doc.stats.layouts;
    let analyses_before = h.area.tabs[0].doc.stats.analyses;

    for size in [
        vec2(800.0, 500.0),
        vec2(1920.0, 1080.0),
        vec2(2560.0, 1440.0),
        vec2(3840.0, 2160.0),
        vec2(1024.0, 640.0),
    ] {
        h.size = size;
        h.settle();
        let info = h.info();
        assert!(info.clip.width() > 0.0 && info.clip.max.x <= size.x + 0.5);
        assert!(info.clip.max.y <= size.y + 0.5);
        assert_eq!(h.selection(), cursor, "cursor survives resize to {size:?}");
        assert!(
            info.visible_lines.contains(&599),
            "line 600 stays visible at {size:?}"
        );
    }
    assert_eq!(
        h.area.tabs[0].doc.stats.layouts, layouts_before,
        "a window resize never re-lays out code"
    );

    // A scale-factor change re-rasterizes text but keeps everything else.
    for ppp in [2.0, 1.25, 1.5] {
        h.ppp = ppp;
        h.settle();
        assert_eq!(h.selection(), cursor);
        assert!(
            h.info().visible_lines.contains(&599),
            "visible at scale {ppp}"
        );
    }
    assert_eq!(h.area.tabs[0].doc.stats.analyses, analyses_before);
    assert_eq!(h.text(), src);
}

#[test]
fn gutter_click_toggles_breakpoint_on_that_line() {
    let mut h = Harness::new("EXPORT F()\nBEGIN\n  PRINT(1);\nEND;");
    h.frame(vec![]);
    let info = h.info();
    let y = info.galley_pos.y + info.row_height * 2.5;
    h.click(pos2(info.clip.min.x + 6.0, y));
    assert_eq!(h.actions, vec![IdeAction::ToggleBreakpoint(3)]);
}

#[test]
fn current_line_band_is_painted_behind_the_text() {
    // Regression: the band was painted after the text and hid it.
    let mut h = Harness::new("EXPORT F()\nBEGIN\nEND;");
    h.focus();
    h.select(12, 12);
    let out = h.last.take().unwrap();
    let band = crate::theme::editor::CURRENT_LINE;
    let mut band_at = None;
    let mut text_at = None;
    for (i, clipped) in out.shapes.iter().enumerate() {
        let mut visit = |s: &egui::Shape| match s {
            egui::Shape::Rect(r) if r.fill == band => {
                band_at.get_or_insert(i);
            }
            egui::Shape::Text(t) if t.galley.text().contains("BEGIN") => {
                text_at.get_or_insert(i);
            }
            _ => {}
        };
        match &clipped.shape {
            egui::Shape::Vec(v) => v.iter().for_each(&mut visit),
            s => visit(s),
        }
    }
    let (band_at, text_at) = (
        band_at.expect("band painted"),
        text_at.expect("text painted"),
    );
    assert!(
        band_at < text_at,
        "band {band_at} must come before text {text_at}"
    );
}

#[test]
fn find_selects_matches_and_replace_all_is_one_undo_step() {
    let mut h = Harness::new("PRINT(1);\nprint(2);\nPRINT(3);");
    h.focus();
    h.area.find.open(true, Some("PRINT".into()));
    h.area.find.query.match_case = true;
    let ctx = h.ctx.clone();
    h.area.run_find(&ctx, FindCommand::Next);
    h.frame(vec![]);
    assert_eq!(h.selection(), (0, 5));
    h.area.run_find(&ctx, FindCommand::Next);
    h.frame(vec![]);
    assert_eq!(h.selection(), (20, 25), "case-sensitive: skips `print`");
    h.area.find.replacement = "DISP".into();
    h.area.run_find(&ctx, FindCommand::ReplaceAll);
    h.frame(vec![]);
    assert_eq!(h.text(), "DISP(1);\nprint(2);\nDISP(3);");
    h.focus();
    h.undo();
    assert_eq!(h.text(), "PRINT(1);\nprint(2);\nPRINT(3);");
}

#[test]
fn paused_line_scrolls_into_view() {
    let mut h = Harness::new(&program(800));
    h.frame(vec![]);
    assert!(!h.info().visible_lines.contains(&699));
    h.state.paused_line = 700;
    h.settle();
    assert!(h.info().visible_lines.contains(&699));
}

#[test]
fn large_buffer_frames_reuse_analysis_and_layout() {
    let mut h = Harness::new(&program(20_000));
    h.focus();
    let (a, l) = {
        let s = h.area.tabs[0].doc.stats;
        (s.analyses, s.layouts)
    };
    for _ in 0..10 {
        h.frame(vec![]);
    }
    let s = h.area.tabs[0].doc.stats;
    assert_eq!(
        (s.analyses, s.layouts),
        (a, l),
        "idle frames do no language work"
    );
    h.type_text("x");
    let s = h.area.tabs[0].doc.stats;
    assert_eq!(s.analyses, a + 1, "one keystroke, one analysis");
}

#[test]
fn a_burst_of_input_in_one_frame_keeps_order_and_smart_keys() {
    // Fast typing, key repeat or scripted input can deliver many events in
    // one frame; they must apply in order with auto-indent and pairing.
    let mut h = Harness::new("");
    h.focus();
    let key = |key| Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    };
    let mut events = Vec::new();
    for ch in "BEGIN".chars() {
        events.push(Event::Text(ch.to_string()));
    }
    events.push(key(Key::Enter));
    for ch in "PRINT(\"a\");".chars() {
        events.push(Event::Text(ch.to_string()));
    }
    events.push(key(Key::Enter));
    for ch in "x".chars() {
        events.push(Event::Text(ch.to_string()));
    }
    events.push(key(Key::Backspace));
    h.frame(events);
    assert_eq!(h.text(), "BEGIN\n  PRINT(\"a\");\n  ");
    assert_eq!(h.selection(), (22, 22));
}
