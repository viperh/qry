//! The confirm dialog: a message, then ` Cancel ` and a button that does
//! what is asked. A destructive one is coral and starts on Cancel.

use super::{
    Submission,
    exportform::ExportForm,
    form::{Field, Form},
};
use crate::{action::Action, app::state::SessionId, theme::Accent};

/// What confirming does.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum OnConfirm {
    /// Export as this dialog says, over the file that is there. Cancelling
    /// goes back to the dialog.
    OverwriteExport(Box<ExportForm>),
    /// Export every row as this dialog says, running again a statement
    /// that may change data. Cancelling goes back to the dialog.
    RerunExport(Box<ExportForm>),
    /// Close this session, though a query is running there.
    CloseSession(SessionId),
    /// Forget the stored connection with this id, and, as its checkboxes
    /// say, its keychain password and its query history.
    DeleteConnection {
        id: String,
        forget_secret: bool,
        delete_history: bool,
    },
    /// Run the statements waiting on this `prod` session, writes and all.
    RunOnProd(SessionId),
    /// Replace the existing backup at `path` with these connections.
    OverwriteConnections { path: String, ids: Vec<String> },
    /// Roll back these sessions' open transactions, then quit.
    QuitRollingBack(Vec<SessionId>),
    /// Delete the snippet with this name and scope.
    DeleteSnippet {
        name: String,
        connection: Option<String>,
    },
}

/// What a confirm's checkbox decides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Check {
    ForgetSecret,
    DeleteHistory,
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
    /// Checkboxes under the message, and what each decides.
    fields: Vec<Field>,
    checks: Vec<Check>,
    labels: Vec<&'static str>,
    focus: usize,
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
            fields: Vec::new(),
            checks: Vec::new(),
            labels: Vec::new(),
            focus: 0,
        }
    }

    /// Adds a checkbox under the message.
    pub(crate) fn with_check(mut self, check: Check, caption: &str, checked: bool) -> Self {
        self.fields.push(Field::checkbox(checked, caption));
        self.checks.push(check);
        self.labels.push("");
        self
    }

    /// Whether checkbox `check` is there and ticked.
    pub(crate) fn is_checked(&self, check: Check) -> bool {
        self.checks
            .iter()
            .position(|c| *c == check)
            .is_some_and(|i| self.checked(i))
    }

    /// Asks before running a statement that writes on a `prod` connection,
    /// once per session.
    pub(crate) fn run_on_prod(session: SessionId, title: &str) -> Self {
        Self::new(
            "Write to Production",
            format!(
                "{title} is labelled prod and this run changes data. Run it? qry will not ask again in this session."
            ),
            "Run",
            false,
            OnConfirm::RunOnProd(session),
        )
    }

    /// Asks before an export replaces the file `name`.
    pub(crate) fn overwrite_export(name: &str, form: ExportForm) -> Self {
        Self::new(
            "File Exists",
            format!("{name} exists — overwrite?"),
            "Overwrite",
            true,
            OnConfirm::OverwriteExport(Box::new(form)),
        )
    }

    /// Asks before an export of every row runs a statement that is not a
    /// plain read again.
    pub(crate) fn rerun_export(form: ExportForm) -> Self {
        Self::new(
            "Run Again",
            "re-running may change data: the statement is not a SELECT. Run it again to export every row?",
            "Run again",
            true,
            OnConfirm::RerunExport(Box::new(form)),
        )
    }

    /// The Export dialog a cancelled export confirm goes back to.
    pub(crate) fn export_form(&self) -> Option<&ExportForm> {
        match &self.on_confirm {
            OnConfirm::OverwriteExport(form) | OnConfirm::RerunExport(form) => Some(form),
            _ => None,
        }
    }

    /// Asks before replacing an existing backup file.
    pub(crate) fn overwrite_connections(path: String, ids: Vec<String>) -> Self {
        Self::new(
            "Replace File",
            format!("{path} already exists. Replace it with this backup?"),
            "Replace",
            false,
            OnConfirm::OverwriteConnections { path, ids },
        )
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

    /// Asks before deleting a snippet.
    pub(crate) fn delete_snippet(name: &str, connection: Option<String>) -> Self {
        Self::new(
            "Delete Snippet",
            format!("Delete the snippet \"{name}\"? This cannot be undone."),
            "Delete",
            true,
            OnConfirm::DeleteSnippet {
                name: name.to_string(),
                connection,
            },
        )
    }

    /// Asks before closing a session with an open transaction: dropping
    /// the connection rolls it back.
    pub(crate) fn close_in_transaction(session: SessionId, title: &str) -> Self {
        Self::new(
            "Close Session",
            format!("{title} has an open transaction. Closing the session rolls it back."),
            "Roll back & close",
            true,
            OnConfirm::CloseSession(session),
        )
    }

    /// Asks before quitting with transactions open in `sessions`, named
    /// `titles`.
    pub(crate) fn quit_in_transaction(sessions: Vec<SessionId>, titles: &str) -> Self {
        Self::new(
            "Open Transaction",
            format!(
                "{titles} {} an open transaction. Quitting rolls it back; cancel to commit first.",
                if sessions.len() == 1 { "has" } else { "have" }
            ),
            "Roll back & quit",
            true,
            OnConfirm::QuitRollingBack(sessions),
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

    /// Wide enough for the message on a line or two, within 40 (56 with
    /// checkboxes) to 72.
    fn width(&self) -> u16 {
        let chars = u16::try_from(self.message.chars().count()).unwrap_or(u16::MAX);
        // With checkboxes the keys go in the border, which needs room.
        let least = if self.fields.is_empty() { 40 } else { 56 };
        (chars / 2 + 6).clamp(least, 72)
    }

    fn labels(&self) -> &[&'static str] {
        &self.labels
    }

    fn fields(&self) -> &[Field] {
        &self.fields
    }

    fn fields_mut(&mut self) -> &mut [Field] {
        &mut self.fields
    }

    fn focus(&self) -> usize {
        self.focus
    }

    fn set_focus(&mut self, focus: usize) {
        self.focus = focus.min(self.fields.len().saturating_sub(1));
    }

    fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn set_error(&mut self, error: Option<String>) {
        self.error = error;
    }

    fn submit(&self) -> Result<Submission, String> {
        if !self.confirm_selected {
            return Ok(Submission::Dismissed);
        }
        let on_confirm = match &self.on_confirm {
            OnConfirm::DeleteConnection { id, .. } => OnConfirm::DeleteConnection {
                id: id.clone(),
                forget_secret: self.is_checked(Check::ForgetSecret),
                delete_history: self.is_checked(Check::DeleteHistory),
            },
            other => other.clone(),
        };
        Ok(Submission::Confirmed(on_confirm))
    }

    /// ← / → move between the two buttons. With checkboxes, Tab and the
    /// arrows up and down move between them and Space ticks one; without,
    /// Tab and Space move between the buttons too.
    fn act(&mut self, action: Action) {
        let count = self.fields.len();
        match action {
            Action::Left | Action::Right => self.confirm_selected = !self.confirm_selected,
            Action::NextField | Action::Down if count > 0 => self.focus = (self.focus + 1) % count,
            Action::PrevField | Action::Up if count > 0 => {
                self.focus = (self.focus + count - 1) % count;
            }
            Action::NextChoice if count > 0 => {
                if let Some(Field::Checkbox { checked, .. }) = self.fields.get_mut(self.focus) {
                    *checked = !*checked;
                }
            }
            Action::NextField | Action::PrevField | Action::NextChoice => {
                self.confirm_selected = !self.confirm_selected;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "confirm_tests.rs"]
mod confirm_tests;
