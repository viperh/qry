//! The query editor.

use ratatui::prelude::*;

use crate::{app::state::State, theme::Theme};

/// Draws the text area as the reducer styled it for the focus and theme.
pub(crate) fn draw(state: &State, _theme: &Theme, frame: &mut Frame, area: Rect) {
    frame.render_widget(&state.editor().textarea, area);
}

/// `ln 4, col 12` for the title row, counted from 1.
pub(crate) fn position(state: &State) -> String {
    let cursor = state.editor().textarea.cursor();
    let (row, col) = (cursor.0, cursor.1);
    format!("ln {}, col {}", row + 1, col + 1)
}
