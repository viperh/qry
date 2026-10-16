//! The results grid: one draw with a large result loaded, sizing its
//! columns, and handling the answer that brings it.
//!
//! The draws go through the public whole-screen entry point
//! (`AppHarness::render_buffer`, which calls `qry::views::draw` into a
//! `TestBackend`), not the results view alone: the grid view is being
//! redesigned (T303), and the whole screen is what a frame costs anyway.
//! Each figure includes making the test terminal and copying its buffer
//! out, which is small next to the draw.

use std::hint::black_box;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};

#[path = "app.rs"]
mod app;
#[path = "common.rs"]
mod common;

/// The cap the grid draws columns at (`MAX_COLUMN_WIDTH`).
const WIDTH_CAP: u16 = 40;

fn results_render(c: &mut Criterion) {
    let result = common::wide_result(100_000, 12);
    for (w, h) in [(300, 100), (80, 24)] {
        let mut harness = app::app_with_result(result.clone(), w, h);
        let mut group = c.benchmark_group(format!("results_render_{w}x{h}"));
        group.bench_function("100k_rows", |b| {
            b.iter(|| black_box(harness.render_buffer(w, h)));
        });
        group.finish();
    }
}

/// T303's gate: 120x40 of a 1M x 20 result.
fn results_render_1m(c: &mut Criterion) {
    let mut harness = app::app_with_result(common::wide_result(1_000_000, 20), 120, 40);
    c.bench_function("results_render_1m", |b| {
        b.iter(|| black_box(harness.render_buffer(120, 40)));
    });
}

fn column_widths(c: &mut Criterion) {
    let mut group = c.benchmark_group("column_widths");
    for (id, rows) in [("100k_rows", 100_000), ("1k_rows", 1_000)] {
        let result = common::wide_result(rows, 12);
        group.bench_function(id, |b| {
            b.iter(|| black_box(qry_core::grid::column_widths(black_box(&result), WIDTH_CAP)));
        });
    }
    group.finish();
}

/// What the UI thread does when a 100k-row answer arrives: the reducer
/// stores it and sizes its columns. Not gated; it shows what the sizing in
/// the app costs until it uses the sampled `column_widths`.
fn results_arrive(c: &mut Criterion) {
    let result = common::wide_result(100_000, 12);
    let mut group = c.benchmark_group("results_arrive");
    group.sample_size(20);
    group.bench_function("100k_rows", |b| {
        b.iter_batched(
            || (app::harness(), result.clone()),
            |(mut h, result)| {
                app::show_result(&mut h, result);
                h
            },
            BatchSize::LargeInput,
        );
    });
    group.finish();
}

criterion_group!(
    benches,
    results_render,
    results_render_1m,
    column_widths,
    results_arrive
);
criterion_main!(benches);
