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
use crate::{action::Action, secrets::Password, theme::Accent};

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
    /// On or off, toggled with Space (or ← / →), shown as `[x]` / `[ ]`
    /// before its caption.
    Checkbox {
        checked: bool,
        caption: String,
    },
    /// A button, `[ Test ]`, reached with Tab like a field. Enter on it does
    /// what the form says it does rather than submitting.
    Button {
        caption: &'static str,
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

    pub(crate) fn checkbox(checked: bool, caption: impl Into<String>) -> Self {
        Field::Checkbox {
            checked,
            caption: caption.into(),
        }
    }

    pub(crate) fn button(caption: &'static str) -> Self {
        Field::Button { caption }
    }

    /// Whether typing a character inserts it here.
    fn is_typed(&self) -> bool {
        matches!(self, Field::Text(_) | Field::Secret(_))
    }

    fn insert(&mut self, c: char) {
        match self {
            Field::Text(input) => input.insert(c),
            Field::Secret(input) => input.insert(c),
            Field::Choice { .. } | Field::Checkbox { .. } | Field::Button { .. } => {}
        }
    }

    pub(crate) fn apply(&mut self, action: Action) {
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
            Field::Checkbox { checked, .. } => {
                if matches!(action, Action::NextChoice | Action::Left | Action::Right) {
                    *checked = !*checked;
                }
            }
            Field::Button { .. } => {}
        }
    }
}

/// A line a form shows under its fields: green for good news, coral for
/// bad, teal while something is under way, dim for a remark.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Note {
    pub(crate) text: String,
    pub(crate) tone: Tone,
}

impl Note {
    pub(crate) fn new(tone: Tone, text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tone,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tone {
    Ok,
    Error,
    Busy,
}

/// The keys a form's footer offers by default.
const FOOTER: &[(Action, &str)] = &[(Action::Submit, "ok"), (Action::Cancel, "cancel")];

pub(crate) trait Form {
    fn title(&self) -> &'static str;
    /// The footer's `key label` pairs while there is no error and nothing
    /// pending. The keys come from the `Dialog` keymap, so an action bound
    /// to nothing is left out.
    fn footer(&self) -> &'static [(Action, &'static str)] {
        FOOTER
    }

    /// The colour of its border and title.
    fn accent(&self) -> Accent {
        Accent::Info
    }

    /// How wide it would like to be; a narrow terminal makes it smaller.
    fn width(&self) -> u16 {
        64
    }
    /// One label per field, in the same order as [`Form::fields`].
    fn labels(&self) -> &[&'static str];
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

    /// What is wrong with field `field`, shown in coral on a row of its own
    /// under it.
    fn field_error(&self, field: usize) -> Option<&str> {
        let _ = field;
        None
    }

    /// A line under the fields, such as a connection test's outcome.
    fn note(&self) -> Option<Note> {
        None
    }

    /// Checks the fields before a submit, marking what is wrong (and
    /// moving to the first such field). Returns whether all is well; the
    /// reducer submits only then.
    fn check(&mut self) -> bool {
        true
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
            Field::Secret(_)
            | Field::Choice { .. }
            | Field::Checkbox { .. }
            | Field::Button { .. } => "",
        }
    }

    /// What a password field holds; `None` for any other field.
    fn secret(&self, field: usize) -> Option<&Password> {
        match &self.fields()[field] {
            Field::Secret(input) => Some(&input.value),
            Field::Text(_)
            | Field::Choice { .. }
            | Field::Checkbox { .. }
            | Field::Button { .. } => None,
        }
    }

    fn selected(&self, field: usize) -> usize {
        match &self.fields()[field] {
            Field::Choice { selected, .. } => *selected,
            Field::Text(_) | Field::Secret(_) | Field::Checkbox { .. } | Field::Button { .. } => 0,
        }
    }

    /// Whether a checkbox is ticked; `false` for any other field.
    fn checked(&self, field: usize) -> bool {
        matches!(self.fields()[field], Field::Checkbox { checked: true, .. })
    }

    /// Whether the focused field types characters, so a plain character key
    /// goes to it before any binding.
    fn is_typing(&self) -> bool {
        self.fields().get(self.focus()).is_some_and(Field::is_typed)
    }

    /// `action` is what the `Dialog` keymap binds `key` to. In a text field,
    /// plain characters are typed even if bound, so Space types a space
    /// there.
    fn handle_key(&mut self, key: KeyEvent, action: Option<Action>) {
        let focus = self.focus();
        if let Some(c) = typed_char(key)
            && self.is_typing()
        {
            self.set_error(None);
            self.fields_mut()[focus].insert(c);
            self.after_change();
        } else if let Some(action) = action {
            self.act(action);
        }
    }

    /// Moves between fields, or edits or picks in the focused one.
    fn act(&mut self, action: Action) {
        let focus = self.focus();
        let count = self.labels().len();
        if count == 0 {
            return;
        }
        match action {
            Action::NextField => self.set_focus((focus + 1) % count),
            Action::PrevField => self.set_focus((focus + count - 1) % count),
            // Submitting is the reducer's job: it decides what follows.
            Action::Submit => {}
            action => {
                self.set_error(None);
                self.fields_mut()[focus].apply(action);
                self.after_change();
            }
        }
    }
}

#[cfg(test)]
#[path = "form_tests.rs"]
pub(crate) mod form_tests;
