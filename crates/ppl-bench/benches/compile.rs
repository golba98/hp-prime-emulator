use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use ppl_compiler::{CompileOptions, compile_source};

fn bench(c: &mut Criterion) {
    let opts = CompileOptions::default();
    let mut g = c.benchmark_group("compile");
    for (name, file) in ppl_bench::frontend_inputs() {
        g.throughput(Throughput::Bytes(file.text.len() as u64));
        g.bench_function(name, |b| {
            b.iter(|| black_box(compile_source(black_box(&file), &opts)))
        });
    }
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
