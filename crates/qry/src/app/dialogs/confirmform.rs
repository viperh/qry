use super::{
    Submission,
    form::{Field, Form},
};
use crate::app::state::SessionId;

const LABELS: [&str; 1] = ["Session"];
const ONLY: usize = 0;
/// The one thing Enter can mean here.
const CLOSE: [&str; 1] = ["close it, cancelling the query"];

/// Asks before closing a session whose query is still running. Enter
/// closes it, Esc keeps it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ConfirmForm {
    fields: [Field; 1],
    error: Option<String>,
    session: SessionId,
    /// The session's title, for the subtitle.
    title: String,
}

impl ConfirmForm {
    pub(crate) fn close_session(session: SessionId, title: impl Into<String>) -> Self {
        Self {
            fields: [Field::choice(&CLOSE, 0)],
            error: None,
            session,
            title: title.into(),
        }
    }
}

impl Form for ConfirmForm {
    fn title(&self) -> &'static str {
        "Close Session"
    }

    fn hint(&self) -> &'static str {
        "Enter close · Esc keep it"
    }

    fn subtitle(&self) -> Option<String> {
        Some(format!("{} is running a query", self.title))
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
        ONLY
    }

    fn set_focus(&mut self, _focus: usize) {
        // One field: there is nowhere else to go.
    }

    fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn set_error(&mut self, error: Option<String>) {
        self.error = error;
    }

    fn submit(&self) -> Result<Submission, String> {
        Ok(Submission::CloseSession(self.session))
    }
}

#[cfg(test)]
#[path = "confirmform_tests.rs"]
mod confirmform_tests;
