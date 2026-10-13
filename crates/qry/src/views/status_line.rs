//! The status line, after tachy's `status.rs`: the active session on the
//! left (connection, driver, database, transaction state), the grid
//! position on the right, and the last message between them.
//!
//! When it is narrow, items go in a fixed order: the transaction state,
//! the database and the driver, to make room for the message first. The
//! connection's name stays, and a message too long for what is left is cut
//! with `…`. It never wraps.

use ratatui::{prelude::*, widgets::Paragraph};

use crate::{
    app::state::{SessionStatus, State, StatusCode},
    theme::Theme,
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
    let mut items = vec![
        name,
        vec![
            sep(),
            Span::styled(session.driver.name(), theme.status_line()),
        ],
    ];
    if let Some(database) = &session.database {
        items.push(vec![
            sep(),
            Span::styled(database.clone(), theme.status_line()),
        ]);
    }
    if session.is_open() {
        items.push(vec![
            sep(),
            if session.in_transaction {
                Span::styled("in transaction", theme.status_warning())
            } else {
                Span::styled("auto-commit", theme.status_dim())
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

/// `1204` as `1 204`.
pub(crate) fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

pub(crate) fn draw(state: &State, theme: &Theme, frame: &mut Frame, area: Rect) {
    let room = usize::from(area.width);
    let right: Vec<Span> = position(state)
        .map(|p| vec![Span::styled(p, theme.status_line())])
        .unwrap_or_default();
    let message = match &state.status {
        StatusCode::Error(err) => Some(Span::styled(
            format!("Error: {err}"),
            theme.status_warning(),
        )),
        StatusCode::Success(msg) => Some(Span::styled(msg.clone(), theme.status_ok())),
        StatusCode::None => None,
    };
    let message_width = message.as_ref().map_or(0, |m| m.width() + 2);
    let mut items = items(state, theme);
    // Drop from the end until the left, the message and the right fit,
    // with a space between.
    while items.len() > 1 && width(&items.concat()) + message_width + width(&right) + 1 > room {
        items.pop();
    }
    let mut left = items.concat();
    if let Some(message) = message {
        let free = room.saturating_sub(width(&left) + width(&right) + 4);
        if free >= 8 {
            let text: String = if message.width() > free {
                let mut cut: String = message.content.chars().take(free - 1).collect();
                cut.push('…');
                cut
            } else {
                message.content.to_string()
            };
            left.push(Span::raw("  "));
            left.push(Span::styled(text, message.style));
        }
    }

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
