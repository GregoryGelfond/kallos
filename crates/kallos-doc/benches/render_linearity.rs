//! The linearity invariant as a criterion regression guard: `render` is O(n) in
//! document size (N-sweep) and independent of target width (width-sweep). Pure
//! `kallos-doc` public API. Run: `cargo bench -p kallos-doc`.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use kallos_doc::{render, Doc, DocBuilder};
use std::hint::black_box;

/// A flat wide `group(seq(leaf, line, leaf, line, …))` of `n` leaves over one
/// backing source — the size-sweep fixture.
fn wide_doc(n: usize) -> (Doc, String) {
    let mut b = DocBuilder::new();
    let src = "a ".repeat(n);
    let mut items = Vec::with_capacity(n * 2);
    for i in 0..n {
        let leaf = b.leaf(u32::try_from(i * 2).expect("offset fits u32"), "a");
        items.push(leaf);
        let line = b.line();
        items.push(line);
    }
    let seq = b.seq(&items);
    let group = b.group(seq);
    (b.finish(group), src)
}

fn bench_render(c: &mut Criterion) {
    let mut n_sweep = c.benchmark_group("render_n_sweep");
    for &n in &[100usize, 1000, 10_000] {
        let (doc, src) = wide_doc(n);
        n_sweep.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| render(black_box(&doc), black_box(&src), 100));
        });
    }
    n_sweep.finish();

    let mut width_sweep = c.benchmark_group("render_width_sweep");
    let (doc, src) = wide_doc(2000);
    for &w in &[20usize, 100, 1000] {
        width_sweep.bench_with_input(BenchmarkId::from_parameter(w), &w, |b, &w| {
            b.iter(|| render(black_box(&doc), black_box(&src), w));
        });
    }
    width_sweep.finish();
}

criterion_group!(benches, bench_render);
criterion_main!(benches);
