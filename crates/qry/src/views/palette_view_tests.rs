//! Unit tests for `palette.rs`: where it goes and what it looks like.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::{
    app::UiEvent,
    config::Config,
    testing::{AppHarness, styled},
};

fn opened() -> AppHarness {
    let mut h = AppHarness::new(Config::embedded());
    h.send(UiEvent::Started);
    h.keys("ctrl-p");
    h
}

#[test]
fn palette_rect_geometry() {
    let area = |w| Rect::new(0, 0, w, 40);
    assert_eq!(palette_rect(area(80), 5), Rect::new(4, 3, 72, 8));
    assert_eq!(palette_rect(area(104), 5), Rect::new(4, 3, 96, 8));
    assert_eq!(palette_rect(area(200), 5), Rect::new(52, 3, 96, 8));
    assert_eq!(palette_rect(area(200), 50).height, 20, "at most 20 rows");
    assert_eq!(
        palette_rect(Rect::new(0, 0, 80, 10), 50).height,
        7,
        "the screen's height"
    );
}

#[test]
fn matched_characters_are_amber_and_the_selection_purple() {
    let mut h = opened();
    h.type_str("exp");
    let buffer = h.render_buffer(120, 40);
    let theme = *h.theme();
    let lines = crate::testing::buffer_lines(&buffer);
    let y = lines.iter().position(|l| l.contains("Export")).unwrap();
    let x = lines[y][..lines[y].find("Export").unwrap()].chars().count();
    let cell = &buffer[(u16::try_from(x).unwrap(), u16::try_from(y).unwrap())];
    assert_eq!(cell.fg, theme.amber, "the matched E");
    assert_eq!(cell.bg, theme.purple_tint, "on the selected row");
    let corner = &buffer[(12, 3)];
    assert_eq!(corner.symbol(), "┌");
    assert_eq!(corner.fg, theme.purple);
}

#[test]
fn palette_snapshots() {
    let mut h = opened();
    h.resize(80, 24);
    insta::assert_snapshot!("palette_empty_80x24", h.render(80, 24));
    insta::assert_snapshot!(
        "palette_empty_80x24_styled",
        styled(&h.render_buffer(80, 24))
    );

    let mut h = opened();
    h.resize(120, 40);
    h.type_str("exp");
    insta::assert_snapshot!("palette_query_exp_120x40", h.render(120, 40));
    insta::assert_snapshot!(
        "palette_query_exp_120x40_styled",
        styled(&h.render_buffer(120, 40))
    );

    let mut h = opened();
    h.resize(120, 40);
    h.type_str("export csv").keys("enter");
    insta::assert_snapshot!("palette_arg_needed_120x40", h.render(120, 40));

    let mut h = opened();
    h.resize(120, 40);
    for _ in 0..20 {
        h.keys("down");
    }
    let text = h.render(120, 40);
    assert!(
        !text.contains(" Quit "),
        "scrolled past the first rows: {text}"
    );
    insta::assert_snapshot!("palette_scrolled_120x40", text);
}

#[test]
fn renders_at_every_size_without_panic() {
    let mut h = opened();
    for (w, ht) in [(0, 0), (1, 1), (10, 3), (80, 24), (300, 100)] {
        h.render(w, ht);
    }
}
