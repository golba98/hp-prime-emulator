use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use ppl_syntax::Diagnostics;

fn bench(c: &mut Criterion) {
    let mut g = c.benchmark_group("lexer");
    for (name, file) in ppl_bench::frontend_inputs() {
        g.throughput(Throughput::Bytes(file.text.len() as u64));
        g.bench_function(name, |b| {
            b.iter(|| {
                let mut diags = Diagnostics::default();
                let toks = ppl_syntax::lexer::lex(black_box(&file.text), &mut diags);
                black_box((toks, diags))
            })
        });
    }
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
