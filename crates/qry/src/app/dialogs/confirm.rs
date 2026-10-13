//! The confirm dialog: a message, then ` Cancel ` and a button that does
//! what is asked. A destructive one is coral and starts on Cancel.

use super::{
    Submission,
    form::{Field, Form},
};
use crate::{action::Action, app::state::SessionId, theme::Accent};

/// What confirming does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OnConfirm {
    /// Close this session, though a query is running there.
    CloseSession(SessionId),
    /// Forget the stored connection with this id, and its password.
    DeleteConnection(String),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Confirm {
    title: &'static str,
    message: String,
    confirm_label: &'static str,
    destructive: bool,
    on_confirm: OnConfirm,
    /// Whether the confirm button, not Cancel, is selected.
    confirm_selected: bool,
    error: Option<String>,
}

impl Confirm {
    pub(crate) fn new(
        title: &'static str,
        message: impl Into<String>,
        confirm_label: &'static str,
        destructive: bool,
        on_confirm: OnConfirm,
    ) -> Self {
        Self {
            title,
            message: message.into(),
            confirm_label,
            destructive,
            on_confirm,
            confirm_selected: !destructive,
            error: None,
        }
    }

    /// Asks before closing a session whose query is still running.
    pub(crate) fn close_session(session: SessionId, title: &str) -> Self {
        Self::new(
            "Close Session",
            format!("{title} is running a query. Closing the session cancels it."),
            "Close",
            true,
            OnConfirm::CloseSession(session),
        )
    }

    /// Asks before forgetting a stored connection.
    pub(crate) fn delete_connection(id: &str, name: &str, keychain: bool) -> Self {
        let message = if keychain {
            format!("Delete \"{name}\" and forget its stored password? This cannot be undone.")
        } else {
            format!("Delete \"{name}\"? This cannot be undone.")
        };
        Self::new(
            "Delete Connection",
            message,
            "Delete",
            true,
            OnConfirm::DeleteConnection(id.to_string()),
        )
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn confirm_label(&self) -> &'static str {
        self.confirm_label
    }

    pub(crate) fn destructive(&self) -> bool {
        self.destructive
    }

    pub(crate) fn confirm_selected(&self) -> bool {
        self.confirm_selected
    }

    pub(crate) fn select(&mut self, confirm: bool) {
        self.confirm_selected = confirm;
    }
}

impl Form for Confirm {
    fn title(&self) -> &'static str {
        self.title
    }

    fn footer(&self) -> &'static [(Action, &'static str)] {
        &[(Action::Submit, "choose"), (Action::Cancel, "cancel")]
    }

    fn accent(&self) -> Accent {
        if self.destructive {
            Accent::Danger
        } else {
            Accent::Act
        }
    }

    /// Wide enough for the message on a line or two, within 40 to 72.
    fn width(&self) -> u16 {
        let chars = u16::try_from(self.message.chars().count()).unwrap_or(u16::MAX);
        (chars / 2 + 6).clamp(40, 72)
    }

    fn labels(&self) -> &'static [&'static str] {
        &[]
    }

    fn fields(&self) -> &[Field] {
        &[]
    }

    fn fields_mut(&mut self) -> &mut [Field] {
        &mut []
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
        Ok(if self.confirm_selected {
            Submission::Confirmed(self.on_confirm.clone())
        } else {
            Submission::Dismissed
        })
    }

    /// ← / → / Tab move between the two buttons.
    fn act(&mut self, action: Action) {
        if matches!(
            action,
            Action::Left
                | Action::Right
                | Action::NextField
                | Action::PrevField
                | Action::NextChoice
        ) {
            self.confirm_selected = !self.confirm_selected;
        }
    }
}

#[cfg(test)]
#[path = "confirm_tests.rs"]
mod confirm_tests;
