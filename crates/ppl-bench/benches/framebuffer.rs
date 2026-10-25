use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use ppl_graphics::{BLACK, Grob, SCREEN_H, SCREEN_W};
use ppl_gui::{ScaleFilter, Viewport, blit_filtered, blit_scaled};

fn bench_blit(c: &mut Criterion) {
    let frame = ppl_bench::test_frame();
    let mut g = c.benchmark_group("framebuffer");
    for (name, dw, dh, integer) in [
        ("blit_320x240_to_320x240_1x", 320usize, 240usize, true),
        ("blit_320x240_to_640x480_2x", 640, 480, true),
        ("blit_320x240_to_800x600_fit", 800, 600, false),
    ] {
        let vp = Viewport::calculate(dw as u32, dh as u32, integer);
        let mut dst = vec![0u32; dw * dh];
        g.throughput(Throughput::Elements((dw * dh) as u64));
        g.bench_function(name, |b| {
            b.iter(|| {
                blit_scaled(
                    black_box(frame.pixels()),
                    SCREEN_W,
                    SCREEN_H,
                    &mut dst,
                    dw,
                    dh,
                    &vp,
                    BLACK,
                );
                black_box(&dst);
            })
        });
    }
    for (name, dw, dh, filter) in [
        ("mirror_nearest_853x640", 853, 640, ScaleFilter::Nearest),
        ("mirror_linear_960x720", 960, 720, ScaleFilter::Linear),
    ] {
        let vp = Viewport::calculate_aspect_fit(dw as u32, dh as u32);
        let mut dst = vec![0; dw * dh];
        g.throughput(Throughput::Elements((dw * dh) as u64));
        g.bench_function(name, |b| {
            b.iter(|| {
                blit_filtered(
                    black_box(frame.pixels()),
                    SCREEN_W,
                    SCREEN_H,
                    &mut dst,
                    dw,
                    dh,
                    &vp,
                    BLACK,
                    filter,
                );
                black_box(&dst);
            })
        });
    }
    g.throughput(Throughput::Bytes((SCREEN_W * SCREEN_H * 4) as u64));
    g.bench_function("grob_clone_320x240", |b| {
        b.iter(|| black_box(black_box(&frame).clone()))
    });
    let mut target = Grob::screen();
    g.bench_function("grob_clone_from_320x240", |b| {
        b.iter(|| {
            target.clone_from(black_box(&frame));
            black_box(&target);
        })
    });
    g.finish();
}

criterion_group!(benches, bench_blit);
criterion_main!(benches);
