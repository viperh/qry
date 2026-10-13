//! What every overlay is drawn with: the dimmed backdrop over the main
//! screen, and the frame around a dialog, after tachy's `dialogs/mod.rs`.
//!
//! The order is fixed in [`super::draw`]: the main screen, the backdrop
//! (only while an overlay is open), the overlay, then a toast on top.

use ratatui::{
    buffer::Buffer,
    prelude::*,
    widgets::{Block, Clear},
};

use crate::theme::{Accent, Theme};

/// The braille spinner a waiting dialog shows, one frame per 100 ms.
pub(crate) const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Clears `rect`, fills it with the dialog colours, draws a one-cell
/// border and ` title ` in the accent, and returns the inside.
pub(crate) fn dialog_frame(
    frame: &mut Frame,
    theme: &Theme,
    rect: Rect,
    title: &str,
    accent: Accent,
) -> Rect {
    let colour = theme.accent(accent);
    let block = Block::bordered()
        .style(theme.dialog())
        .border_style(theme.dialog_border(colour))
        .title(Line::styled(
            format!(" {title} "),
            theme.dialog_title(colour),
        ));
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);
    inner
}

/// Restyles every cell of `area` already drawn with the backdrop style,
/// keeping what it shows, so the screen reads as a dimmed layer under an
/// overlay.
pub(crate) fn apply_backdrop(buf: &mut Buffer, area: Rect, theme: &Theme) {
    let area = area.intersection(buf.area);
    let style = theme.backdrop();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buf[(x, y)];
            cell.set_style(style);
            if let Some(fg) = style.fg {
                cell.fg = fg;
            }
        }
    }
}

#[cfg(test)]
#[path = "overlay_tests.rs"]
mod overlay_tests;
