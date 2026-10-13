//! The modal forms, drawn as a popup over the panes.

use ratatui::{
    layout::Flex,
    prelude::*,
    widgets::{Block, Clear, Paragraph},
};

use crate::{
    app::dialogs::{
        form::{Field, Form},
        textinput::{SecretInput, TextInput},
    },
    theme::Theme,
};

/// Each field is a bordered box, with a blank row between boxes.
const FIELD_HEIGHT: u16 = 3;
const FIELD_GAP: u16 = 1;
const LABEL_WIDTH: u16 = 14;
pub(crate) const MODAL_WIDTH: u16 = 60;

/// Height of the popup with every field shown.
pub(crate) fn height(form: &dyn Form) -> u16 {
    let fields = form.labels().len() as u16;
    fields * (FIELD_HEIGHT + FIELD_GAP) - FIELD_GAP + 4
}

/// Draws the popup centred on `area`, shrinking to fit a short terminal.
pub(crate) fn draw(form: &dyn Form, theme: &Theme, frame: &mut Frame, area: Rect) {
    let height = height(form).min(area.height);
    let [popup] = Layout::horizontal([Constraint::Length(MODAL_WIDTH)])
        .flex(Flex::Center)
        .areas(area);
    let [popup] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(popup);

    frame.render_widget(Clear, popup);
    let footer = match (form.pending(), form.error()) {
        (Some(pending), _) => Line::styled(format!(" {pending} "), theme.spinner()),
        (None, Some(error)) => Line::styled(format!(" {error} "), theme.value_error()),
        (None, None) => Line::styled(format!(" {} ", form.hint()), theme.hint()),
    };
    let mut block = Block::bordered()
        .style(theme.dialog())
        .border_style(theme.dialog_border(theme.teal))
        .title(Line::styled(form.title(), theme.dialog_title(theme.teal)))
        .title_bottom(footer);
    if let Some(subtitle) = form.subtitle() {
        block = block.title(Line::styled(format!(" {subtitle} "), theme.muted()).right_aligned());
    }
    let inner = block.inner(popup).inner(Margin::new(2, 1));
    frame.render_widget(block, popup);
    draw_fields(form, theme, frame, inner);
}

/// Draws as many fields as fit, scrolled so the focused one is visible.
fn draw_fields(form: &dyn Form, theme: &Theme, frame: &mut Frame, area: Rect) {
    let labels = form.labels();
    let fits =
        usize::from((area.height + FIELD_GAP) / (FIELD_HEIGHT + FIELD_GAP)).clamp(1, labels.len());
    let first = (form.focus() + 1).saturating_sub(fits);
    let rows = Layout::vertical(vec![Constraint::Length(FIELD_HEIGHT); fits])
        .spacing(FIELD_GAP)
        .split(area);

    for (row, i) in rows.iter().zip(first..) {
        if row.height < FIELD_HEIGHT {
            continue;
        }
        let focused = i == form.focus();
        let [label_area, box_area] =
            Layout::horizontal([Constraint::Length(LABEL_WIDTH), Constraint::Fill(1)]).areas(*row);

        // The label sits on the box's middle row, level with the text.
        let label_area = Rect {
            y: label_area.y + 1,
            height: 1,
            ..label_area
        };
        let label = if focused {
            Line::styled(format!("› {}", labels[i]), theme.field_focused())
        } else {
            Line::styled(format!("  {}", labels[i]), theme.field())
        };
        frame.render_widget(label, label_area);

        let block = Block::bordered().border_style(if focused {
            theme.field_border_focused()
        } else {
            theme.field_border()
        });
        let input_area = block.inner(box_area).inner(Margin::new(1, 0));
        frame.render_widget(block, box_area);

        draw_field(&form.fields()[i], focused, theme, frame, input_area);
    }
}

fn draw_field(field: &Field, focused: bool, theme: &Theme, frame: &mut Frame, area: Rect) {
    match field {
        Field::Text(input) => draw_text(input, focused, theme, frame, area),
        Field::Secret(input) => draw_secret(input, focused, theme, frame, area),
        Field::Choice { options, selected } => {
            let option = options[*selected];
            let line = if focused {
                Line::styled(format!("‹ {option} ›"), theme.field_focused())
            } else {
                Line::styled(format!("  {option}"), theme.field())
            };
            frame.render_widget(line, area);
        }
    }
}

/// Draws the input, scrolled so the cursor stays visible.
fn draw_text(input: &TextInput, focused: bool, theme: &Theme, frame: &mut Frame, area: Rect) {
    let width = usize::from(area.width).max(1);
    let offset = (input.cursor + 1).saturating_sub(width);
    let visible: String = input.value.chars().skip(offset).take(width).collect();
    let cursor = focused.then_some(input.cursor - offset);
    draw_line(frame, area, (visible, input.placeholder), cursor, theme);
}

/// Draws a dot per character, scrolled like a text input.
fn draw_secret(input: &SecretInput, focused: bool, theme: &Theme, frame: &mut Frame, area: Rect) {
    let width = usize::from(area.width).max(1);
    let offset = (input.cursor + 1).saturating_sub(width);
    let visible = "•".repeat(input.len().saturating_sub(offset).min(width));
    let cursor = focused.then_some(input.cursor - offset);
    draw_line(frame, area, (visible, input.placeholder), cursor, theme);
}

/// Draws the visible text of an input, or its placeholder when there is
/// none, and places the terminal cursor at `cursor` cells in, if given. The
/// terminal cursor is only placed on the focused input.
fn draw_line(
    frame: &mut Frame,
    area: Rect,
    (visible, placeholder): (String, Option<&'static str>),
    cursor: Option<usize>,
    theme: &Theme,
) {
    match (visible.is_empty(), placeholder) {
        (true, Some(placeholder)) => {
            frame.render_widget(
                Paragraph::new(placeholder).style(theme.field_placeholder()),
                area,
            );
        }
        _ => frame.render_widget(Paragraph::new(visible).style(theme.field()), area),
    }
    if let Some(cursor) = cursor {
        let x = area.x + u16::try_from(cursor).unwrap_or(0);
        frame.set_cursor_position((x, area.y));
    }
}
