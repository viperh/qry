//! Puts the app into the state a benchmark draws, through
//! [`AppHarness`] and the public entry points only (keys, events and
//! `views::draw` behind `render_buffer`), so the benches keep working while
//! the views and the reducer change underneath them.

#![allow(dead_code, unreachable_pub)]

use std::sync::Arc;

use qry::{
    app::{DbAnswer, UiEvent},
    config::Config,
    keymap::KeyContext,
    testing::AppHarness,
};
use qry_core::QueryResult;

/// A fresh app on the built-in config, drawing the dark theme in truecolor.
pub fn harness() -> AppHarness {
    AppHarness::new(Config::embedded())
}

/// Moves the focus to the pane whose keys apply in `context`: `F6` from the
/// editor (where Tab indents), Tab elsewhere.
///
/// # Panics
/// If the pane is not reached within a few presses.
pub fn focus(h: &mut AppHarness, context: KeyContext) {
    for _ in 0..8 {
        match h.contexts().first() {
            Some(c) if *c == context => return,
            Some(KeyContext::Editor) => _ = h.keys("f6"),
            _ => _ = h.keys("tab"),
        }
    }
    panic!(
        "could not focus {context:?}; contexts are {:?}",
        h.contexts()
    );
}

/// Delivers `result` as the active session's query answer, as the worker
/// would after `F5`.
pub fn show_result(h: &mut AppHarness, result: QueryResult) {
    h.db(DbAnswer::QueryDone(Arc::new(result)));
}

/// An app showing `result` with the results pane focused and the selection
/// on the middle row, drawn once at `w`x`h` so the grid knows its page.
pub fn app_with_result(result: QueryResult, w: u16, height: u16) -> AppHarness {
    let middle = result.rows.len() / 2;
    let mut h = harness();
    show_result(&mut h, result);
    focus(&mut h, KeyContext::Results);
    h.render_buffer(w, height);
    // One row at a time, so the selection lands on the same row whatever
    // the page size.
    let down = qry::testing::parse_keys("down");
    for _ in 0..middle {
        h.key(down[0]);
    }
    h.render_buffer(w, height);
    h
}

/// An app whose editor holds `text`, focused, with the cursor at the end.
pub fn app_with_editor_text(text: &str) -> AppHarness {
    let mut h = harness();
    focus(&mut h, KeyContext::Editor);
    h.send(UiEvent::Paste(text.to_string()));
    h
}
