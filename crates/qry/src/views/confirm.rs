//! The confirm dialog: the message, word-wrapped, and two buttons.

use ratatui::{
    prelude::*,
    widgets::{Paragraph, Wrap},
};

use super::{layout::centered_rect, overlay::dialog_frame};
use crate::{
    action::Action,
    app::dialogs::{
        confirm::Confirm,
        form::{Field, Form},
    },
    keymap::{KeyContext, Keymap, display_chord},
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

/// The keys a confirm with checkboxes shows in its bottom border.
const KEYS: [(Action, &str); 3] = [
    (Action::Submit, "confirm"),
    (Action::NextChoice, "toggle"),
    (Action::Cancel, "cancel"),
];

pub(crate) fn draw(
    confirm: &Confirm,
    keymap: &Keymap,
    theme: &Theme,
    frame: &mut Frame,
    area: Rect,
) {
    let width = confirm
        .width()
        .min(area.width.saturating_sub(4))
        .max(area.width.min(10));
    let lines = wrap(confirm.message(), width.saturating_sub(4));
    let checks = u16::try_from(confirm.fields().len()).unwrap_or(u16::MAX);
    let message_rows = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    let height = message_rows + checks + 4;
    let rect = centered_rect(area, width, height);
    let inner = dialog_frame(frame, theme, rect, confirm.title(), confirm.accent());
    if checks > 0 {
        draw_keys(keymap, theme, frame, rect);
    }
    let inner = inner.inner(Margin::new(1, 0));
    if inner.height == 0 {
        return;
    }
    let text: Vec<Line> = lines.into_iter().map(Line::raw).collect();
    let body = inner.height.saturating_sub(2);
    frame.render_widget(
        Paragraph::new(text)
            .style(theme.dialog())
            .wrap(Wrap { trim: true }),
        Rect {
            height: body.saturating_sub(checks),
            ..inner
        },
    );
    for (i, field) in confirm.fields().iter().enumerate() {
        let Field::Checkbox { checked, caption } = field else {
            continue;
        };
        let row = message_rows + u16::try_from(i).unwrap_or(u16::MAX);
        if row >= body {
            break;
        }
        let focused = i == confirm.focus();
        let mark = Span::styled(
            if *checked { "[x]" } else { "[ ]" },
            if focused {
                theme.field_focused()
            } else {
                theme.field()
            },
        );
        let line = Line::from(vec![
            mark,
            Span::styled(format!(" {caption}"), theme.field()),
        ]);
        frame.render_widget(line, Rect::new(inner.x, inner.y + row, inner.width, 1));
    }

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

/// `Enter confirm · Space toggle · Esc cancel` in the bottom border, with
/// the keys the `Dialog` keymap has for them.
fn draw_keys(keymap: &Keymap, theme: &Theme, frame: &mut Frame, rect: Rect) {
    let mut spans = vec![Span::raw(" ")];
    for (n, (action, label)) in KEYS.iter().enumerate() {
        let chords = keymap.chords_for(KeyContext::Dialog, action);
        if chords.is_empty() {
            continue;
        }
        if n > 0 {
            spans.push(Span::styled(" · ", theme.hint()));
        }
        spans.push(Span::styled(display_chord(&chords), theme.hint_key()));
        spans.push(Span::styled(format!(" {label}"), theme.hint()));
    }
    spans.push(Span::raw(" "));
    let line = Line::from(spans);
    let w = u16::try_from(line.width()).unwrap_or(u16::MAX);
    if w + 2 <= rect.width && rect.height > 1 {
        frame.render_widget(line, Rect::new(rect.x + 1, rect.bottom() - 1, w, 1));
    }
}
