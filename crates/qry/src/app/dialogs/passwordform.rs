use super::{
    Submission,
    form::{Field, Form},
};
use crate::secrets::Password;

const LABELS: [&str; 1] = ["Password"];
const PASSWORD: usize = 0;

/// Asked for when a connection's password is not stored, or the keychain
/// could not be reached. What is typed is typed into a [`Password`], which
/// the runtime leaves in the stash for the worker; it is never in an event,
/// and an effect only carries it redacted.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PasswordForm {
    fields: [Field; 1],
    error: Option<String>,
    pending: Option<&'static str>,
    /// Whose password this is, for the title.
    name: String,
}

impl PasswordForm {
    pub(crate) fn new(name: impl Into<String>) -> Self {
        Self {
            fields: [Field::masked()],
            error: None,
            pending: None,
            name: name.into(),
        }
    }
}

impl Form for PasswordForm {
    fn title(&self) -> &'static str {
        "Password"
    }

    fn hint(&self) -> &'static str {
        "Enter connect · Esc cancel"
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
        PASSWORD
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

    fn pending(&self) -> Option<&'static str> {
        self.pending
    }

    fn set_pending(&mut self, pending: Option<&'static str>) {
        self.pending = pending;
    }

    /// Shows whose password is wanted, since the title cannot carry it.
    fn subtitle(&self) -> Option<String> {
        Some(format!("for {}", self.name))
    }

    fn submit(&self) -> Result<Submission, String> {
        // Not trimmed: spaces can be part of a password.
        let typed = self.secret(PASSWORD).map(Password::duplicate);
        Ok(Submission::PasswordEntered(typed.unwrap_or_default()))
    }
}

#[cfg(test)]
#[path = "passwordform_tests.rs"]
mod passwordform_tests;
