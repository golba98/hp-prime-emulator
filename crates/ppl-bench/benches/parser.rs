use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};

fn bench(c: &mut Criterion) {
    let mut g = c.benchmark_group("parser");
    for (name, file) in ppl_bench::frontend_inputs() {
        g.throughput(Throughput::Bytes(file.text.len() as u64));
        g.bench_function(name, |b| {
            b.iter(|| black_box(ppl_syntax::parse(black_box(&file.text))))
        });
    }
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
