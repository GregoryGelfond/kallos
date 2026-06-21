//! End-to-end formatter scaling guards: the pipeline is linear in input
//! size. Two fixtures — a deep bracket nest and a wide argument list; a
//! deep-operator-chain fixture joins when operator-chain layout lands.
//! Run: `cargo bench -p kallos`.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use kallos::{format, Style};
use std::hint::black_box;

fn deep_bracket_nest(n: usize) -> String {
    format!("p({}0{}).\n", "f(".repeat(n), ")".repeat(n))
}

fn wide_arg_list(n: usize) -> String {
    let args: Vec<String> = (0..n).map(|i| format!("a{i}")).collect();
    format!("p({}).\n", args.join(", "))
}

fn bench_pipeline(c: &mut Criterion) {
    // House default (line width 100) — built once, matching the prior bench width.
    let style = Style::default().with_line_width(100);
    let mut group = c.benchmark_group("pipeline");
    for &n in &[50usize, 500, 5000] {
        let nest = deep_bracket_nest(n);
        group.throughput(Throughput::Bytes(
            u64::try_from(nest.len()).expect("fits u64"),
        ));
        group.bench_with_input(BenchmarkId::new("deep_bracket_nest", n), &nest, |b, src| {
            b.iter(|| format(black_box(src), &style));
        });

        let wide = wide_arg_list(n);
        group.throughput(Throughput::Bytes(
            u64::try_from(wide.len()).expect("fits u64"),
        ));
        group.bench_with_input(BenchmarkId::new("wide_arg_list", n), &wide, |b, src| {
            b.iter(|| format(black_box(src), &style));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_pipeline);
criterion_main!(benches);
