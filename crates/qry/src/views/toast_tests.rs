//! Unit tests for `toast.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ratatui::buffer::Buffer;

use super::*;
use crate::{
    notice::Notice,
    testing::{buffer_lines, render_with, styled},
};

fn toast(notice: Notice, count: u32) -> Toast {
    Toast { notice, count }
}

fn drawn(toast: &Toast, queued: usize, width: u16) -> Buffer {
    let theme = Theme::DARK;
    render_with(width, 1, |frame| {
        let area = frame.area();
        frame.render_widget(line(toast, queued, Some("ctrl-l"), &theme, width), area);
    })
}

fn text(toast: &Toast, queued: usize, width: u16) -> String {
    buffer_lines(&drawn(toast, queued, width))[0].clone()
}

#[test]
fn formats_with_padding_ellipsis_and_queue_count() {
    let info = toast(Notice::info("Fetched 204 rows in 38 ms"), 1);
    let wide = text(&info, 2, 120);
    assert!(wide.starts_with(" Fetched 204 rows in 38 ms "), "{wide:?}");
    assert!(wide.ends_with("  (+2) "), "{wide:?}");
    assert_eq!(wide.chars().count(), 120);

    let long = toast(Notice::warning("x".repeat(100)), 3);
    let narrow = text(&long, 0, 40);
    assert!(narrow.starts_with(" ! xxx"), "{narrow:?}");
    assert!(narrow.contains("…"), "{narrow:?}");
    assert!(narrow.contains(" ×3"), "{narrow:?}");
    assert_eq!(narrow.chars().count(), 40);
    assert!(
        !narrow.contains("ctrl-l"),
        "only errors point to the messages"
    );
}

#[test]
fn truncated_error_shows_messages_hint() {
    let error = toast(
        Notice::error("cannot save connections: permission denied (os error 13)"),
        1,
    );
    let short = text(&error, 0, 40);
    assert!(short.contains("…") && short.contains("ctrl-l"), "{short:?}");
    let full = text(&error, 0, 120);
    assert!(!full.contains("ctrl-l"), "nothing was cut: {full:?}");
}

#[test]
fn level_styles_and_prefixes() {
    let theme = Theme::DARK;
    let cases = [
        (Notice::info("i"), " i", theme.fg),
        (Notice::success("s"), " ✓ s", theme.green),
        (Notice::warning("w"), " ! w", theme.coral),
        (Notice::error("e"), " ✗ e", theme.coral),
    ];
    for (notice, start, mark) in cases {
        let level = notice.level;
        let buffer = drawn(&toast(notice, 1), 0, 30);
        assert!(buffer_lines(&buffer)[0].starts_with(start), "{level:?}");
        assert_eq!(buffer[(1, 0)].fg, mark, "{level:?}");
        assert_eq!(buffer[(1, 0)].bg, theme.bg, "{level:?}");
    }
    let error = drawn(&toast(Notice::error("e"), 1), 0, 30);
    assert!(error[(3, 0)].modifier.contains(Modifier::BOLD) || error[(3, 0)].fg == theme.coral);
}

#[test]
fn toast_snapshots() {
    let cases = [
        (
            "toast_info_120x1",
            toast(Notice::info("Fetched 204 rows in 38 ms"), 1),
            0,
            120,
        ),
        (
            "toast_success_120x1",
            toast(Notice::success("Connected to app@db.local:5432/app"), 1),
            0,
            120,
        ),
        (
            "toast_error_queued_120x1",
            toast(
                Notice::error("cannot save connections: permission denied (os error 13)"),
                1,
            ),
            2,
            120,
        ),
        (
            "toast_error_truncated_80x1",
            toast(
                Notice::error(
                    "Connection failed: could not connect to server: Connection refused. Is the server running on host db.local and accepting TCP/IP connections on port 5432?",
                ),
                2,
            ),
            1,
            80,
        ),
    ];
    for (name, toast, queued, width) in cases {
        insta::assert_snapshot!(name, text(&toast, queued, width));
        insta::assert_snapshot!(
            format!("{name}_styled"),
            styled(&drawn(&toast, queued, width))
        );
    }
}
