//! A whole frame: `qry::views::draw` for the full 160x48 screen with the
//! theme on, so the chrome (top bar, panes, status line, key hints) is
//! included. Drawn through `AppHarness::render_buffer` into a `TestBackend`.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};

#[path = "app.rs"]
mod app;
#[path = "common.rs"]
mod common;

const W: u16 = 160;
const H: u16 = 48;

fn full_frame(c: &mut Criterion) {
    let mut group = c.benchmark_group(format!("full_frame_{W}x{H}"));

    let mut idle = app::harness();
    group.bench_function("idle", |b| {
        b.iter(|| black_box(idle.render_buffer(W, H)));
    });

    let mut loaded = app::app_with_result(common::wide_result(100_000, 12), W, H);
    group.bench_function("100k_result", |b| {
        b.iter(|| black_box(loaded.render_buffer(W, H)));
    });

    group.finish();
}

criterion_group!(benches, full_frame);
criterion_main!(benches);
