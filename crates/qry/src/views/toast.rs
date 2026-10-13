//! The toast: one line over the bottom of the right-hand side, drawn after
//! everything else, after tachy's `toast.rs`.
//!
//! ` ✗ text… ×2  ctrl-l  (+3) `: the level's mark, the text cut to fit,
//! how many identical notices it stands for, where to read a cut error in
//! full, and how many toasts wait.

use ratatui::prelude::*;

use crate::{
    action::Action,
    app::state::State,
    keymap::{KeyContext, display_chord},
    notice::{Level, Toast},
    theme::{Theme, ToastLevel},
};

fn level(level: Level) -> ToastLevel {
    match level {
        Level::Info => ToastLevel::Info,
        Level::Success => ToastLevel::Success,
        Level::Warning => ToastLevel::Warning,
        Level::Error => ToastLevel::Error,
    }
}

/// The toast's spans for a line `width` wide. `messages_key` is the chord
/// that opens the Messages dialog, if one is bound.
pub(crate) fn line(
    toast: &Toast,
    queued: usize,
    messages_key: Option<&str>,
    theme: &Theme,
    width: u16,
) -> Line<'static> {
    let notice = &toast.notice;
    let style = theme.toast(level(notice.level));
    let glyph = notice.level.glyph();
    let mark = match notice.level {
        Level::Success => Span::styled(format!(" {glyph} "), theme.value_ok().bg(theme.bg)),
        Level::Info => Span::styled(" ", style),
        Level::Warning | Level::Error => Span::styled(format!(" {glyph} "), style),
    };
    let count = (toast.count > 1).then(|| Span::styled(format!(" ×{}", toast.count), style));
    let queue =
        (queued > 0).then(|| Span::styled(format!("  (+{queued})"), theme.dim().bg(theme.bg)));

    let width_of = |span: &Option<Span>| span.as_ref().map_or(0, Span::width);
    let room =
        usize::from(width).saturating_sub(mark.width() + width_of(&count) + width_of(&queue) + 1);
    let text: String = notice.text.lines().collect::<Vec<_>>().join(" ");
    let cut = text.chars().count() > room;
    // A cut error says where to read it in full.
    let hint = (cut && notice.level == Level::Error)
        .then_some(messages_key)
        .flatten()
        .map(|key| Span::styled(format!("  {key}"), theme.hint_key().bg(theme.bg)));
    let room = room.saturating_sub(width_of(&hint));
    let shown = if text.chars().count() > room {
        let mut cut: String = text.chars().take(room.saturating_sub(1)).collect();
        cut.push('…');
        cut
    } else {
        text
    };

    let mut spans = vec![mark, Span::styled(shown, style)];
    spans.extend(count);
    let used: usize =
        spans.iter().map(Span::width).sum::<usize>() + width_of(&hint) + width_of(&queue);
    // The hint and the queue count sit at the right.
    let gap = usize::from(width).saturating_sub(used + 1);
    spans.push(Span::styled(" ".repeat(gap), style));
    spans.extend(hint);
    spans.extend(queue);
    spans.push(Span::styled(" ", style));
    Line::from(spans)
}

pub(crate) fn draw(state: &State, theme: &Theme, frame: &mut Frame, area: Rect) {
    let Some((toast, queued)) = state.toasts.visible() else {
        return;
    };
    let chords = state
        .config
        .keymap
        .chords_for(KeyContext::Global, &Action::ShowMessages);
    let key = (!chords.is_empty()).then(|| display_chord(&chords));
    frame.render_widget(line(toast, queued, key.as_deref(), theme, area.width), area);
}

#[cfg(test)]
#[path = "toast_tests.rs"]
mod toast_tests;
