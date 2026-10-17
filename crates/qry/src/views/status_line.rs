//! The status line, after tachy's `status.rs`: the active session on the
//! left (connection, driver, database, transaction state) and the grid
//! position on the right. Messages are toasts.
//!
//! When it is narrow, items go in a fixed order: the transaction state,
//! the database and the driver. The connection's name stays. It never
//! wraps.

use ratatui::{prelude::*, widgets::Paragraph};

use crate::{
    app::state::{SessionStatus, State},
    theme::{Pill, Theme},
};

fn width(spans: &[Span]) -> usize {
    spans.iter().map(Span::width).sum()
}

/// The left-hand items, most important first, each with its separator.
fn items(state: &State, theme: &Theme) -> Vec<Vec<Span<'static>>> {
    let session = state.sessions.active();
    let sep = || Span::styled(" · ", theme.status_dim());
    let name = match &session.status {
        SessionStatus::Failed(error) => vec![Span::styled(
            format!(" {}: {error}", session.title),
            theme.status_warning(),
        )],
        SessionStatus::Connecting => vec![Span::styled(
            format!(" connecting to {}…", session.title),
            theme.status_dim(),
        )],
        SessionStatus::Open { .. } | SessionStatus::Closed => {
            vec![Span::styled(
                format!(" {}", session.title),
                theme.status_session(),
            )]
        }
    };
    let mut items = vec![name];
    // A running statement comes straight after the session's name.
    if let Some(running) = crate::views::results::running(state) {
        items.push(vec![sep(), Span::styled(running, theme.status_jobs())]);
    }
    items.extend([vec![
        sep(),
        Span::styled(session.driver.name(), theme.status_line()),
    ]]);
    if let Some(database) = &session.database {
        items.push(vec![
            sep(),
            Span::styled(database.clone(), theme.status_line()),
        ]);
    }
    if session.is_open() {
        items.push(vec![
            sep(),
            if session.tx_open() {
                Span::styled(" TX ", theme.mode_pill(Pill::Edit))
            } else {
                Span::styled("auto", theme.status_dim())
            },
        ]);
    }
    items
}

/// `R 12 / 1 204  C 3/9` while the grid holds a result with rows.
fn position(state: &State) -> Option<String> {
    let results = state.results();
    let result = results.result.as_ref()?;
    if result.rows.is_empty() || result.columns.is_empty() {
        return None;
    }
    Some(format!(
        "R {} / {}  C {}/{} ",
        thousands(results.row + 1),
        thousands(result.rows.len()),
        results.col + 1,
        result.columns.len()
    ))
}

/// `1204` as `1 204`, with a thin space.
pub(crate) fn thousands(n: usize) -> String {
    qry_core::jobs::format_rows(u64::try_from(n).unwrap_or(u64::MAX))
}

pub(crate) fn draw(state: &State, theme: &Theme, frame: &mut Frame, area: Rect) {
    let room = usize::from(area.width);
    let right: Vec<Span> = position(state)
        .map(|p| vec![Span::styled(p, theme.status_line())])
        .unwrap_or_default();
    let mut items = items(state, theme);
    // Drop from the end until the left and the right fit, a space apart.
    while items.len() > 1 && width(&items.concat()) + width(&right) + 1 > room {
        items.pop();
    }
    let left = items.concat();

    frame.render_widget(
        Paragraph::new(Line::from(left)).style(theme.status_line()),
        area,
    );
    let right_width = u16::try_from(width(&right)).unwrap_or(u16::MAX);
    if right_width > 0 && right_width < area.width {
        let at = Rect::new(area.right() - right_width, area.y, right_width, 1);
        frame.render_widget(Line::from(right), at);
    }
}

#[cfg(test)]
#[path = "status_line_tests.rs"]
mod status_line_tests;
