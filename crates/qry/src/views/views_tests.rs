//! Unit tests for `views.rs`: the main screen and what is drawn over it, as
//! the [`AppHarness`] draws them.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::{
    app::{DbAnswer, UiEvent},
    config::Config,
    testing::AppHarness,
};

/// The main screen with the built-in config, as the runtime sets it up.
fn harness() -> AppHarness {
    AppHarness::new(Config::embedded())
}

#[test]
fn home_empty_snapshots() {
    let mut h = harness();
    insta::assert_snapshot!("home_empty_80x24", h.render(80, 24));
    insta::assert_snapshot!("home_empty_160x48", h.render(160, 48));
}

/// The refactor's baseline: the screen once the scratch database is open,
/// as every run starts.
#[test]
fn home_unchanged_snapshots() {
    let mut h = harness();
    h.send(UiEvent::Started)
        .db(DbAnswer::Connected(":memory:".into()))
        .db(DbAnswer::Notice(crate::notice::Notice::success(
            "Connected to scratch (:memory:)",
        )));
    insta::assert_snapshot!("home_unchanged_80x24", h.render(80, 24));
    insta::assert_snapshot!("home_unchanged_160x48", h.render(160, 48));
}

#[test]
fn connection_form_new_snapshot() {
    let mut h = harness();
    h.keys("ctrl-n");
    insta::assert_snapshot!("connection_form_new_80x24", h.render(80, 24));
}

#[test]
fn export_form_snapshot() {
    let mut h = harness();
    // ctrl-s with the results focused.
    h.keys("shift-tab ctrl-s");
    insta::assert_snapshot!("export_form_80x24", h.render(80, 24));
}

#[test]
fn password_prompt_snapshot() {
    let mut h = harness();
    h.db(DbAnswer::NeedPassword {
        id: "c7f3a1e2".into(),
        name: "prod".into(),
    });
    insta::assert_snapshot!("password_prompt_80x24", h.render(80, 24));
}

#[test]
fn help_snapshot() {
    let mut h = harness();
    h.keys("f1");
    insta::assert_snapshot!("help_80x24", h.render(80, 24));
}

#[test]
fn tree_is_focused_and_highlighted_at_startup() {
    use crate::app::state::Focus;

    let mut h = harness();
    assert_eq!(h.state().focus, Focus::Tree);

    // The first letters of the tree's and the editor's titles, under the
    // top bar.
    let focused = h.theme().pane_title_focused().fg.unwrap();
    let buf = h.render_buffer(80, 24);
    assert_eq!(buf[(1, 1)].symbol(), "S");
    assert_eq!(buf[(32, 1)].symbol(), "Q");
    assert_eq!(buf[(1, 1)].fg, focused);
    assert_ne!(buf[(32, 1)].fg, focused);

    h.keys("tab");
    assert_eq!(h.state().focus, Focus::Editor);
    let buf = h.render_buffer(80, 24);
    assert_ne!(buf[(1, 1)].fg, focused);
    assert_eq!(buf[(32, 1)].fg, focused);

    // Tab indents in the editor; F6 moves on.
    h.keys("f6");
    assert_eq!(h.state().focus, Focus::Results);
    h.keys("tab");
    assert_eq!(h.state().focus, Focus::Tree);
    h.keys("shift-tab");
    assert_eq!(h.state().focus, Focus::Results);
}
