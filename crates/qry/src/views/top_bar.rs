//! The top bar: ` qry `, one tab per session, the active connection and
//! the mode pill, after tachy's `top_bar.rs`.
//!
//! When it is narrow, the connection summary goes first. Then tabs give
//! way to `…` from the far side, but the active tab always stays.

use ratatui::{prelude::*, widgets::Paragraph};

use crate::{
    app::state::{SessionState, State},
    theme::Theme,
    views::layout::TabHits,
};

const APP_NAME: &str = " qry ";
const ELLIPSIS: &str = " … ";

/// What a tab says: its number and title, and a mark while a query runs
/// there.
fn label(number: usize, session: &SessionState) -> String {
    let running = if session.busy() { " ◐" } else { "" };
    format!(" {number} {}{running} {}", session.title, tag(session))
}

/// The connection's label after the title, `[prod] `, or nothing.
fn tag(session: &SessionState) -> String {
    session
        .label
        .map(|l| format!("[{}] ", l.name()))
        .unwrap_or_default()
}

fn width(text: &str) -> u16 {
    u16::try_from(Span::raw(text).width()).unwrap_or(u16::MAX)
}

/// The tabs that fit in `room` columns, by position, with whether a `…`
/// goes before and after them.
fn visible_tabs(widths: &[u16], active: usize, room: u16) -> (usize, usize, bool, bool) {
    let ellipsis = width(ELLIPSIS);
    let total: u16 = widths.iter().sum();
    if total <= room {
        return (0, widths.len(), false, false);
    }
    // Grow a window around the active tab, to the right first, while the
    // tabs and the `…` they leave out still fit.
    let (mut first, mut end) = (active, active + 1);
    let cost = |first: usize, end: usize| {
        let lead = if first > 0 { ellipsis } else { 0 };
        let tail = if end < widths.len() { ellipsis } else { 0 };
        widths[first..end].iter().sum::<u16>() + lead + tail
    };
    loop {
        if end < widths.len() && cost(first, end + 1) <= room {
            end += 1;
        } else if first > 0 && cost(first - 1, end) <= room {
            first -= 1;
        } else {
            break;
        }
    }
    (first, end, first > 0, end < widths.len())
}

/// Where each tab goes in a top bar drawn in `area`, by position. The click
/// handler uses the same function, so it hits what was drawn.
pub(crate) fn tab_hits(state: &State, area: Rect) -> TabHits {
    layout_tabs(state, area).0
}

/// The tabs' rects, and the spans to draw from the app name up to the
/// last tab.
fn layout_tabs(state: &State, area: Rect) -> (TabHits, Vec<(u16, String, Option<usize>)>) {
    let sessions = &state.sessions;
    let labels: Vec<String> = sessions
        .iter()
        .enumerate()
        .map(|(i, session)| label(i + 1, session))
        .collect();
    let widths: Vec<u16> = labels.iter().map(|l| width(l)).collect();
    let active = sessions
        .order
        .iter()
        .position(|id| *id == sessions.active)
        .unwrap_or(0);
    let pill = width(&format!(" {} ", state.pill().label())) + 1;
    let room = area.width.saturating_sub(width(APP_NAME) + pill);
    let (first, end, lead, tail) = visible_tabs(&widths, active, room);

    let mut x = area.x + width(APP_NAME);
    let mut hits = TabHits::new();
    let mut pieces = Vec::new();
    if lead {
        pieces.push((x, ELLIPSIS.to_string(), None));
        x += width(ELLIPSIS);
    }
    for i in first..end {
        let w = widths[i].min(area.right().saturating_sub(x));
        hits.push((i, Rect::new(x, area.y, w, 1)));
        pieces.push((x, labels[i].clone(), Some(i)));
        x += widths[i];
    }
    if tail {
        pieces.push((x, ELLIPSIS.to_string(), None));
    }
    (hits, pieces)
}

pub(crate) fn draw(state: &State, theme: &Theme, frame: &mut Frame, area: Rect) {
    if area.is_empty() {
        return;
    }
    frame.render_widget(Paragraph::new("").style(theme.surface()), area);
    let sessions = &state.sessions;
    let order: Vec<&SessionState> = sessions.iter().collect();

    let mut spans = vec![Span::styled(APP_NAME, theme.app_name())];
    let (_, pieces) = layout_tabs(state, area);
    for (_, text, tab) in &pieces {
        let style = match tab.map(|i| order[i]) {
            Some(session) if session.id == sessions.active => theme.tab_active(),
            Some(session) if session.busy() => theme.tab_running(),
            Some(_) => theme.tab(),
            None => theme.overflow_hint(),
        };
        // The label's tag takes its own colour over the tab's.
        let tag = tab.map(|i| tag(order[i])).unwrap_or_default();
        match tab.map(|i| order[i]).and_then(|s| s.label) {
            Some(label) if text.ends_with(&tag) => {
                let base = &text[..text.len() - tag.len()];
                spans.push(Span::styled(base.to_string(), style));
                spans.push(Span::styled(tag, style.patch(theme.conn_label(label))));
            }
            _ => spans.push(Span::styled(text.clone(), style)),
        }
    }
    let used: u16 = spans.iter().map(|s| width(&s.content)).sum();

    // The pill, and the connection summary before it if there is room.
    let pill = state.pill();
    let pill_text = format!(" {} ", pill.label());
    let summary = format!("{}  ", sessions.active().summary);
    let right = width(&pill_text) + 1;
    let mut tail = Vec::new();
    if used + width(&summary) + right <= area.width {
        tail.push(Span::styled(summary, theme.dim()));
    }
    tail.push(Span::styled(pill_text, theme.mode_pill(pill)));
    tail.push(Span::raw(" "));
    let tail_width: u16 = tail.iter().map(|s| width(&s.content)).sum();

    frame.render_widget(Line::from(spans), area);
    if tail_width <= area.width {
        let at = Rect::new(area.right() - tail_width, area.y, tail_width, 1);
        frame.render_widget(Line::from(tail), at);
    }
}

#[cfg(test)]
#[path = "top_bar_tests.rs"]
mod top_bar_tests;
