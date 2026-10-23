//! IDE editor costs on large programs: syntax highlighting, live
//! diagnostics (full compile) and a complete headless egui frame of the
//! editor panel (layout + tessellation), steady state and after an edit.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use ppl_compiler::{CompileOptions, compile_source};
use ppl_gui::ide::editor::EditorArea;
use ppl_gui::ide::{IdeAppState, IdePanel};
use ppl_syntax::SourceFile;

/// Synthetic source of roughly `lines` lines.
fn source_with_lines(lines: usize) -> String {
    // Each synthetic function is ~27 lines / ~700 bytes.
    let text = ppl_bench::synthetic_source_text(lines * 26);
    assert!(text.lines().count() >= lines / 2);
    text
}

fn frame(ctx: &egui::Context, editor: &mut EditorArea, state: &mut IdeAppState) {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1920.0, 1080.0),
        )),
        ..Default::default()
    };
    let out = ctx.run(input, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| editor.render(ui, state));
    });
    black_box(ctx.tessellate(out.shapes, out.pixels_per_point));
}

fn bench(c: &mut Criterion) {
    let mut g = c.benchmark_group("editor");
    g.sample_size(10);
    for lines in [1_000usize, 5_000, 20_000] {
        let text = source_with_lines(lines);
        let real_lines = text.lines().count();

        g.bench_function(format!("highlight_{real_lines}_lines"), |b| {
            b.iter(|| black_box(ppl_gui::ide::syntax::highlight(black_box(&text))))
        });

        let file = SourceFile::new("<bench>", text.clone());
        g.bench_function(format!("diagnostics_{real_lines}_lines"), |b| {
            b.iter(|| black_box(compile_source(&file, &CompileOptions::default())))
        });

        let ctx = egui::Context::default();
        let mut state = IdeAppState::default();
        let mut editor = EditorArea::new();
        editor.new_tab("BIG.ppl", text.clone());
        // Warm up fonts and caches.
        frame(&ctx, &mut editor, &mut state);
        frame(&ctx, &mut editor, &mut state);
        g.bench_function(format!("frame_steady_{real_lines}_lines"), |b| {
            b.iter(|| frame(&ctx, &mut editor, &mut state))
        });
        let mut n = 0u64;
        g.bench_function(format!("frame_after_edit_{real_lines}_lines"), |b| {
            b.iter(|| {
                n += 1;
                // A one-character edit near the top of the buffer.
                let tab = editor.active_tab_mut().unwrap();
                let at = tab.content.find('\n').unwrap();
                tab.content.insert(at, if n % 2 == 0 { ' ' } else { 'x' });
                frame(&ctx, &mut editor, &mut state)
            })
        });
    }
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
