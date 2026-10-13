//! The confirm dialog: the message, word-wrapped, and two buttons.

use ratatui::{
    prelude::*,
    widgets::{Paragraph, Wrap},
};

use super::{layout::centered_rect, overlay::dialog_frame};
use crate::{
    app::dialogs::{confirm::Confirm, form::Form},
    theme::Theme,
};

/// The message wrapped to `width`, as the dialog shows it.
pub(crate) fn wrap(message: &str, width: u16) -> Vec<String> {
    let width = usize::from(width.max(1));
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in message.split_whitespace() {
        let needed = line.chars().count() + usize::from(!line.is_empty()) + word.chars().count();
        if !line.is_empty() && needed > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

pub(crate) fn draw(confirm: &Confirm, theme: &Theme, frame: &mut Frame, area: Rect) {
    let width = confirm
        .width()
        .min(area.width.saturating_sub(4))
        .max(area.width.min(10));
    let lines = wrap(confirm.message(), width.saturating_sub(4));
    let height = u16::try_from(lines.len()).unwrap_or(u16::MAX) + 4;
    let rect = centered_rect(area, width, height);
    let inner = dialog_frame(frame, theme, rect, confirm.title(), confirm.accent());
    let inner = inner.inner(Margin::new(1, 0));
    if inner.height == 0 {
        return;
    }
    let text: Vec<Line> = lines.into_iter().map(Line::raw).collect();
    frame.render_widget(
        Paragraph::new(text)
            .style(theme.dialog())
            .wrap(Wrap { trim: true }),
        Rect {
            height: inner.height.saturating_sub(2),
            ..inner
        },
    );

    let primary = if confirm.destructive() {
        theme.button_danger()
    } else {
        theme.button_primary()
    };
    let (cancel, act) = if confirm.confirm_selected() {
        (theme.button(), primary)
    } else {
        (primary, theme.button())
    };
    let label = |text: &str, selected: bool| {
        if selected {
            format!("[ {text} ]")
        } else {
            format!("  {text}  ")
        }
    };
    let buttons = Line::from(vec![
        Span::styled(label("Cancel", !confirm.confirm_selected()), cancel),
        Span::raw("  "),
        Span::styled(
            label(confirm.confirm_label(), confirm.confirm_selected()),
            act,
        ),
    ])
    .right_aligned();
    frame.render_widget(
        buttons,
        Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
    );
}
