//! Opening and closing the palette from each pane.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{state::Focus, *};
use crate::{testing::AppHarness, theme::Pill};

fn open(h: &AppHarness) -> bool {
    matches!(h.state().overlays.last(), Some(state::Overlay::Palette(_)))
}

#[test]
fn ctrl_p_opens_everywhere_colon_not_in_editor() {
    let mut h = AppHarness::new(Config::embedded());
    for focus in [Focus::Tree, Focus::Editor, Focus::Results] {
        h.state_mut().focus = focus;
        h.keys("ctrl-p");
        assert!(open(&h), "{focus:?}");
        h.keys("esc");
        assert!(!open(&h));
    }
    for focus in [Focus::Tree, Focus::Results] {
        h.state_mut().focus = focus;
        h.keys(":");
        assert!(open(&h), "{focus:?}");
        h.keys("esc");
    }
    h.state_mut().focus = Focus::Editor;
    h.keys(":");
    assert!(!open(&h));
    assert_eq!(h.state().editor().query(), ":");
    // From the help too, which it replaces.
    h.keys("f1 ctrl-p");
    assert!(open(&h));
    assert!(h.state().help().is_none());
}

#[test]
fn esc_restores_mode_and_focus() {
    let mut h = AppHarness::new(Config::embedded());
    h.keys("tab");
    let before = h.state().clone();
    h.keys("ctrl-p").type_str("quit").keys("esc");
    assert!(h.take_effects().is_empty(), "nothing ran");
    assert_eq!(h.state().focus, Focus::Editor);
    assert_eq!(h.state().pill(), Pill::Edit);
    let mut after = h.state().clone();
    after.dirty = before.dirty;
    assert_eq!(after, before);
}

#[test]
fn pill_shows_command_while_open() {
    let mut h = AppHarness::new(Config::embedded());
    h.keys("ctrl-p");
    assert_eq!(h.state().pill(), Pill::Command);
    assert!(
        h.render(120, 40)
            .lines()
            .next()
            .unwrap()
            .contains(" COMMAND ")
    );
}
