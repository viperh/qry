//! The help through the reducer: opening, closing, and keeping every other
//! key away while it is open.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::{
    app::{UiEvent, state::Focus},
    config::Config,
    testing::{AppHarness, styled},
    theme::Pill,
};

fn harness() -> AppHarness {
    let mut h = AppHarness::new(Config::embedded());
    h.send(UiEvent::Started);
    h.take_effects();
    h
}

#[test]
fn question_mark_opens_and_esc_closes() {
    let mut h = harness();
    h.keys("?");
    assert!(h.state().help().is_some());
    let text = h.render(120, 40);
    assert!(
        text.contains(&format!("Help — qry {}", env!("CARGO_PKG_VERSION"))),
        "{text}"
    );
    h.keys("esc");
    assert!(h.state().help().is_none());
    h.keys("? ?");
    assert!(h.state().help().is_none(), "? closes it too");
    h.keys("f1 f1");
    assert!(h.state().help().is_none());
}

#[test]
fn f1_opens_from_the_editor_while_question_mark_types() {
    let mut h = harness();
    h.keys("tab ?");
    assert!(h.state().help().is_none());
    assert_eq!(h.state().editor().query(), "?");
    h.keys("f1");
    assert!(h.state().help().is_some());
}

#[test]
fn help_swallows_other_actions() {
    let mut h = harness();
    h.keys("tab").type_str("select 1").keys("f1");
    h.keys("ctrl-enter f5 f8 tab ctrl-n x");
    assert!(h.take_effects().is_empty(), "no query, no dialog");
    assert_eq!(h.state().focus, Focus::Editor);
    assert_eq!(h.state().editor().query(), "select 1");
    assert!(h.state().dialog().is_none());
}

#[test]
fn pill_keeps_previous_mode_under_help() {
    let mut h = harness();
    h.keys("tab f1");
    assert_eq!(h.state().pill(), Pill::Edit);
    h.keys("esc");
    assert!(h.render(120, 40).lines().next().unwrap().contains(" EDIT "));
}

#[test]
fn opening_resets_scroll() {
    let mut h = harness();
    h.keys("f1");
    h.render(80, 24);
    h.keys("G");
    assert!(h.state().help().unwrap().scroll.get() > 0);
    h.keys("esc f1");
    assert_eq!(h.state().help().unwrap().scroll.get(), 0);
}

#[test]
fn a_resize_reclamps_the_scroll() {
    let mut h = harness();
    h.keys("f1");
    h.render(80, 24);
    h.keys("G");
    h.render(300, 100);
    assert_eq!(h.state().help().unwrap().scroll.get(), 0, "it all fits now");
}

#[test]
fn help_snapshots() {
    let mut h = harness();
    h.keys("f1");
    for (name, w, ht) in [
        ("help_120x40", 120, 40),
        ("help_160x48", 160, 48),
        ("help_80x24_top", 80, 24),
    ] {
        h.resize(w, ht);
        insta::assert_snapshot!(name, h.render(w, ht));
        insta::assert_snapshot!(format!("{name}_styled"), styled(&h.render_buffer(w, ht)));
    }
    h.keys("G");
    insta::assert_snapshot!("help_80x24_bottom", h.render(80, 24));
    insta::assert_snapshot!("help_80x24_bottom_styled", styled(&h.render_buffer(80, 24)));

    let json = r#"{ "keybindings": { "Editor": { "<alt-r>": "RunStatement" }, "Results": { "<Y>": null } } }"#;
    let mut h = AppHarness::new(Config::with_user_file("config.json", json).unwrap());
    h.keys("f1");
    let help = h.state().help().unwrap();
    let running = help
        .groups
        .iter()
        .find(|g| g.title == "Editor · Running")
        .unwrap();
    assert!(running.entries[0].0.contains("alt-r"), "{running:?}");
    // Scrolled to where the editor's groups are.
    h.render(120, 40);
    h.keys("pagedown j j j j");
    let text = h.render(120, 40);
    assert!(text.contains("alt-r"), "{text}");
    insta::assert_snapshot!("help_120x40_remapped", text);
}
