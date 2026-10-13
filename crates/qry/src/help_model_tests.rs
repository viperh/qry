//! Unit tests for `help_model.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::*;
use crate::{config::Config, testing::parse_keys};

fn keymap() -> Keymap {
    Config::embedded().keymap
}

fn entry<'a>(groups: &'a [HelpGroup], title: &str, desc: &str) -> Option<&'a (String, String)> {
    groups
        .iter()
        .find(|g| g.title == title)?
        .entries
        .iter()
        .find(|(_, d)| d == desc)
}

/// The groups that list `context`'s bindings.
fn titles_for(context: KeyContext) -> Vec<&'static str> {
    GROUPS
        .iter()
        .filter(|g| g.context == context)
        .map(|g| g.title)
        .collect()
}

#[test]
fn every_bound_action_appears_once_per_context() {
    let keymap = keymap();
    let groups = build(&keymap);
    for context in [
        KeyContext::Global,
        KeyContext::Tree,
        KeyContext::Editor,
        KeyContext::Results,
        KeyContext::Inspector,
        KeyContext::Dialog,
        KeyContext::Palette,
        KeyContext::Jobs,
    ] {
        let titles = titles_for(context);
        for (_, action) in keymap.bindings(context) {
            let desc = description(context, action);
            let found: usize = groups
                .iter()
                .filter(|g| titles.contains(&g.title))
                .map(|g| g.entries.iter().filter(|(_, d)| *d == desc).count())
                .sum();
            assert_eq!(found, 1, "{context:?} {action:?} ({desc})");
        }
    }
}

#[test]
fn remapped_key_shows_and_unbound_action_disappears() {
    let mut keymap = keymap();
    keymap.bind(
        KeyContext::Editor,
        &parse_keys("alt-r"),
        Action::RunStatement,
    );
    for chord in keymap.chords_for(KeyContext::Results, &Action::CopyRow) {
        keymap.unbind(KeyContext::Results, &chord);
    }
    let groups = build(&keymap);
    let (keys, _) = entry(&groups, "Editor · Running", "run the statement").unwrap();
    assert!(keys.split(' ').any(|k| k == "alt-r"), "{keys}");
    assert!(entry(&groups, "Results · Copy & search", "copy the row").is_none());
}

#[test]
fn chords_order_plain_then_shorter() {
    let groups = build(&keymap());
    let (keys, _) = entry(&groups, "Results · Navigation", "down").unwrap();
    assert_eq!(keys, "j ↓");
    let mut keymap = keymap();
    keymap.bind(
        KeyContext::Global,
        &[KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)],
        Action::Quit,
    );
    let groups = build(&keymap);
    let (keys, _) = entry(&groups, "Global", "quit").unwrap();
    assert_eq!(keys, "q ctrl-q");
}

#[test]
fn session_switch_collapses_to_one_row() {
    let groups = build(&keymap());
    let sessions = groups.iter().find(|g| g.title == "Sessions").unwrap();
    let rows: Vec<_> = sessions
        .entries
        .iter()
        .filter(|(_, d)| d == "switch to session N")
        .collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "ctrl-1–9");
}

#[test]
fn same_description_entries_merge() {
    let mut keymap = keymap();
    // Two actions that read the same in the help: give F2 the same job as
    // ? in the tree, through another action with that description.
    keymap.bind(KeyContext::Tree, &parse_keys("F2"), Action::Help);
    let groups = build(&keymap);
    let tree = groups.iter().find(|g| g.title == "Tree").unwrap();
    let help: Vec<_> = tree
        .entries
        .iter()
        .filter(|(_, d)| d == "show / hide the help")
        .collect();
    assert_eq!(help.len(), 1, "{help:?}");
    assert!(
        help[0].0.contains('?') && help[0].0.contains("F2"),
        "{help:?}"
    );

    // CopyCell and CopyRow keep their own rows: they say different things.
    let groups = build(&Config::embedded().keymap);
    let copy = groups
        .iter()
        .find(|g| g.title == "Results · Copy & search")
        .unwrap();
    assert!(copy.entries.len() >= 2);
}

#[test]
fn unlisted_actions_fall_into_other_group() {
    let mut keymap = keymap();
    keymap.bind(KeyContext::Editor, &parse_keys("F9"), Action::ToggleJobs);
    keymap.bind(KeyContext::Results, &parse_keys("F9"), Action::Refresh);
    let groups = build(&keymap);
    let other = groups.iter().find(|g| g.title == "Editor · Other").unwrap();
    assert!(other.continues);
    assert!(other.entries.iter().any(|(k, _)| k == "F9"));
    assert!(groups.iter().any(|g| g.title == "Results · Other"));
}

#[test]
fn empty_groups_are_dropped() {
    let mut keymap = keymap();
    for (chord, _) in keymap
        .bindings(KeyContext::Jobs)
        .map(|(c, a)| (c.clone(), *a))
        .collect::<Vec<_>>()
    {
        keymap.unbind(KeyContext::Jobs, &chord);
    }
    let groups = build(&keymap);
    assert!(!groups.iter().any(|g| g.title == "Jobs"));
    assert!(
        build(&Config::embedded().keymap)
            .iter()
            .any(|g| g.title == "Jobs")
    );
}

#[test]
fn context_description_overrides_default() {
    assert_eq!(
        description(KeyContext::Tree, &Action::Open),
        "connect / switch to its session"
    );
    assert_eq!(description(KeyContext::Results, &Action::Open), "open");
    assert_eq!(description(KeyContext::Global, &Action::Quit), "quit");
}

#[test]
fn the_footer_and_the_help_keys_come_from_the_keymap() {
    let keymap = keymap();
    let footer = footer(&keymap);
    assert_eq!(footer[0].label, "run:");
    assert!(footer[0].pieces.contains(&Piece::Key("F8".into())));
    assert_eq!(
        help_keys(&keymap),
        [
            ("k/j".to_string(), "scroll"),
            ("PgUp/PgDn".to_string(), "page"),
            ("Home/G".to_string(), "top/bottom"),
            ("?/Esc".to_string(), "close"),
        ]
    );
}
