//! Unit tests for `help.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::{
    config::Config,
    help_model,
    testing::{buffer_lines, render_with},
};

fn help() -> HelpState {
    HelpState::from_config(&Config::embedded())
}

fn lines(help: &HelpState, width: u16, height: u16) -> Vec<String> {
    let buffer = render_with(width, height, |frame| {
        draw(help, &Theme::DARK, frame, frame.area());
    });
    buffer_lines(&buffer)
}

#[test]
fn column_count_thresholds() {
    assert_eq!(column_count(80), 1);
    assert_eq!(column_count(99), 1);
    assert_eq!(column_count(100), 2);
    assert_eq!(column_count(159), 2);
    assert_eq!(column_count(160), 3);
}

#[test]
fn pack_balances_columns() {
    let (packed, limit) = pack(&[10, 3, 3, 3, 10, 1], 2);
    assert_eq!(limit, 16);
    assert_eq!(packed, [vec![0, 1, 2], vec![3, 4, 5]]);
    // One column takes everything.
    assert_eq!(pack(&[10, 3, 3], 1).0, [vec![0, 1, 2]]);
}

#[test]
fn pack_keeps_continuation_with_parent() {
    let groups = vec![
        HelpGroup {
            title: "A",
            entries: vec![("a".into(), "one".into()); 5],
            notes: vec![],
            continues: false,
        },
        HelpGroup {
            title: "B",
            entries: vec![("b".into(), "two".into()); 5],
            notes: vec![],
            continues: false,
        },
        HelpGroup {
            title: "B more",
            entries: vec![("c".into(), "three".into()); 5],
            notes: vec![],
            continues: true,
        },
    ];
    let state = HelpState {
        groups,
        ..HelpState::default()
    };
    let shown = lines(&state, 120, 20);
    let row = shown.iter().position(|l| l.contains("B more")).unwrap();
    let col = shown[row].find("B more").unwrap();
    let parent_row = shown
        .iter()
        .position(|l| l.contains(" B ") || l.trim_start().starts_with("│ B"))
        .unwrap();
    assert_eq!(shown[parent_row].find('B').unwrap(), col, "the same column");
    assert_eq!(row, parent_row + 6, "right under it, with no blank line");
}

#[test]
fn key_column_capped_at_half_and_truncated_with_ellipsis() {
    let groups = vec![HelpGroup {
        title: "Long",
        entries: vec![
            ("k".repeat(80), "d".repeat(80)),
            ("x".into(), "short".into()),
        ],
        notes: vec![],
        continues: false,
    }];
    let state = HelpState {
        groups,
        ..HelpState::default()
    };
    let shown = lines(&state, 80, 24);
    let row = shown.iter().find(|l| l.contains("kkk")).unwrap();
    let keys = row.split_whitespace().nth(1).unwrap();
    // 80 wide: 76 usable, half is 38.
    assert_eq!(keys.chars().count(), 38, "{row}");
    assert!(keys.ends_with('…'), "{row}");
    assert!(row.contains("ddd") && row.contains('…'), "{row}");
}

#[test]
fn scroll_clamps_and_g_reaches_last_page() {
    let state = help();
    let first = lines(&state, 80, 24);
    let max = state.max_scroll.get();
    assert!(max > 0, "the help is taller than 80x24");
    state.scroll(crate::action::Action::Up);
    assert_eq!(state.scroll.get(), 0);
    state.scroll(crate::action::Action::Bottom);
    assert_eq!(state.scroll.get(), max);
    let last = lines(&state, 80, 24);
    let indicator = last[23].clone();
    let total = indicator
        .split(" of ")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap();
    assert!(
        indicator.contains(&format!("–{total} of {total}")),
        "{indicator}"
    );
    assert_ne!(first, last);
    // A bigger screen shows more, so the offset is clamped again.
    lines(&state, 300, 100);
    assert_eq!(state.scroll.get(), state.max_scroll.get());
}

#[test]
fn position_indicator_only_when_scrollable() {
    let state = help();
    let wide = lines(&state, 300, 100);
    assert!(!wide[99].contains(" of "), "{}", wide[99]);
    let top = lines(&state, 80, 24);
    assert!(top[23].contains(" 1–"), "{}", top[23]);
}

#[test]
fn footer_does_not_scroll() {
    let state = help();
    let top = lines(&state, 80, 24);
    state.scroll(crate::action::Action::Bottom);
    let bottom = lines(&state, 80, 24);
    let footer = |l: &[String]| l.iter().rev().skip(1).take(3).cloned().collect::<Vec<_>>();
    assert_eq!(footer(&top), footer(&bottom));
    let pinned = footer(&top);
    for label in ["run:", "results:", "tree:"] {
        assert!(
            pinned.iter().any(|l| l.contains(label)),
            "{label}: {top:#?}"
        );
    }
}

#[test]
fn bottom_border_keys_follow_help_keymap() {
    let mut config = Config::embedded();
    config.keymap.bind(
        crate::keymap::KeyContext::Help,
        &crate::testing::parse_keys("n"),
        crate::action::Action::Down,
    );
    for chord in config.keymap.chords_for(
        crate::keymap::KeyContext::Help,
        &crate::action::Action::Down,
    ) {
        if crate::keymap::chord_to_string(&chord) == "j" {
            config
                .keymap
                .unbind(crate::keymap::KeyContext::Help, &chord);
        }
    }
    let state = HelpState::from_config(&config);
    let shown = lines(&state, 120, 40);
    assert!(shown[39].contains("k/n scroll"), "{}", shown[39]);
    let default = lines(&help(), 120, 40);
    assert!(
        default[39].contains("k/j scroll  PgUp/PgDn page  Home/G top/bottom  ?/Esc close"),
        "{}",
        default[39]
    );
    let _ = help_model::help_keys(&config.keymap);
}

#[test]
fn renders_at_every_size_without_panic() {
    let state = help();
    for (w, h) in [
        (0, 0),
        (1, 1),
        (3, 2),
        (10, 3),
        (80, 24),
        (120, 40),
        (160, 48),
        (300, 100),
    ] {
        lines(&state, w, h);
    }
}
