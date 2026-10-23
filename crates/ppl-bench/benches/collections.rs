use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use ppl_bench::{DET_20, LIST_APPEND_10K, LIST_N, MAKELIST_10K, MATMUL_20};

fn bench(c: &mut Criterion) {
    let mut g = c.benchmark_group("collections");
    for (w, elems) in [
        (MAKELIST_10K, Some(LIST_N)),
        (LIST_APPEND_10K, Some(LIST_N)),
        (MATMUL_20, None),
        (DET_20, None),
    ] {
        match elems {
            Some(n) => g.throughput(Throughput::Elements(n)),
            None => g.throughput(Throughput::Elements(1)),
        };
        let mut m = w.prepare();
        g.bench_function(w.name, |b| b.iter(|| black_box(w.run(&mut m))));
    }
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
