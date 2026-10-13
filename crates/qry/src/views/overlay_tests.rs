//! Unit tests for `overlay.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ratatui::widgets::Paragraph;

use super::*;
use crate::{
    testing::{buffer_lines, render_with},
    views::layout::centered_rect,
};

#[test]
fn centered_rect_is_centred_and_clamped() {
    let area = Rect::new(0, 0, 80, 24);
    assert_eq!(centered_rect(area, 40, 10), Rect::new(20, 7, 40, 10));
    assert_eq!(
        centered_rect(area, 41, 11),
        Rect::new(20, 7, 41, 11),
        "odd sizes round right"
    );
    assert_eq!(centered_rect(area, 100, 30), area, "wider than the area");
    let offset = Rect::new(10, 5, 20, 10);
    assert_eq!(centered_rect(offset, 10, 4), Rect::new(15, 8, 10, 4));
}

#[test]
fn dialog_frame_returns_inner_rect_one_cell_in() {
    let theme = Theme::DARK;
    let mut inner = Rect::default();
    let buffer = render_with(40, 10, |frame| {
        inner = dialog_frame(frame, &theme, Rect::new(5, 2, 20, 6), "Title", Accent::Act);
    });
    assert_eq!(inner, Rect::new(6, 3, 18, 4));
    let lines = buffer_lines(&buffer);
    assert!(lines[2].contains("┌ Title ─"), "{lines:#?}");
    assert_eq!(buffer[(5, 2)].fg, theme.amber, "the accent");
    assert_eq!(buffer[(7, 2)].fg, theme.amber, "the title in the accent");
    assert_eq!(buffer[(10, 4)].bg, theme.surface_raised);
}

#[test]
fn backdrop_dims_every_cell_outside_the_overlay() {
    let theme = Theme::DARK;
    let buffer = render_with(20, 4, |frame| {
        let text = vec![Line::raw("abcdefghijklmnopqrst"); 4];
        frame.render_widget(Paragraph::new(text).style(theme.base()), frame.area());
        let area = frame.area();
        apply_backdrop(frame.buffer_mut(), area, &theme);
    });
    for (x, y) in [(0, 0), (19, 0), (0, 3), (19, 3), (7, 2)] {
        let cell = &buffer[(x, y)];
        assert!(cell.modifier.contains(Modifier::DIM), "({x},{y})");
        assert_eq!(cell.fg, theme.fg_dim, "({x},{y})");
    }
    assert_eq!(
        buffer_lines(&buffer)[1],
        "abcdefghijklmnopqrst",
        "symbols kept"
    );
}

#[test]
fn spinner_frames_are_braille() {
    assert_eq!(SPINNER.len(), 10);
    assert_eq!(SPINNER[0], "⠋");
}
