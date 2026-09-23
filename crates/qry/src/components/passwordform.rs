use crate::action::Action;
use crate::components::form::{Field, Form};
use crate::secrets::{self, Password};

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
mod tests {
    use super::*;
    use crate::components::form::tests::{lines, render, type_str};

    #[test]
    fn what_is_typed_goes_to_the_stash_and_not_to_the_action() {
        let _guard = secrets::test_lock();
        let _ = secrets::take_stash();
        let mut form = PasswordForm::new("prod");
        type_str(&mut form, " hunter2 ");

        assert_eq!(form.submit().unwrap(), Action::PasswordEntered);
        assert_eq!(
            secrets::take_stash().map(|p| p.as_str().to_string()),
            Some(" hunter2 ".to_string())
        );
    }

    #[test]
    fn the_popup_names_the_connection_and_masks_what_is_typed() {
        let mut form = PasswordForm::new("prod");
        type_str(&mut form, "hunter2");
        let shown = lines(&render(&form, 80, 24)).join("\n");

        assert!(shown.contains("Password"), "{shown}");
        assert!(shown.contains("for prod"), "{shown}");
        assert!(shown.contains("•••••••"), "{shown}");
        assert!(!shown.contains("hunter2"), "{shown}");
    }
}
