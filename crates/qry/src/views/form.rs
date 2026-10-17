//! The modal forms, drawn in a dialog frame over the dimmed screen.
//!
//! One row per field: a `› ` marker on the focused one, a label column, and
//! the value. Below them a footer: the keys, the spinner while a submit
//! waits, or the error.

use ratatui::prelude::*;

use super::{
    layout::centered_rect,
    overlay::{SPINNER, dialog_frame},
};
use crate::{
    app::dialogs::{
        form::{Field, Form, Note, Tone},
        textinput::{SecretInput, TextInput},
    },
    keymap::{KeyContext, Keymap, display_chord},
    theme::Theme,
};

/// The widest a label column gets.
const MAX_LABEL: u16 = 18;

/// One row of a form's body: a field, or the error under one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Field(usize),
    Error(usize),
}

fn rows(form: &dyn Form) -> Vec<Row> {
    let mut rows = Vec::new();
    for i in 0..form.labels().len() {
        rows.push(Row::Field(i));
        if form.field_error(i).is_some() {
            rows.push(Row::Error(i));
        }
    }
    rows
}

/// Rows the note takes: a blank one and its own.
fn note_rows(form: &dyn Form) -> u16 {
    if form.note().is_some() { 2 } else { 0 }
}

/// The popup's size with every field shown: the fields and their errors,
/// the note, a blank row, the footer and the border.
pub(crate) fn height(form: &dyn Form) -> u16 {
    u16::try_from(rows(form).len()).unwrap_or(u16::MAX) + note_rows(form) + 4
}

/// The popup's width in an `area` this wide: what the form asks for, with
/// two columns to spare on each side.
pub(crate) fn width(form: &dyn Form, area: u16) -> u16 {
    form.width().min(area.saturating_sub(4)).max(area.min(10))
}

/// Draws the popup centred on `area`, shrinking to fit a small terminal.
/// `spinner` is the pending footer's frame.
pub(crate) fn draw(
    form: &dyn Form,
    keymap: &Keymap,
    spinner: usize,
    theme: &Theme,
    frame: &mut Frame,
    area: Rect,
) {
    let rect = centered_rect(area, width(form, area.width), height(form));
    let inner = dialog_frame(frame, theme, rect, form.title(), form.accent());
    if let Some(subtitle) = form.subtitle() {
        let text = format!(" {subtitle} ");
        let w = u16::try_from(Span::raw(&text).width()).unwrap_or(u16::MAX);
        if w + 4 < rect.width {
            let at = Rect::new(rect.right() - w - 2, rect.y, w, 1);
            let style = if form.subtitle_warns() {
                theme.subtitle_warning()
            } else {
                theme.muted()
            };
            frame.render_widget(Span::styled(text, style), at);
        }
    }
    let inner = inner.inner(Margin::new(1, 0));
    if inner.height == 0 {
        return;
    }
    let footer_row = Rect::new(inner.x, inner.bottom() - 1, inner.width, 1);
    let notes = note_rows(form);
    let fields = Rect {
        height: inner.height.saturating_sub(2 + notes),
        ..inner
    };
    draw_fields(form, theme, frame, fields);
    if let Some(note) = form.note()
        && inner.height >= 2 + notes
    {
        let at = Rect::new(inner.x, inner.bottom() - 3, inner.width, 1);
        frame.render_widget(note_line(&note, theme), at);
    }
    frame.render_widget(
        footer(form, keymap, spinner, theme, inner.width),
        footer_row,
    );
}

/// The footer: the spinner and what it waits for, the error, or the keys.
fn footer(
    form: &dyn Form,
    keymap: &Keymap,
    spinner: usize,
    theme: &Theme,
    width: u16,
) -> Line<'static> {
    if let Some(pending) = form.pending() {
        return Line::from(vec![
            Span::styled(SPINNER[spinner % SPINNER.len()], theme.spinner()),
            Span::styled(format!(" {pending}"), theme.spinner()),
        ]);
    }
    if let Some(error) = form.error() {
        let room = usize::from(width);
        let text = if error.chars().count() > room {
            let mut cut: String = error.chars().take(room.saturating_sub(1)).collect();
            cut.push('…');
            cut
        } else {
            error.to_string()
        };
        return Line::styled(text, theme.value_error());
    }
    let mut spans = Vec::new();
    for (action, label) in form.footer() {
        let chords = keymap.chords_for(KeyContext::Dialog, action);
        if chords.is_empty() {
            continue;
        }
        if !spans.is_empty() {
            spans.push(Span::styled(" · ", theme.hint()));
        }
        spans.push(Span::styled(display_chord(&chords), theme.hint_key()));
        spans.push(Span::styled(format!(" {label}"), theme.hint()));
    }
    Line::from(spans)
}

/// The note, each part in its tone's colour.
fn note_line(note: &Note, theme: &Theme) -> Line<'static> {
    let style = |tone: Tone| match tone {
        Tone::Ok => theme.value_ok(),
        Tone::Error => theme.value_error(),
        Tone::Busy => theme.spinner(),
        Tone::Plain => theme.field(),
        Tone::Dim => theme.dim(),
    };
    let mut spans = vec![Span::styled(format!("  {}", note.text), style(note.tone))];
    spans.extend(
        note.more
            .iter()
            .map(|(tone, text)| Span::styled(text.clone(), style(*tone))),
    );
    // The first tone runs on to the end of the line, as one part did.
    Line::from(spans).style(style(note.tone))
}

/// Draws as many rows as fit, scrolled so the focused field (and the error
/// under it) is visible.
fn draw_fields(form: &dyn Form, theme: &Theme, frame: &mut Frame, area: Rect) {
    let labels = form.labels();
    if labels.is_empty() || area.height == 0 {
        return;
    }
    let rows = rows(form);
    let label_width = labels
        .iter()
        .map(|l| u16::try_from(l.chars().count()).unwrap_or(u16::MAX))
        .max()
        .unwrap_or(0)
        .saturating_add(2)
        .min(MAX_LABEL);
    let fits = usize::from(area.height).min(rows.len());
    // The last row that belongs to the focused field: its error, if any.
    let focus_end = rows
        .iter()
        .rposition(|row| matches!(row, Row::Field(i) | Row::Error(i) if *i == form.focus()))
        .unwrap_or(0);
    let first = (focus_end + 1).saturating_sub(fits);
    for (line, row) in (0..fits).zip(rows.iter().skip(first)) {
        let y = area.y + u16::try_from(line).unwrap_or(0);
        let i = match *row {
            Row::Field(i) => i,
            Row::Error(i) => {
                let indent = label_width + 2;
                let message = form.field_error(i).unwrap_or_default();
                let at = Rect::new(
                    area.x + indent.min(area.width),
                    y,
                    area.width.saturating_sub(indent),
                    1,
                );
                frame.render_widget(Span::styled(message.to_string(), theme.value_error()), at);
                continue;
            }
        };
        let focused = i == form.focus();
        let (marker, label_style) = if focused {
            (
                Span::styled("› ", theme.field_focused()),
                theme.inspector_label_focused(),
            )
        } else {
            (Span::raw("  "), theme.dim())
        };
        let label = Span::styled(
            format!("{:<width$}", labels[i], width = usize::from(label_width)),
            label_style,
        );
        let head = Rect::new(area.x, y, (label_width + 2).min(area.width), 1);
        frame.render_widget(Line::from(vec![marker, label]), head);
        let value = Rect::new(
            head.right(),
            y,
            area.right().saturating_sub(head.right()),
            1,
        );
        draw_field(&form.fields()[i], focused, theme, frame, value);
    }
}

fn draw_field(field: &Field, focused: bool, theme: &Theme, frame: &mut Frame, area: Rect) {
    match field {
        Field::Text(input) => draw_text(input, focused, theme, frame, area),
        Field::Secret(input) => draw_secret(input, focused, theme, frame, area),
        Field::Choice { options, selected } => {
            let arrows = if focused {
                theme.hint_key()
            } else {
                theme.dim()
            };
            let line = Line::from(vec![
                Span::styled("‹ ", arrows),
                Span::styled(options[*selected], theme.field()),
                Span::styled(" ›", arrows),
            ]);
            frame.render_widget(line, area);
        }
        Field::Checkbox { checked, caption } => {
            let mark = if *checked { "[x]" } else { "[ ]" };
            let style = if focused {
                theme.field_focused()
            } else {
                theme.field()
            };
            let line = Line::from(vec![
                Span::styled(mark, style),
                Span::styled(format!(" {caption}"), theme.field()),
            ]);
            frame.render_widget(line, area);
        }
        Field::Button { caption } => {
            let style = if focused {
                theme.button_primary()
            } else {
                theme.button()
            };
            frame.render_widget(Span::styled(format!("[ {caption} ]"), style), area);
        }
    }
}

/// Draws a text input, scrolled so the cursor stays visible.
fn draw_text(input: &TextInput, focused: bool, theme: &Theme, frame: &mut Frame, area: Rect) {
    let chars: Vec<char> = input.value.chars().collect();
    draw_line(
        &chars,
        input.cursor,
        input.placeholder,
        focused,
        theme,
        frame,
        area,
    );
}

/// Draws a dot per character, scrolled like a text input.
fn draw_secret(input: &SecretInput, focused: bool, theme: &Theme, frame: &mut Frame, area: Rect) {
    let dots = vec!['•'; input.len()];
    draw_line(
        &dots,
        input.cursor,
        input.placeholder,
        focused,
        theme,
        frame,
        area,
    );
}

/// Draws `chars` with a block cursor at `cursor` in the focused input. A
/// value too long for `area` scrolls to keep the cursor in view, with `…`
/// in front once its start is cut off. An empty one shows its placeholder.
fn draw_line(
    chars: &[char],
    cursor: usize,
    placeholder: Option<&'static str>,
    focused: bool,
    theme: &Theme,
    frame: &mut Frame,
    area: Rect,
) {
    let width = usize::from(area.width);
    if width == 0 {
        return;
    }
    let cursor_cell = |c: char| Span::styled(c.to_string(), theme.cursor_cell());
    if chars.is_empty() {
        let mut spans = Vec::new();
        if focused {
            spans.push(cursor_cell(' '));
        }
        if let Some(placeholder) = placeholder {
            spans.push(Span::styled(placeholder, theme.field_placeholder()));
        }
        frame.render_widget(Line::from(spans), area);
        return;
    }
    // The first character shown: 0 while the cursor fits, else far enough
    // that it does, with one cell for the `…`.
    let offset = if cursor < width {
        0
    } else {
        cursor + 2 - width
    };
    let lead = offset > 0;
    let room = width - usize::from(lead);
    let mut spans = Vec::new();
    if lead {
        spans.push(Span::styled("…", theme.dim()));
    }
    let visible: String = chars.iter().skip(offset).take(room).collect();
    if focused {
        let at = cursor - offset;
        let before: String = visible.chars().take(at).collect();
        let under = visible.chars().nth(at).unwrap_or(' ');
        let after: String = visible.chars().skip(at + 1).collect();
        spans.push(Span::styled(before, theme.field()));
        spans.push(cursor_cell(under));
        spans.push(Span::styled(after, theme.field()));
    } else {
        spans.push(Span::styled(visible, theme.field()));
    }
    frame.render_widget(Line::from(spans), area);
}
