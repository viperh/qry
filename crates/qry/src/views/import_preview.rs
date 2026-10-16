//! The import preview: a teal dialog listing what the file held, one row
//! per connection, then the file's warnings and a note about passwords.
//!
//! ```text
//! ┌ Import Preview ─────────────────────────── DBeaver · 8 found ┐
//! │ › [x] + local-app        postgres · app@localhost             │
//! │   [ ] = prod             postgres · alice@db.prod             │
//! │       ! legacy           unsupported driver: mssql            │
//! │                                                               │
//! │ ! oracle-erp has no user name; edit it before connecting      │
//! │ passwords are never imported; each connection will ask …      │
//! │                                                               │
//! │ Enter import selected · Space toggle · a all · Esc cancel     │
//! └───────────────────────────────────────────────────────────────┘
//! ```

use ratatui::prelude::*;

use super::{layout::centered_rect, overlay::dialog_frame};
use crate::{
    action::Action,
    app::dialogs::{
        form::Form,
        importdialog::{EntryKind, ImportPreview},
    },
    keymap::{KeyContext, Keymap, display_chord},
    theme::Theme,
};

const NOTE: &str = "passwords are never imported; each connection will ask on first connect";

/// Rows besides the entries: warnings' blank, the note's blank and itself,
/// the footer's blank and itself.
fn extra_rows(preview: &ImportPreview) -> u16 {
    let warnings = u16::try_from(preview.warnings.len()).unwrap_or(u16::MAX);
    warnings + 4
}

pub(crate) fn draw(
    preview: &ImportPreview,
    keymap: &Keymap,
    theme: &Theme,
    frame: &mut Frame,
    area: Rect,
) {
    let width = preview
        .width()
        .min(area.width.saturating_sub(4))
        .max(area.width.min(10));
    let entries = u16::try_from(preview.entries.len().max(1)).unwrap_or(u16::MAX);
    let height = entries + extra_rows(preview) + 2;
    let rect = centered_rect(area, width, height.min(area.height.saturating_sub(2)));
    let inner = dialog_frame(frame, theme, rect, preview.title(), preview.accent());
    if let Some(subtitle) = preview.subtitle() {
        let text = format!(" {subtitle} · {} selected ", preview.selected());
        let w = u16::try_from(Span::raw(&text).width()).unwrap_or(u16::MAX);
        if w + 4 < rect.width {
            let at = Rect::new(rect.right() - w - 2, rect.y, w, 1);
            frame.render_widget(Span::styled(text, theme.muted()), at);
        }
    }
    let inner = inner.inner(Margin::new(1, 0));
    if inner.height == 0 {
        return;
    }

    // The entries take what the warnings, the note and the footer leave.
    let page = usize::from(inner.height.saturating_sub(extra_rows(preview)).max(1));
    preview.page.set(page);
    let cursor = preview.cursor.min(preview.entries.len().saturating_sub(1));
    let mut first = preview.scroll.get();
    if cursor < first {
        first = cursor;
    } else if cursor >= first + page {
        first = cursor + 1 - page;
    }
    first = first.min(preview.entries.len().saturating_sub(page));
    preview.scroll.set(first);

    let name_width = preview
        .entries
        .iter()
        .map(|e| e.name.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(8, 28);
    let mut y = inner.y;
    for (i, entry) in preview.entries.iter().enumerate().skip(first).take(page) {
        let focused = i == cursor;
        let marker = if focused {
            Span::styled("› ", theme.field_focused())
        } else {
            Span::raw("  ")
        };
        let (glyph, style, detail) = match &entry.kind {
            EntryKind::New => ("+", theme.value_ok(), entry.detail.clone()),
            EntryKind::Existing => ("=", theme.dim(), format!("{} · already here", entry.detail)),
            EntryKind::Skipped(reason) => ("!", theme.value_error(), reason.clone()),
        };
        let check = match (&entry.kind, entry.selected) {
            (EntryKind::New, true) => "[x] ",
            (EntryKind::New, false) => "[ ] ",
            _ => "    ",
        };
        let name: String = entry.name.chars().take(name_width).collect();
        let line = Line::from(vec![
            marker,
            Span::styled(
                check,
                if focused {
                    theme.field_focused()
                } else {
                    theme.field()
                },
            ),
            Span::styled(format!("{glyph} "), style),
            Span::styled(format!("{name:<name_width$}  "), style),
            Span::styled(detail, theme.dim()),
        ]);
        frame.render_widget(line, Rect::new(inner.x, y, inner.width, 1));
        y += 1;
    }
    if preview.entries.is_empty() {
        frame.render_widget(
            Span::styled("  the file held no connections", theme.dim()),
            Rect::new(inner.x, y, inner.width, 1),
        );
    }

    // Warnings, the note and the footer from the bottom up.
    let bottom = inner.bottom();
    let footer_y = bottom - 1;
    frame.render_widget(
        footer(keymap, theme),
        Rect::new(inner.x, footer_y, inner.width, 1),
    );
    let note_y = footer_y.saturating_sub(2);
    if note_y >= inner.y {
        frame.render_widget(
            Span::styled(NOTE, theme.dim()),
            Rect::new(inner.x, note_y, inner.width, 1),
        );
    }
    let count = u16::try_from(preview.warnings.len()).unwrap_or(u16::MAX);
    for (n, warning) in preview.warnings.iter().enumerate() {
        let wy = note_y
            .saturating_sub(count)
            .saturating_add(u16::try_from(n).unwrap_or(0));
        if wy < inner.y + 1 || wy >= note_y {
            continue;
        }
        frame.render_widget(
            Line::from(vec![
                Span::styled("! ", theme.value_error()),
                Span::styled(warning.clone(), theme.muted()),
            ]),
            Rect::new(inner.x, wy, inner.width, 1),
        );
    }
}

fn footer(keymap: &Keymap, theme: &Theme) -> Line<'static> {
    let mut spans = Vec::new();
    let mut push = |key: String, label: &str| {
        if key.is_empty() {
            return;
        }
        if !spans.is_empty() {
            spans.push(Span::styled(" · ", theme.hint()));
        }
        spans.push(Span::styled(key, theme.hint_key()));
        spans.push(Span::styled(format!(" {label}"), theme.hint()));
    };
    let chord = |action: Action| display_chord(&keymap.chords_for(KeyContext::Dialog, &action));
    push(chord(Action::Submit), "import selected");
    push(chord(Action::NextChoice), "toggle");
    push("a".into(), "all");
    push("A".into(), "none");
    push(chord(Action::Cancel), "cancel");
    Line::from(spans)
}
