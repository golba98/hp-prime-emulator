use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use ppl_bench::{FIB, FIB_CALLS, FOR_SUM, FOR_SUM_N, REAL_CHAIN, REAL_CHAIN_N};

/// Each workload's throughput unit: loop iterations, or user-function calls
/// for fib.
fn bench(c: &mut Criterion) {
    let mut g = c.benchmark_group("vm");
    for (w, elems) in [
        (FOR_SUM, FOR_SUM_N),
        (FIB, FIB_CALLS),
        (REAL_CHAIN, REAL_CHAIN_N),
    ] {
        g.throughput(Throughput::Elements(elems));
        let mut m = w.prepare();
        g.bench_function(w.name, |b| b.iter(|| black_box(w.run(&mut m))));
    }
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
