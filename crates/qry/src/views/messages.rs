//! The Messages dialog: every notice so far, newest first, in full.

use ratatui::{prelude::*, widgets::Paragraph};

use super::{confirm::wrap, layout::centered_rect, overlay::dialog_frame};
use crate::{
    app::state::{MessagesState, State},
    notice::Level,
    theme::{Accent, Theme, ToastLevel},
};

/// The time column: `12:04:31  `.
const TIME_WIDTH: u16 = 10;

fn glyph_style(level: Level, theme: &Theme) -> Style {
    match level {
        Level::Success => theme.value_ok(),
        Level::Info => theme.dialog(),
        Level::Warning => theme.toast(ToastLevel::Warning),
        Level::Error => theme.toast(ToastLevel::Error),
    }
}

/// The dialog's lines, and on which line each entry starts.
fn lines(state: &State, width: u16, theme: &Theme) -> (Vec<Line<'static>>, Vec<usize>) {
    let selected = match state.overlays.last() {
        Some(crate::app::state::Overlay::Messages(m)) => m.selected,
        _ => 0,
    };
    let text_width = width.saturating_sub(TIME_WIDTH + 2);
    let mut lines = Vec::new();
    let mut starts = Vec::new();
    for (i, (at, notice)) in state.messages.newest_first().enumerate() {
        starts.push(lines.len());
        let row_style = if i == selected {
            theme.row_selected()
        } else {
            theme.dialog()
        };
        let glyph = match notice.level.glyph() {
            "" => "  ".to_string(),
            g => format!("{g} "),
        };
        for (n, chunk) in wrap(&notice.text, text_width).into_iter().enumerate() {
            let head = if n == 0 {
                vec![
                    Span::styled(format!("{at}  "), theme.dim()),
                    Span::styled(glyph.clone(), glyph_style(notice.level, theme)),
                ]
            } else {
                vec![Span::raw(" ".repeat(usize::from(TIME_WIDTH) + 2))]
            };
            let mut spans = head;
            spans.push(Span::raw(chunk));
            lines.push(Line::from(spans).style(row_style));
        }
    }
    (lines, starts)
}

pub(crate) fn draw(
    messages: &MessagesState,
    state: &State,
    theme: &Theme,
    frame: &mut Frame,
    area: Rect,
) {
    let width = (u32::from(area.width) * 9 / 10) as u16;
    let height = (u32::from(area.height) * 7 / 10) as u16;
    let rect = centered_rect(
        area,
        width.max(area.width.min(20)),
        height.max(area.height.min(5)),
    );
    let inner = dialog_frame(frame, theme, rect, "Messages", Accent::Info);
    let footer = Line::from(vec![
        Span::styled("j/k", theme.hint_key()),
        Span::styled(" scroll  ", theme.hint()),
        Span::styled("y", theme.hint_key()),
        Span::styled(" copy  ", theme.hint()),
        Span::styled("c", theme.hint_key()),
        Span::styled(" clear  ", theme.hint()),
        Span::styled("Esc", theme.hint_key()),
        Span::styled(" close ", theme.hint()),
    ]);
    if rect.height > 1 {
        let at = Rect::new(
            rect.x + 2,
            rect.bottom() - 1,
            rect.width.saturating_sub(4),
            1,
        );
        frame.render_widget(footer, at);
    }
    let inner = inner.inner(Margin::new(1, 0));
    if inner.height == 0 {
        return;
    }
    if state.messages.is_empty() {
        frame.render_widget(Paragraph::new("No messages yet").style(theme.dim()), inner);
        return;
    }
    let (lines, starts) = lines(state, inner.width, theme);
    let page = usize::from(inner.height);
    messages.page.set(page);
    // Scroll as little as keeps the selected entry's first line in view.
    let at = starts.get(messages.selected).copied().unwrap_or(0);
    let mut scroll = messages.scroll.get();
    if at < scroll {
        scroll = at;
    } else if at >= scroll + page {
        scroll = at + 1 - page;
    }
    messages.scroll.set(scroll);
    let shown: Vec<Line> = lines.into_iter().skip(scroll).take(page).collect();
    frame.render_widget(Paragraph::new(shown), inner);
}

#[cfg(test)]
#[path = "messages_tests.rs"]
mod messages_tests;
