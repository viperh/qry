//! The top bar: one tab per session, numbered as `ctrl-1`…`ctrl-9` reach
//! them. T202 gives it the rest of its chrome.

use ratatui::{prelude::*, widgets::Paragraph};

use crate::{
    app::state::{SessionState, State},
    theme::Theme,
};

/// What a tab says: its number and title, and a mark while a query runs
/// there.
fn label(number: usize, session: &SessionState) -> String {
    let running = if session.running > 0 { " ◐" } else { "" };
    format!(" {number} {}{running} ", session.title)
}

pub(crate) fn draw(state: &State, theme: &Theme, frame: &mut Frame, area: Rect) {
    if area.is_empty() {
        return;
    }
    frame.render_widget(Paragraph::new("").style(theme.tab()), area);

    let sessions = &state.sessions;
    let tabs: Vec<(String, Style)> = sessions
        .iter()
        .enumerate()
        .map(|(i, session)| {
            let style = if session.id == sessions.active {
                theme.tab_active()
            } else if session.running > 0 {
                theme.tab_running()
            } else {
                theme.tab()
            };
            (label(i + 1, session), style)
        })
        .collect();
    let widths: Vec<usize> = tabs
        .iter()
        .map(|(text, _)| Span::raw(text).width())
        .collect();
    let active = sessions
        .order
        .iter()
        .position(|id| *id == sessions.active)
        .unwrap_or(0);

    let room = usize::from(area.width);
    let ellipsis = Span::raw("…").width();
    // Drop tabs from the left until the active one fits.
    let mut first = 0;
    while first < active {
        let lead = if first > 0 { ellipsis } else { 0 };
        if lead + widths[first..=active].iter().sum::<usize>() <= room {
            break;
        }
        first += 1;
    }

    let mut spans = Vec::new();
    let mut used = 0;
    if first > 0 {
        spans.push(Span::styled("…", theme.overflow_hint()));
        used += ellipsis;
    }
    for (i, (text, style)) in tabs.iter().enumerate().skip(first) {
        // The tabs up to the active one fit; after it, room is kept for a
        // trailing `…` while more follow.
        let after = if i + 1 < tabs.len() { ellipsis } else { 0 };
        if i > active && used + widths[i] + after > room {
            spans.push(Span::styled("…", theme.overflow_hint()));
            break;
        }
        spans.push(Span::styled(text.clone(), *style));
        used += widths[i];
    }
    frame.render_widget(Line::from(spans), area);
}
