use crate::{
    action::Action,
    components::form::{Field, Form},
    secrets::{self, Password},
};

const LABELS: [&str; 1] = ["Password"];
const PASSWORD: usize = 0;

/// Asked for when a connection's password is not stored, or the keychain
/// could not be reached. What is typed goes to the stash, never into an
/// action, and the worker takes it from there.
pub struct PasswordForm {
    fields: [Field; 1],
    error: Option<String>,
    pending: Option<&'static str>,
    /// Whose password this is, for the title.
    name: String,
}

impl PasswordForm {
    pub fn new(name: impl Into<String>) -> Self {
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

    fn submit(&self) -> Result<Action, String> {
        // Not trimmed: spaces can be part of a password.
        secrets::stash(Password::new(self.text(PASSWORD).to_string()));
        Ok(Action::PasswordEntered)
    }
}

#[cfg(test)]
#[path = "passwordform_tests.rs"]
mod passwordform_tests;
