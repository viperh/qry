use super::{
    Submission,
    form::{Field, Form},
};
use crate::{app::state::SessionId, secrets::Password};

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
    /// The session whose worker asked, which the answer goes back to.
    session: SessionId,
    /// The id of the connection asking, which the stash keys the password
    /// by.
    connection: String,
    /// Whose password this is, for the title.
    name: String,
}

impl PasswordForm {
    pub(crate) fn new(
        session: SessionId,
        connection: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            fields: [Field::masked()],
            error: None,
            pending: None,
            session,
            connection: connection.into(),
            name: name.into(),
        }
    }

    pub(crate) fn session(&self) -> SessionId {
        self.session
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
        Ok(Submission::PasswordEntered {
            session: self.session,
            connection: self.connection.clone(),
            password: typed.unwrap_or_default(),
        })
    }
}

#[cfg(test)]
#[path = "passwordform_tests.rs"]
mod passwordform_tests;
