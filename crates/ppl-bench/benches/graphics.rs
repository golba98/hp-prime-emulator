use criterion::{BatchSize, Criterion, Throughput, black_box, criterion_group, criterion_main};
use ppl_bench::{GFX_BATCH, GRAPHICS_WORKLOADS, grob_fill_circles, grob_lines, grob_text};
use ppl_graphics::Grob;

type GrobCase = (&'static str, fn(&mut Grob));

fn bench_ppl(c: &mut Criterion) {
    let mut g = c.benchmark_group("graphics_ppl");
    g.throughput(Throughput::Elements(GFX_BATCH as u64));
    for w in GRAPHICS_WORKLOADS {
        let mut m = w.prepare();
        g.bench_function(w.name, |b| b.iter(|| black_box(w.run(&mut m))));
    }
    g.finish();
}

fn bench_grob(c: &mut Criterion) {
    let mut g = c.benchmark_group("graphics_grob");
    g.throughput(Throughput::Elements(GFX_BATCH as u64));
    let cases: [GrobCase; 3] = [
        ("grob_line_x200", grob_lines),
        ("grob_fill_circle_x200", grob_fill_circles),
        ("grob_draw_text_x200", grob_text),
    ];
    for (name, f) in cases {
        g.bench_function(name, |b| {
            b.iter_batched_ref(
                Grob::screen,
                |grob| {
                    f(grob);
                    black_box(grob.pixels().len())
                },
                BatchSize::SmallInput,
            )
        });
    }
    g.finish();
}

criterion_group!(benches, bench_ppl, bench_grob);
criterion_main!(benches);
