//! Unit tests for `hints.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::{
    config::Config,
    testing::{AppHarness, buffer_lines, render_with},
};

fn line(h: &AppHarness, width: u16) -> String {
    let buffer = render_with(width, 1, |frame| {
        let state = h.state();
        draw(state, &state.theme, frame, frame.area());
    });
    buffer_lines(&buffer)[0].clone()
}

#[test]
fn hints_follow_the_focused_context() {
    let mut h = AppHarness::new(Config::embedded());
    let tree = line(&h, 120);
    h.keys("tab");
    let editor = line(&h, 120);
    h.keys("f6");
    let results = line(&h, 120);
    assert!(tree.contains("Enter open"), "{tree}");
    assert!(
        editor.contains("F5 run") && editor.contains("F8 run all"),
        "{editor}"
    );
    assert!(results.contains("ctrl-e export"), "{results}");
    assert_ne!(tree, editor);
    assert_ne!(editor, results);
    h.keys("ctrl-n");
    assert!(line(&h, 120).contains("Enter submit"));
}

#[test]
fn remapped_key_appears_and_unbound_pair_is_dropped() {
    let json =
        r#"{ "keybindings": { "Tree": { "<o>": "Open", "<enter>": null, "<space>": null } } }"#;
    let config = Config::with_user_file("config.json", json).unwrap();
    let h = AppHarness::new(config);
    let text = line(&h, 120);
    assert!(text.contains("o open"), "{text}");
    assert!(!text.contains("expand"), "{text}");
}

#[test]
fn help_hint_stays_pinned_when_narrow() {
    let mut h = AppHarness::new(Config::embedded());
    for width in [80, 40, 12] {
        let text = line(&h, width);
        assert!(text.trim_end().ends_with("? help"), "{width}: {text}");
    }
    let text = line(&h, 40);
    assert!(!text.contains("quit"), "pairs go from the right: {text}");
    h.keys("tab");
    assert!(line(&h, 80).trim_end().ends_with("F1 help"));
}
