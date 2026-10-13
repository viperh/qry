//! The status panel under the panes.

use ratatui::{prelude::*, widgets::Paragraph};

use crate::{
    app::state::{State, StatusCode},
    theme::Theme,
};

/// Whether there is nothing to show, so the panel takes no room.
pub(crate) fn is_empty(state: &State) -> bool {
    state.status == StatusCode::None
}

pub(crate) fn draw(state: &State, theme: &Theme, frame: &mut Frame, area: Rect) {
    match &state.status {
        StatusCode::Error(err) => frame.render_widget(
            Paragraph::new(format!("Error: {err}"))
                .style(theme.status_warning())
                .centered(),
            area,
        ),
        StatusCode::Success(msg) => frame.render_widget(
            Paragraph::new(msg.as_str())
                .style(theme.status_ok())
                .centered(),
            area,
        ),
        StatusCode::None => {}
    }
}
