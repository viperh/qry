//! Modal forms: a column of labelled fields drawn over the panes.
//!
//! [`Form`] holds everything the New Connection and Export modals share —
//! key handling, layout, scrolling and the popup frame. An implementor only
//! supplies its fields and what submitting them means.

use crossterm::event::KeyEvent;
use ratatui::{
    layout::Flex,
    prelude::*,
    widgets::{Block, Clear},
};

use crate::{
    action::Action,
    components::textinput::{TextInput, typed_char},
    keymap::FormCommand,
};

/// Each field is a bordered box, with a blank row between boxes.
const FIELD_HEIGHT: u16 = 3;
const FIELD_GAP: u16 = 1;
const LABEL_WIDTH: u16 = 14;
pub const MODAL_WIDTH: u16 = 60;

pub enum Field {
    Text(TextInput),
    /// Cycled with ← / → (or Space) instead of typed.
    Choice {
        options: &'static [&'static str],
        selected: usize,
    },
}

impl Field {
    pub fn text() -> Self {
        Field::Text(TextInput::default())
    }

    /// A text field that shows `hint`, greyed, while it is empty.
    pub fn text_with_placeholder(hint: &'static str) -> Self {
        Field::Text(TextInput {
            placeholder: Some(hint),
            ..TextInput::default()
        })
    }

    pub fn masked() -> Self {
        Field::Text(TextInput {
            masked: true,
            ..TextInput::default()
        })
    }

    pub fn choice(options: &'static [&'static str], selected: usize) -> Self {
        Field::Choice { options, selected }
    }

    fn apply(&mut self, command: FormCommand) {
        match self {
            Field::Text(input) => match command {
                FormCommand::Left => input.move_left(),
                FormCommand::Right => input.move_right(),
                FormCommand::LineStart => input.move_home(),
                FormCommand::LineEnd => input.move_end(),
                FormCommand::DeleteBack => input.delete_back(),
                FormCommand::DeleteForward => input.delete_forward(),
                _ => {}
            },
            Field::Choice { options, selected } => match command {
                FormCommand::Right | FormCommand::NextChoice => {
                    *selected = (*selected + 1) % options.len();
                }
                FormCommand::Left => *selected = (*selected + options.len() - 1) % options.len(),
                _ => {}
            },
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect, focused: bool) {
        match self {
            Field::Text(input) => input.render(frame, area, focused),
            Field::Choice { options, selected } => {
                let option = options[*selected];
                let line = if focused {
                    Line::from(format!("‹ {option} ›")).cyan()
                } else {
                    Line::from(format!("  {option}"))
                };
                frame.render_widget(line, area);
            }
        }
    }
}

pub trait Form {
    fn title(&self) -> &'static str;
    /// Shown in the popup's bottom border while there is no error.
    fn hint(&self) -> &'static str;
    /// One label per field, in the same order as [`Form::fields`].
    fn labels(&self) -> &'static [&'static str];
    fn fields(&self) -> &[Field];
    fn fields_mut(&mut self) -> &mut [Field];
    fn focus(&self) -> usize;
    fn set_focus(&mut self, focus: usize);
    fn error(&self) -> Option<&str>;
    fn set_error(&mut self, error: Option<String>);
    /// What to send when the form is accepted, or why it was refused.
    fn submit(&self) -> Result<Action, String>;

    /// Shown next to the title, for what the title cannot carry.
    fn subtitle(&self) -> Option<String> {
        None
    }

    /// Called after a key changed the form, so a form whose fields depend on
    /// each other can add or remove one.
    fn after_change(&mut self) {}

    /// What the form is waiting for, shown in place of the hint. While this
    /// is set its owner ignores Submit, so one Enter is one attempt.
    fn pending(&self) -> Option<&'static str> {
        None
    }

    fn set_pending(&mut self, pending: Option<&'static str>) {
        let _ = pending;
    }

    fn text(&self, field: usize) -> &str {
        match &self.fields()[field] {
            Field::Text(input) => &input.value,
            Field::Choice { .. } => "",
        }
    }

    fn selected(&self, field: usize) -> usize {
        match &self.fields()[field] {
            Field::Choice { selected, .. } => *selected,
            Field::Text(_) => 0,
        }
    }

    /// `command` is what the config binds `key` to. In a text field, plain
    /// characters are typed even if bound, so Space types a space there.
    fn handle_key(&mut self, key: KeyEvent, command: Option<FormCommand>) {
        let focus = self.focus();
        let count = self.labels().len();

        if let Some(c) = typed_char(key)
            && matches!(self.fields()[focus], Field::Text(_))
        {
            self.set_error(None);
            if let Field::Text(input) = &mut self.fields_mut()[focus] {
                input.insert(c);
            }
            self.after_change();
            return;
        }

        match command {
            Some(FormCommand::NextField) => self.set_focus((focus + 1) % count),
            Some(FormCommand::PrevField) => self.set_focus((focus + count - 1) % count),
            // Submitting is the owner's job: it has the channel to send on.
            Some(FormCommand::Submit) => {}
            Some(command) => {
                self.set_error(None);
                self.fields_mut()[focus].apply(command);
                self.after_change();
            }
            None => {}
        }
    }

    /// Height of the popup with every field shown.
    fn height(&self) -> u16 {
        let fields = self.labels().len() as u16;
        fields * (FIELD_HEIGHT + FIELD_GAP) - FIELD_GAP + 4
    }

    /// Draws the popup centred on `area`, shrinking to fit a short terminal.
    fn render(&self, frame: &mut Frame, area: Rect) {
        let height = self.height().min(area.height);
        let [popup] = Layout::horizontal([Constraint::Length(MODAL_WIDTH)])
            .flex(Flex::Center)
            .areas(area);
        let [popup] = Layout::vertical([Constraint::Length(height)])
            .flex(Flex::Center)
            .areas(popup);

        frame.render_widget(Clear, popup);
        let footer = match (self.pending(), self.error()) {
            (Some(pending), _) => Line::from(format!(" {pending} ")).cyan(),
            (None, Some(error)) => Line::from(format!(" {error} ")).red(),
            (None, None) => Line::from(format!(" {} ", self.hint())).dark_gray(),
        };
        let mut block = Block::bordered().title(self.title()).title_bottom(footer);
        if let Some(subtitle) = self.subtitle() {
            block = block.title(Line::from(format!(" {subtitle} ")).right_aligned());
        }
        let inner = block.inner(popup).inner(Margin::new(2, 1));
        frame.render_widget(block, popup);
        self.render_fields(frame, inner);
    }

    /// Draws as many fields as fit, scrolled so the focused one is visible.
    fn render_fields(&self, frame: &mut Frame, area: Rect) {
        let labels = self.labels();
        let fits = usize::from((area.height + FIELD_GAP) / (FIELD_HEIGHT + FIELD_GAP))
            .clamp(1, labels.len());
        let first = (self.focus() + 1).saturating_sub(fits);
        let rows = Layout::vertical(vec![Constraint::Length(FIELD_HEIGHT); fits])
            .spacing(FIELD_GAP)
            .split(area);

        for (row, i) in rows.iter().zip(first..) {
            if row.height < FIELD_HEIGHT {
                continue;
            }
            let focused = i == self.focus();
            let [label_area, box_area] =
                Layout::horizontal([Constraint::Length(LABEL_WIDTH), Constraint::Fill(1)])
                    .areas(*row);

            // The label sits on the box's middle row, level with the text.
            let label_area = Rect {
                y: label_area.y + 1,
                height: 1,
                ..label_area
            };
            let label = if focused {
                Line::from(format!("› {}", labels[i])).cyan().bold()
            } else {
                Line::from(format!("  {}", labels[i]))
            };
            frame.render_widget(label, label_area);

            let block = Block::bordered().border_style(if focused {
                Style::new().cyan()
            } else {
                Style::new().dark_gray()
            });
            let input_area = block.inner(box_area).inner(Margin::new(1, 0));
            frame.render_widget(block, box_area);

            self.fields()[i].render(frame, input_area, focused);
        }
    }
}

#[cfg(test)]
#[path = "form_tests.rs"]
pub(crate) mod form_tests;
