//! Unit tests for `help.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ratatui::{Terminal, backend::TestBackend};

use super::*;

fn text(help: &mut Help, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| help.draw(frame, frame.area()).unwrap())
        .unwrap();
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                + "\n"
        })
        .collect()
}

#[test]
fn sections_come_from_the_config_in_a_fixed_order() {
    let help = Help::from_config(&Config::embedded());
    let titles: Vec<_> = help.sections.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(
        titles,
        [
            "Global",
            "Connection tree",
            "Editor",
            "Results",
            "Forms",
            "New Connection",
            "Export",
            "Password",
            "Import",
            "Export Connections",
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
    assert_eq!(find("Global", "Quit").as_deref(), Some("Ctrl-q"));
    assert_eq!(find("Connection tree", "Connect").as_deref(), Some("Enter"));
    assert_eq!(
        find("Connection tree", "Expand or collapse").as_deref(),
        Some("Space")
    );
    assert_eq!(find("Editor", "Run query").as_deref(), Some("F8"));
    assert_eq!(find("Results", "Up a row").as_deref(), Some("↑ / i"));
    assert_eq!(find("Results", "Last row").as_deref(), Some("G"));
    assert_eq!(find("New Connection", "Close").as_deref(), Some("Esc"));
    assert_eq!(find("Export", "Close").as_deref(), Some("Esc"));
    assert_eq!(
        find("Forms", "Previous field").as_deref(),
        Some("Shift-Tab")
    );
    assert_eq!(find("Forms", "Submit the form").as_deref(), Some("Enter"));
}

#[test]
fn a_rebound_key_shows_up_in_the_help() {
    let config: Config =
        json5::from_str(r#"{ "panes": { "Results": { "<x>": "FirstRow" } } }"#).unwrap();
    let mut help = Help::from_config(&config);
    let shown = text(&mut help, 60, 10);
    assert!(shown.contains("x : First row"), "{shown}");
    assert!(!shown.contains("Editor"), "{shown}");
}

#[test]
fn a_tall_help_scrolls_and_stops_at_the_end() {
    let mut help = Help::from_config(&Config::embedded());
    let first = text(&mut help, 80, 20);
    assert!(first.contains("Global:"), "{first}");
    assert!(first.contains(" 1-18 of "), "{first}");

    for _ in 0..500 {
        help.scroll(HelpCommand::PageDown);
    }
    let last = text(&mut help, 80, 20);
    assert!(!last.contains("Global:"), "{last}");
    assert!(last.contains("Close help"), "{last}");

    help.scroll_to_top();
    assert!(text(&mut help, 80, 20).contains("Global:"));
}
