//! Modal forms: a column of labelled fields drawn over the panes.
//!
//! [`Form`] holds everything the modals share — fields, focus and key
//! handling. An implementor only supplies its fields and what submitting
//! them means. Drawing them is [`crate::views::form`]'s job.

use crossterm::event::KeyEvent;

use super::{
    Submission,
    textinput::{SecretInput, TextInput, typed_char},
};
use crate::{action::Action, secrets::Password};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Field {
    Text(TextInput),
    /// A password: typed like text, shown as dots, kept in a [`Password`].
    Secret(SecretInput),
    /// Cycled with ← / → (or Space) instead of typed.
    Choice {
        options: &'static [&'static str],
        selected: usize,
    },
}

impl Field {
    pub(crate) fn text() -> Self {
        Field::Text(TextInput::default())
    }

    /// A text field that shows `hint`, greyed, while it is empty.
    pub(crate) fn text_with_placeholder(hint: &'static str) -> Self {
        Field::Text(TextInput {
            placeholder: Some(hint),
            ..TextInput::default()
        })
    }

    pub(crate) fn masked() -> Self {
        Field::Secret(SecretInput::default())
    }

    pub(crate) fn choice(options: &'static [&'static str], selected: usize) -> Self {
        Field::Choice { options, selected }
    }

    /// Whether typing a character inserts it here.
    fn is_typed(&self) -> bool {
        matches!(self, Field::Text(_) | Field::Secret(_))
    }

    fn insert(&mut self, c: char) {
        match self {
            Field::Text(input) => input.insert(c),
            Field::Secret(input) => input.insert(c),
            Field::Choice { .. } => {}
        }
    }

    fn apply(&mut self, action: Action) {
        match self {
            Field::Text(input) => match action {
                Action::Left => input.move_left(),
                Action::Right => input.move_right(),
                Action::LineStart => input.move_home(),
                Action::LineEnd => input.move_end(),
                Action::DeleteBack => input.delete_back(),
                Action::DeleteForward => input.delete_forward(),
                _ => {}
            },
            Field::Secret(input) => match action {
                Action::Left => input.move_left(),
                Action::Right => input.move_right(),
                Action::LineStart => input.move_home(),
                Action::LineEnd => input.move_end(),
                Action::DeleteBack => input.delete_back(),
                Action::DeleteForward => input.delete_forward(),
                _ => {}
            },
            Field::Choice { options, selected } => match action {
                Action::Right | Action::NextChoice => {
                    *selected = (*selected + 1) % options.len();
                }
                Action::Left => *selected = (*selected + options.len() - 1) % options.len(),
                _ => {}
            },
        }
    }
}

pub(crate) trait Form {
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
    /// What the form asks for when it is accepted, or why it was refused.
    fn submit(&self) -> Result<Submission, String>;

    /// Shown next to the title, for what the title cannot carry.
    fn subtitle(&self) -> Option<String> {
        None
    }

    /// Called after a key changed the form, so a form whose fields depend on
    /// each other can add or remove one.
    fn after_change(&mut self) {}

    /// What the form is waiting for, shown in place of the hint. While this
    /// is set the reducer ignores Submit, so one Enter is one attempt.
    fn pending(&self) -> Option<&'static str> {
        None
    }

    fn set_pending(&mut self, pending: Option<&'static str>) {
        let _ = pending;
    }

    fn text(&self, field: usize) -> &str {
        match &self.fields()[field] {
            Field::Text(input) => &input.value,
            Field::Secret(_) | Field::Choice { .. } => "",
        }
    }

    /// What a password field holds; `None` for any other field.
    fn secret(&self, field: usize) -> Option<&Password> {
        match &self.fields()[field] {
            Field::Secret(input) => Some(&input.value),
            Field::Text(_) | Field::Choice { .. } => None,
        }
    }

    fn selected(&self, field: usize) -> usize {
        match &self.fields()[field] {
            Field::Choice { selected, .. } => *selected,
            Field::Text(_) | Field::Secret(_) => 0,
        }
    }

    /// Whether the focused field types characters, so a plain character key
    /// goes to it before any binding.
    fn is_typing(&self) -> bool {
        self.fields()[self.focus()].is_typed()
    }

    /// `action` is what the `Dialog` keymap binds `key` to. In a text field,
    /// plain characters are typed even if bound, so Space types a space
    /// there.
    fn handle_key(&mut self, key: KeyEvent, action: Option<Action>) {
        let focus = self.focus();
        let count = self.labels().len();

        if let Some(c) = typed_char(key)
            && self.fields()[focus].is_typed()
        {
            self.set_error(None);
            self.fields_mut()[focus].insert(c);
            self.after_change();
            return;
        }

        match action {
            Some(Action::NextField) => self.set_focus((focus + 1) % count),
            Some(Action::PrevField) => self.set_focus((focus + count - 1) % count),
            // Submitting is the reducer's job: it decides what follows.
            Some(Action::Submit) => {}
            Some(action) => {
                self.set_error(None);
                self.fields_mut()[focus].apply(action);
                self.after_change();
            }
            None => {}
        }
    }
}

#[cfg(test)]
#[path = "form_tests.rs"]
pub(crate) mod form_tests;
