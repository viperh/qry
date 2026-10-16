//! The SQL editor (T302) with a 10 000-line buffer: one whole-screen draw
//! at 120x40, which lexes the buffer for highlighting and splits it into
//! statements, and one keystroke followed by that draw, as typing costs.
//!
//! Drawn through `AppHarness::render_buffer` (`qry::views::draw`), since
//! `views::editor` is private to the crate.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};

#[path = "app.rs"]
mod app;
#[path = "common.rs"]
mod common;

const LINES: usize = 10_000;

fn editor(c: &mut Criterion) {
    let text = common::sql_lines(LINES);
    let mut h = app::app_with_editor_text(&text);
    h.render_buffer(120, 40);

    c.bench_function("editor_render_10k_lines", |b| {
        b.iter(|| black_box(h.render_buffer(120, 40)));
    });

    // A character in and out again, so the buffer stays the same size;
    // each is followed by the frame it causes.
    let keys = qry::testing::parse_keys("x backspace");
    c.bench_function("editor_keystroke_10k_lines", |b| {
        b.iter(|| {
            h.key(keys[0]);
            black_box(h.render_buffer(120, 40));
            h.key(keys[1]);
            black_box(h.render_buffer(120, 40))
        });
    });
}

criterion_group!(benches, editor);
criterion_main!(benches);
