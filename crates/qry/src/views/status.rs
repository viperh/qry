//! The status line under the panes: the active session, then the last
//! message.

use ratatui::{prelude::*, widgets::Paragraph};

use crate::{
    app::state::{SessionStatus, State, StatusCode},
    theme::Theme,
};

pub(crate) fn draw(state: &State, theme: &Theme, frame: &mut Frame, area: Rect) {
    let session = state.sessions.active();
    let mut spans = match &session.status {
        SessionStatus::Open { label } => {
            vec![Span::styled(format!(" {label} "), theme.status_session())]
        }
        SessionStatus::Connecting => vec![Span::styled(
            format!(" connecting to {}… ", session.title),
            theme.status_dim(),
        )],
        SessionStatus::Failed(message) => vec![Span::styled(
            format!(" {}: {message} ", session.title),
            theme.status_warning(),
        )],
        SessionStatus::Closed => Vec::new(),
    };
    match &state.status {
        StatusCode::Error(err) => {
            spans.push(Span::styled(
                format!(" Error: {err}"),
                theme.status_warning(),
            ));
        }
        StatusCode::Success(msg) => spans.push(Span::styled(format!(" {msg}"), theme.status_ok())),
        StatusCode::None => {}
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(theme.status_line()),
        area,
    );
}
