//! Bench harness. One placeholder bench so `cargo bench --no-run`
//! compiles; real cases go here once there's something to measure.

use criterion::{criterion_group, criterion_main, Criterion};

fn noop(c: &mut Criterion) {
    c.bench_function("noop", |b| b.iter(|| 1 + 1));
}

criterion_group!(benches, noop);
criterion_main!(benches);
