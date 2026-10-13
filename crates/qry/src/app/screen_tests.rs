//! The main screen as a whole, through [`AppHarness`]: the chrome around
//! the panes, the keys that change it, the mouse, and a terminal too small
//! to draw it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use super::{state::Focus, *};
use crate::{
    testing::{AppHarness, SIZES, buffer_lines, result, styled},
    views::layout::{DEFAULT_SPLIT, Region},
};

/// A scratch session that ran a small query, as the runtime sets it up.
fn screen() -> AppHarness {
    let mut h = AppHarness::new(Config::embedded());
    h.send(UiEvent::Started);
    h.db(DbAnswer::Connected(":memory:".into()));
    h.keys("tab").type_str("select id, name from users");
    h.keys("f8");
    h.db(DbAnswer::QueryDone(Arc::new(result(
        &["id", "name"],
        &[&[Some("1"), Some("alice")], &[Some("2"), None]],
    ))));
    h.keys("shift-tab");
    h
}

fn sized(w: u16, h: u16) -> AppHarness {
    let mut app = screen();
    app.resize(w, h);
    app.take_effects();
    app
}

#[test]
fn screen_snapshots() {
    for (w, h) in [(80, 24), (120, 40), (160, 48)] {
        let mut app = sized(w, h);
        insta::assert_snapshot!(format!("screen_{w}x{h}"), app.render(w, h));
        insta::assert_snapshot!(
            format!("screen_{w}x{h}_styled"),
            styled(&app.render_buffer(w, h))
        );
    }
}

#[test]
fn screen_too_small_72x20() {
    let mut app = sized(72, 20);
    let text = app.render(72, 20);
    assert!(
        text.contains("terminal too small — 80x24 needed, have 72x20"),
        "{text}"
    );
    insta::assert_snapshot!("screen_too_small_72x20", text);
}

#[test]
fn every_key_but_quit_is_ignored_when_too_small() {
    let mut app = sized(72, 20);
    let before = app.state().clone();
    app.keys("tab ctrl-n f1 j");
    let mut after = app.state().clone();
    after.dirty = before.dirty;
    assert_eq!(after, before);
    assert!(app.take_effects().is_empty());
    app.keys("ctrl-q");
    assert!(matches!(app.take_effects().as_slice(), [Effect::Quit]));
}

#[test]
fn screen_focus_moves_title_accent() {
    let mut app = sized(120, 40);
    let accent = app.theme().pane_title_focused().fg;
    // Where each pane's label starts: the tree, the editor, the results.
    let titles = |app: &mut AppHarness| {
        let buffer = app.render_buffer(120, 40);
        let lines = buffer_lines(&buffer);
        let find = |label: &str| {
            let y = lines.iter().position(|l| l.contains(label)).unwrap();
            let x = lines[y][..lines[y].find(label).unwrap()].chars().count();
            Some(buffer[(u16::try_from(x).unwrap(), u16::try_from(y).unwrap())].fg)
        };
        [find("SCHEMA"), find("QUERY"), find("RESULTS")]
    };
    for (step, focused) in [(0, 0), (1, 1), (2, 2), (3, 0)] {
        let shown = titles(&mut app);
        for (i, fg) in shown.iter().enumerate() {
            assert_eq!(*fg == accent, i == focused, "step {step}: pane {i}");
        }
        insta::assert_snapshot!(
            format!("screen_focus_{step}_120x40"),
            buffer_lines(&app.render_buffer(120, 40))[..3].join("\n")
        );
        app.keys("tab");
    }
}

#[test]
fn every_size_renders_without_panic() {
    for &(w, h) in SIZES {
        let mut app = sized(w, h);
        app.render(w, h);
    }
    for (w, h) in [(0, 0), (1, 1), (2, 1), (10, 3), (80, 24), (300, 100)] {
        let mut app = sized(w, h);
        app.render(w, h);
    }
}

#[test]
fn main_screen_has_no_boxes() {
    let overlays = [
        "form.rs",
        "help.rs",
        "overlay.rs",
        "confirm.rs",
        "palette.rs",
    ];
    for (path, text) in crate::testing::sources(&["views"]) {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if overlays.contains(&name.as_str()) || name.ends_with("_tests.rs") {
            continue;
        }
        assert!(
            !text.contains(&["Block::", "bordered"].concat()),
            "{} draws a box",
            path.display()
        );
    }
}

#[test]
fn ctrl_up_and_down_move_the_split_in_steps() {
    let mut app = sized(120, 40);
    assert_eq!(app.state().chrome.split_percent, DEFAULT_SPLIT);
    app.keys("ctrl-up");
    assert_eq!(app.state().chrome.split_percent, DEFAULT_SPLIT + 5);
    for _ in 0..20 {
        app.keys("ctrl-up");
    }
    assert_eq!(app.state().chrome.split_percent, 90);
    for _ in 0..30 {
        app.keys("ctrl-down");
    }
    assert_eq!(app.state().chrome.split_percent, 10);
}

#[test]
fn f11_maximises_the_focused_region_and_restores() {
    let mut app = sized(120, 40);
    app.keys("f11");
    assert_eq!(
        app.state().chrome.maximised,
        None,
        "the tree is not maximised"
    );
    app.keys("tab tab f11");
    assert_eq!(app.state().chrome.maximised, Some(Region::Results));
    let text = app.render(120, 40);
    assert!(!text.contains("QUERY"), "{text}");
    assert!(text.contains("RESULTS"), "{text}");
    app.keys("f11");
    assert_eq!(app.state().chrome.maximised, None);
    assert!(app.render(120, 40).contains("QUERY"));
    app.keys("shift-tab f11");
    assert_eq!(app.state().chrome.maximised, Some(Region::Editor));
}

#[test]
fn the_hint_line_can_be_toggled_off() {
    let mut app = sized(120, 40);
    assert!(app.render(120, 40).lines().last().unwrap().contains("help"));
    app.keys("ctrl-/");
    assert!(!app.state().chrome.hints);
    let text = app.render(120, 40);
    assert!(!text.lines().last().unwrap().contains("help"), "{text}");
    assert!(
        text.lines().last().unwrap().contains("scratch · sqlite"),
        "{text}"
    );
}

fn click(app: &mut AppHarness, column: u16, row: u16, kind: MouseEventKind) {
    app.send(UiEvent::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }));
}

#[test]
fn mouse_focuses_panes_switches_tabs_and_scrolls() {
    let mut app = sized(120, 40);
    let left = MouseEventKind::Down(MouseButton::Left);
    click(&mut app, 60, 5, left);
    assert_eq!(app.state().focus, Focus::Editor);
    click(&mut app, 60, 30, left);
    assert_eq!(app.state().focus, Focus::Results);
    click(&mut app, 5, 30, left);
    assert_eq!(app.state().focus, Focus::Tree);

    // A second session, then a click on the first tab (` 1 scratch ` from
    // column 5).
    app.keys("shift-enter");
    assert_eq!(app.active_session(), app.sessions()[1]);
    click(&mut app, 7, 0, left);
    assert_eq!(app.active_session(), app.sessions()[0]);

    // The wheel over the results moves its selection, 3 rows a notch.
    app.db(DbAnswer::QueryDone(Arc::new(crate::testing::grid(20, 2))));
    click(&mut app, 60, 30, MouseEventKind::ScrollDown);
    assert_eq!(app.state().results().row, 3);
    click(&mut app, 60, 30, MouseEventKind::ScrollUp);
    assert_eq!(app.state().results().row, 0);
}
