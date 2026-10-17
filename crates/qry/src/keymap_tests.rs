//! Unit tests for `keymap.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pretty_assertions::assert_eq;
use proptest::prelude::*;
use strum::IntoEnumIterator;

use super::*;
use crate::{app::state::HelpState, config::Config};

fn chord(raw: &str) -> Vec<KeyEvent> {
    parse_key_sequence(raw).unwrap()
}

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, mods)
}

fn ch(c: char) -> KeyEvent {
    key(KeyCode::Char(c), KeyModifiers::NONE)
}

#[test]
fn every_action_has_a_description() {
    for action in Action::iter() {
        assert!(!action.description().is_empty(), "{action:?}");
    }
}

#[test]
fn every_default_binding_parses() {
    let config = Config::embedded();
    assert!(config.warnings().is_empty(), "{:?}", config.warnings());
    assert!(config.keymap.prefix_conflicts().is_empty());
}

#[test]
fn the_defaults_are_the_documented_ones() {
    let keymap = Config::embedded().keymap;
    let at = |context, raw: &str| keymap.exact(&[context], &chord(raw));
    use Action as A;
    use KeyContext as K;
    let expected = [
        (K::Global, "ctrl-q", A::Quit),
        (K::Global, "F1", A::Help),
        (K::Global, "ctrl-p", A::Palette),
        (K::Global, "Tab", A::FocusNext),
        (K::Global, "shift-Tab", A::FocusPrev),
        (K::Global, "ctrl-n", A::NewConnection),
        (K::Global, "ctrl-1", A::SwitchSession(1)),
        (K::Global, "ctrl-9", A::SwitchSession(9)),
        (K::Global, "ctrl-PgDn", A::NextSession),
        (K::Global, "ctrl-PgUp", A::PrevSession),
        (K::Global, "ctrl-z", A::Suspend),
        (K::Editor, "ctrl-Enter", A::RunStatement),
        (K::Editor, "F5", A::RunStatement),
        (K::Editor, "F8", A::RunAll),
        (K::Editor, "ctrl-e", A::RunSelection),
        (K::Editor, "ctrl-c", A::CancelQuery),
        (K::Editor, "ctrl-z", A::Undo),
        (K::Editor, "ctrl-shift-z", A::Redo),
        (K::Editor, "ctrl-y", A::Redo),
        (K::Editor, "ctrl-x", A::Cut),
        (K::Editor, "ctrl-v", A::Paste),
        (K::Tree, "Enter", A::Open),
        (K::Tree, "Space", A::Toggle),
        (K::Tree, "e", A::EditConnection),
        (K::Tree, "Del", A::DeleteConnection),
        (K::Tree, "r", A::Refresh),
        (K::Tree, "/", A::Filter),
        (K::Results, "y", A::CopyCell),
        (K::Results, "Y", A::CopyRow),
        (K::Results, "/", A::Search),
        (K::Results, "i", A::ToggleInspector),
        (K::Results, "Enter", A::Inspect),
        (K::Results, "ctrl-e", A::ExportResults),
        (K::Dialog, "Enter", A::Submit),
        (K::Dialog, "Esc", A::Cancel),
        (K::Dialog, "Tab", A::NextField),
        (K::Dialog, "shift-Tab", A::PrevField),
        (K::Dialog, "ctrl-q", A::Quit),
        (K::Help, "j", A::Down),
        (K::Help, "k", A::Up),
        (K::Help, "PgUp", A::PageUp),
        (K::Help, "PgDn", A::PageDown),
        (K::Help, "Home", A::Top),
        (K::Help, "G", A::Bottom),
        (K::Help, "?", A::Help),
        (K::Help, "Esc", A::Cancel),
        (K::Palette, "Enter", A::Submit),
        (K::Palette, "Esc", A::Cancel),
        (K::Palette, "↑", A::Up),
        (K::Palette, "↓", A::Down),
    ];
    for (context, raw, action) in expected {
        assert_eq!(at(context, raw), Some(action), "{context} {raw}");
    }
    for context in [K::Tree, K::Results, K::Inspector, K::Jobs] {
        for (raw, action) in [
            ("?", A::Help),
            (":", A::Palette),
            ("j", A::Down),
            ("↓", A::Down),
            ("k", A::Up),
            ("↑", A::Up),
            ("h", A::Left),
            ("←", A::Left),
            ("l", A::Right),
            ("→", A::Right),
            ("g g", A::Top),
            ("G", A::Bottom),
        ] {
            assert_eq!(at(context, raw), Some(action), "{context} {raw}");
        }
    }
    // Typed in the editor, not bound.
    for raw in ["?", ":"] {
        assert_eq!(at(K::Editor, raw), None, "{raw}");
        assert_eq!(at(K::Global, raw), None, "{raw}");
    }
    // F1 and ctrl-p work in every context but a dialog, which passes
    // ctrl-p through to its field.
    for context in [K::Global, K::Dialog, K::Help, K::Palette] {
        assert_eq!(at(context, "F1"), Some(A::Help), "{context}");
    }
    for context in [K::Global, K::Help, K::Palette] {
        assert_eq!(at(context, "ctrl-p"), Some(A::Palette), "{context}");
    }
    assert_eq!(at(K::Dialog, "ctrl-p"), None);
}

#[test]
fn chord_to_string_names() {
    let cases = [
        (key(KeyCode::Char('p'), KeyModifiers::CONTROL), "ctrl-p"),
        (key(KeyCode::Enter, KeyModifiers::NONE), "Enter"),
        (key(KeyCode::BackTab, KeyModifiers::SHIFT), "shift-Tab"),
        (key(KeyCode::BackTab, KeyModifiers::NONE), "shift-Tab"),
        (key(KeyCode::Left, KeyModifiers::NONE), "←"),
        (key(KeyCode::Char('G'), KeyModifiers::SHIFT), "G"),
        (key(KeyCode::F(5), KeyModifiers::NONE), "F5"),
        (key(KeyCode::Delete, KeyModifiers::ALT), "alt-Del"),
        (key(KeyCode::PageDown, KeyModifiers::CONTROL), "ctrl-PgDn"),
        (key(KeyCode::Char(' '), KeyModifiers::NONE), "Space"),
        (key(KeyCode::Esc, KeyModifiers::NONE), "Esc"),
        (
            key(
                KeyCode::Char('Z'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
            "ctrl-shift-z",
        ),
    ];
    for (key, expected) in cases {
        assert_eq!(chord_to_string(&[key]), expected, "{key:?}");
    }
    assert_eq!(chord_to_string(&[ch('g'), ch('g')]), "g g");
}

#[test]
fn display_chord_prefers_plain_shortest() {
    assert_eq!(display_chord(&[chord("ctrl-Enter"), chord("F5")]), "F5");
    assert_eq!(display_chord(&[chord("↑"), chord("k")]), "k");
    assert_eq!(display_chord(&[chord("k"), chord("↑")]), "k");
    assert_eq!(display_chord(&[chord("Home"), chord("g g")]), "g g");
    assert_eq!(display_chord(&[chord("ctrl-q")]), "ctrl-q");
    assert_eq!(display_chord(&[]), "");
}

#[test]
fn parses_both_spellings() {
    assert_eq!(chord("<g><g>"), [ch('g'), ch('g')]);
    assert_eq!(chord("g g"), [ch('g'), ch('g')]);
    assert_eq!(
        chord("<ctrl-x><ctrl-s>"),
        chord("ctrl-x ctrl-s"),
        "a sequence of modified keys"
    );
    assert_eq!(chord("<ctrl-q>"), chord("ctrl-q"));
    assert_eq!(chord("<lt>"), [ch('<')]);
    assert_eq!(chord("<"), [ch('<')]);
    assert_eq!(chord("<?>"), [ch('?')]);
    assert_eq!(chord("</>"), [ch('/')]);
}

#[test]
fn single_characters_keep_their_case() {
    assert_eq!(chord("G"), [ch('G')]);
    assert_eq!(chord("g"), [ch('g')]);
    assert_eq!(chord("shift-g"), [ch('G')]);
    assert_eq!(chord("<shift-g>"), [ch('G')]);
    assert_eq!(chord("F"), [ch('F')]);
    assert_ne!(chord("A"), chord("a"));
}

#[test]
fn modifiers_and_named_keys_are_case_insensitive() {
    let ctrl_a = key(KeyCode::Char('a'), KeyModifiers::CONTROL);
    assert_eq!(chord("CTRL-a"), [ctrl_a]);
    assert_eq!(chord("<Ctrl-A>"), [ctrl_a]);
    assert_eq!(chord("AlT-eNtEr"), [key(KeyCode::Enter, KeyModifiers::ALT)]);
    assert_eq!(chord("<Esc>"), [key(KeyCode::Esc, KeyModifiers::NONE)]);
    assert_eq!(chord("pgdn"), chord("PageDown"));
    assert_eq!(chord("left"), chord("←"));
    assert_eq!(
        chord("ctrl-alt-a"),
        [key(
            KeyCode::Char('a'),
            KeyModifiers::CONTROL | KeyModifiers::ALT
        )]
    );
    assert_eq!(
        chord("ctrl-shift-enter"),
        [key(
            KeyCode::Enter,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        )]
    );
    // The old config's way of writing it, and the new one.
    assert_eq!(chord("<shift-backtab>"), chord("shift-tab"));
    assert_eq!(chord("space"), [ch(' ')]);
    assert_eq!(chord("minus"), [ch('-')]);
    assert_eq!(chord("F12"), [key(KeyCode::F(12), KeyModifiers::NONE)]);
}

#[test]
fn invalid_keys() {
    for raw in [
        "",
        "invalid-key",
        "ctrl-invalid-key",
        "f13",
        "<ctrl-notakey>",
    ] {
        assert!(parse_key_sequence(raw).is_err(), "{raw:?}");
    }
    let err = parse_key_sequence("<g><nope>").unwrap_err();
    assert!(err.contains("nope"), "{err}");
}

#[test]
fn normalise_drops_what_terminals_disagree_on() {
    let shifted = key(KeyCode::Char('G'), KeyModifiers::SHIFT);
    assert_eq!(normalise(shifted), ch('G'));
    assert_eq!(
        normalise(key(KeyCode::Char('?'), KeyModifiers::SHIFT)),
        ch('?')
    );
    // Ctrl lower-cases, and keeps Shift so ctrl-shift-z is not ctrl-z.
    assert_eq!(
        normalise(key(KeyCode::Char('D'), KeyModifiers::CONTROL)),
        key(KeyCode::Char('d'), KeyModifiers::CONTROL)
    );
    assert_ne!(chord("ctrl-shift-z"), chord("ctrl-z"));
    // AltGr characters are typed, letters with Ctrl+Alt are shortcuts.
    let altgr = KeyModifiers::CONTROL | KeyModifiers::ALT;
    assert_eq!(normalise(key(KeyCode::Char('~'), altgr)), ch('~'));
    assert_eq!(
        normalise(key(KeyCode::Char('a'), altgr)),
        key(KeyCode::Char('a'), altgr)
    );
    assert_eq!(
        normalise(key(KeyCode::BackTab, KeyModifiers::NONE)),
        key(KeyCode::BackTab, KeyModifiers::SHIFT)
    );
    assert_eq!(
        normalise(key(KeyCode::Down, KeyModifiers::SUPER)),
        key(KeyCode::Down, KeyModifiers::NONE)
    );
}

/// A key the parser can write, with any modifiers.
fn any_key() -> impl Strategy<Value = KeyEvent> {
    let codes = prop_oneof![
        proptest::char::range('!', '~').prop_map(KeyCode::Char),
        Just(KeyCode::Char(' ')),
        (1..=12u8).prop_map(KeyCode::F),
        proptest::sample::select(vec![
            KeyCode::Enter,
            KeyCode::Esc,
            KeyCode::Tab,
            KeyCode::BackTab,
            KeyCode::Backspace,
            KeyCode::Delete,
            KeyCode::Insert,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::PageUp,
            KeyCode::PageDown,
        ]),
    ];
    (codes, 0..8u8).prop_map(|(code, bits)| {
        let mut mods = KeyModifiers::NONE;
        for (bit, m) in [
            KeyModifiers::CONTROL,
            KeyModifiers::ALT,
            KeyModifiers::SHIFT,
        ]
        .into_iter()
        .enumerate()
        {
            if bits & (1 << bit) != 0 {
                mods |= m;
            }
        }
        normalise(KeyEvent::new(code, mods))
    })
}

proptest! {
    #[test]
    fn parse_and_print_round_trip(keys in proptest::collection::vec(any_key(), 1..4)) {
        let text = chord_to_string(&keys);
        prop_assert_eq!(parse_key_sequence(&text), Ok(keys), "{}", text);
    }
}

#[test]
fn user_override_and_unbind_merge() {
    let config = Config::with_user_file(
        "user.json",
        r#"{"keybindings": {"Results": {"<h>": null, "<x>": "CopyCell"}}}"#,
    )
    .unwrap();
    let keymap = &config.keymap;
    let results = [KeyContext::Results];
    assert_eq!(keymap.resolve(&results, &chord("h")), Lookup::None);
    assert_eq!(
        keymap.resolve(&results, &chord("x")),
        Lookup::Match(Action::CopyCell)
    );
    // The other defaults stay, including the other chord for Left.
    assert_eq!(
        keymap.resolve(&results, &chord("←")),
        Lookup::Match(Action::Left)
    );
    assert_eq!(
        keymap.resolve(&results, &chord("y")),
        Lookup::Match(Action::CopyCell)
    );
    let defaults = Config::embedded().keymap;
    for context in KeyContext::iter().filter(|c| *c != KeyContext::Results) {
        let mine: HashMap<_, _> = keymap.bindings(context).collect();
        let theirs: HashMap<_, _> = defaults.bindings(context).collect();
        assert_eq!(mine, theirs, "{context}");
    }
}

#[test]
fn unknown_action_is_a_load_error_with_location() {
    let path = "/home/someone/.config/qry/config.json";
    let err = Config::with_user_file(path, r#"{"keybindings": {"Results": {"<x>": "Teleport"}}}"#)
        .unwrap_err()
        .to_string();
    for part in [path, "Results", "<x>", "Teleport"] {
        assert!(err.contains(part), "{part} missing from: {err}");
    }
}

#[test]
fn prefix_conflict_reported_once() {
    let config = Config::with_user_file(
        "user.json",
        r#"{"keybindings": {"Results": {"<g>": "Refresh"}}}"#,
    )
    .unwrap();
    let warnings = config.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("Results `g` and `g g`"),
        "{}",
        warnings[0]
    );

    // `g` waits for a second `g`; only the timeout runs it.
    let results = [KeyContext::Results];
    assert_eq!(config.keymap.resolve(&results, &chord("g")), Lookup::Prefix);
    assert_eq!(
        config.keymap.exact(&results, &chord("g")),
        Some(Action::Refresh)
    );
    assert_eq!(
        config.keymap.resolve(&results, &chord("g g")),
        Lookup::Match(Action::Top)
    );
}

#[test]
fn resolve_overlay_contexts_are_exclusive() {
    let keymap = Config::embedded().keymap;
    assert_eq!(
        keymap.resolve(&[KeyContext::Help], &chord("ctrl-n")),
        Lookup::None
    );
    assert_eq!(
        keymap.resolve(&[KeyContext::Dialog], &chord("ctrl-n")),
        Lookup::None
    );
    assert_eq!(
        keymap.resolve(&[KeyContext::Dialog], &chord("ctrl-q")),
        Lookup::Match(Action::Quit)
    );
    assert_eq!(
        keymap.resolve(&[KeyContext::Tree, KeyContext::Global], &chord("ctrl-n")),
        Lookup::Match(Action::NewConnection)
    );
}

#[test]
fn pane_binding_shadows_global() {
    let config = Config::with_user_file(
        "user.json",
        r#"{"keybindings": {"Editor": {"<tab>": "Indent"}}}"#,
    )
    .unwrap();
    let tab = chord("Tab");
    assert_eq!(
        config
            .keymap
            .resolve(&[KeyContext::Editor, KeyContext::Global], &tab),
        Lookup::Match(Action::Indent)
    );
    assert_eq!(
        config
            .keymap
            .resolve(&[KeyContext::Tree, KeyContext::Global], &tab),
        Lookup::Match(Action::FocusNext)
    );

    let help = HelpState::from_config(&config);
    let line = |title: &str, desc: &str| {
        let group = help.groups.iter().find(|g| g.title == title).unwrap();
        group
            .entries
            .iter()
            .find(|(_, d)| d == desc)
            .map(|(keys, _)| keys.clone())
    };
    assert_eq!(line("Editor · Editing", "indent").as_deref(), Some("Tab"));
    assert_eq!(line("Global", "next pane").as_deref(), Some("Tab"));
}

#[test]
fn chords_for_lists_every_chord_plain_first() {
    let keymap = Config::embedded().keymap;
    let strings = |context, action| -> Vec<String> {
        keymap
            .chords_for(context, &action)
            .iter()
            .map(|c| chord_to_string(c))
            .collect()
    };
    assert_eq!(
        strings(KeyContext::Editor, Action::RunStatement),
        ["F5", "ctrl-Enter"]
    );
    assert_eq!(strings(KeyContext::Results, Action::Up), ["k", "↑"]);
    assert!(strings(KeyContext::Editor, Action::Help).is_empty());
}

#[test]
fn the_pill_follows_the_context() {
    assert_eq!(KeyContext::Editor.pill(), Some(Pill::Edit));
    assert_eq!(KeyContext::Palette.pill(), Some(Pill::Command));
    for context in [
        KeyContext::Tree,
        KeyContext::Results,
        KeyContext::Inspector,
        KeyContext::Jobs,
    ] {
        assert_eq!(context.pill(), Some(Pill::Normal), "{context}");
    }
    for context in [KeyContext::Help, KeyContext::Dialog, KeyContext::Global] {
        assert_eq!(context.pill(), None, "{context}");
    }
}
