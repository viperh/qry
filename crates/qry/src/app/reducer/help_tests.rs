//! Unit tests for `help.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::{
    testing::{buffer_to_string, render_with},
    theme::Theme,
    views,
};

/// The help drawn over a `width`x`height` terminal, as text.
fn text(help: &HelpState, width: u16, height: u16) -> String {
    buffer_to_string(&render_with(width, height, |frame| {
        views::help::draw(help, &Theme::DARK, frame, frame.area());
    }))
}

#[test]
fn sections_come_from_the_config_in_a_fixed_order() {
    let help = HelpState::from_config(&Config::embedded());
    let titles: Vec<_> = help.sections.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(
        titles,
        [
            "Global",
            "Connection tree",
            "Editor",
            "Results",
            "Dialogs",
            "Help"
        ]
    );

    let find = |title: &str, desc: &str| {
        let (_, binds) = help.sections.iter().find(|(t, _)| t == title).unwrap();
        binds
            .iter()
            .find(|(_, d)| d == desc)
            .map(|(k, _)| k.clone())
    };
    assert_eq!(find("Global", "Quit").as_deref(), Some("ctrl-q"));
    assert_eq!(
        find("Global", "Switch to session N").as_deref(),
        Some("ctrl-1 … ctrl-9")
    );
    assert_eq!(find("Connection tree", "Open").as_deref(), Some("Enter"));
    assert_eq!(
        find("Connection tree", "Expand or collapse").as_deref(),
        Some("Space")
    );
    assert_eq!(find("Editor", "Run everything").as_deref(), Some("F8"));
    assert_eq!(
        find("Editor", "Run the statement").as_deref(),
        Some("F5 / ctrl-Enter")
    );
    assert_eq!(find("Results", "Up").as_deref(), Some("k / ↑"));
    assert_eq!(find("Results", "To the top").as_deref(), Some("g g"));
    assert_eq!(find("Results", "To the bottom").as_deref(), Some("G"));
    assert_eq!(find("Dialogs", "Cancel / close").as_deref(), Some("Esc"));
    assert_eq!(
        find("Dialogs", "Previous field").as_deref(),
        Some("shift-Tab")
    );
    assert_eq!(find("Dialogs", "Submit").as_deref(), Some("Enter"));

    // Lines follow the actions' order.
    let (_, global) = &help.sections[0];
    assert_eq!(global[0].1, "Quit");
}

#[test]
fn a_rebound_key_shows_up_in_the_help() {
    let config = Config::with_user_file(
        "c.json",
        r#"{ "keybindings": { "Results": { "<x>": "Top" } } }"#,
    )
    .unwrap();
    let help = HelpState::from_config(&config);
    let shown = text(&help, 80, 120);
    assert!(
        shown
            .lines()
            .any(|l| l.contains("x / g g ") && l.contains(": To the top")),
        "{shown}"
    );
}

#[test]
fn a_tall_help_scrolls_and_stops_at_the_end() {
    let help = HelpState::from_config(&Config::embedded());
    let first = text(&help, 80, 20);
    assert!(first.contains("Global:"), "{first}");
    assert!(first.contains(" 1-18 of "), "{first}");

    for _ in 0..500 {
        help.scroll(Action::PageDown);
    }
    let last = text(&help, 80, 20);
    assert!(!last.contains("Global:"), "{last}");
    assert!(last.contains("Cancel / close"), "{last}");
    help.scroll(Action::Top);
    assert!(text(&help, 80, 20).contains("Global:"));
    help.scroll(Action::Bottom);
    assert_eq!(text(&help, 80, 20), last);

    // Closed and opened again, it starts from the top.
    let mut state = State {
        config: std::sync::Arc::new(Config::embedded()),
        ..State::default()
    };
    toggle(&mut state);
    text(state.help().unwrap(), 80, 20);
    act(&mut state, Action::PageDown);
    assert!(!text(state.help().unwrap(), 80, 20).contains("Global:"));
    act(&mut state, Action::Cancel);
    assert!(state.help().is_none());
    toggle(&mut state);
    assert!(text(state.help().unwrap(), 80, 20).contains("Global:"));
}
