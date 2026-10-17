use super::{
    Submission,
    form::{Field, Form},
};
use crate::action::Action;

const LABELS: [&str; 1] = ["Row"];

/// The results' goto: a row number, or a percentage of the way down.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GotoForm {
    fields: [Field; 1],
    error: Option<String>,
}

impl Default for GotoForm {
    fn default() -> Self {
        Self {
            fields: [Field::text_with_placeholder("42 or 50%")],
            error: None,
        }
    }
}

impl Form for GotoForm {
    fn title(&self) -> &'static str {
        "Go to Row"
    }

    fn footer(&self) -> &'static [(Action, &'static str)] {
        &[(Action::Submit, "go"), (Action::Cancel, "cancel")]
    }

    fn width(&self) -> u16 {
        40
    }

    fn labels(&self) -> &'static [&'static str] {
        &LABELS
    }

    fn fields(&self) -> &[Field] {
        &self.fields
    }

    fn fields_mut(&mut self) -> &mut [Field] {
        &mut self.fields
    }

    fn focus(&self) -> usize {
        0
    }

    fn set_focus(&mut self, _focus: usize) {}

    fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn set_error(&mut self, error: Option<String>) {
        self.error = error;
    }

    fn submit(&self) -> Result<Submission, String> {
        let text = self.text(0).trim();
        if text.is_empty() {
            return Err("A row number or a percentage".into());
        }
        Ok(Submission::Goto(text.to_string()))
    }
}
