//! The query editor.

use ratatui::prelude::*;

use crate::{app::state::State, theme::Theme};

/// Draws the text area as the reducer styled it for the focus and theme.
pub(crate) fn draw(state: &State, _theme: &Theme, frame: &mut Frame, area: Rect) {
    frame.render_widget(&state.editor().textarea, area);
}
