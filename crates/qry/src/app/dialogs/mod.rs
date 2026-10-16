//! The modals: their fields, their keys and what submitting them asks for.
//! They are plain data in [`State`](super::state::State); the reducer opens,
//! submits and closes them, and [`crate::views::form`] draws them.

pub(crate) mod confirm;
pub(crate) mod connexportform;
pub(crate) mod connform;
pub(crate) mod copyform;
pub(crate) mod exportform;
pub(crate) mod form;
pub(crate) mod gotoform;
pub(crate) mod importform;
pub(crate) mod passwordform;
pub(crate) mod snippetform;
pub(crate) mod textinput;

use qry_core::ExportConfig;

use self::{
    confirm::{Confirm, OnConfirm},
    connexportform::ConnExportForm,
    connform::ConnForm,
    copyform::CopyForm,
    exportform::ExportForm,
    form::Form,
    gotoform::GotoForm,
    importform::ImportForm,
    passwordform::PasswordForm,
    snippetform::{SnippetForm, SnippetSave},
};
use super::state::SessionId;
use crate::{connections::StoredConnection, import::Format, secrets::Password};

/// The open modal.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum DialogState {
    Connection(ConnForm),
    Export(ExportForm),
    Password(PasswordForm),
    Import(ImportForm),
    ConnExport(ConnExportForm),
    Confirm(Confirm),
    Copy(CopyForm),
    Goto(GotoForm),
    Snippet(SnippetForm),
}

impl DialogState {
    pub(crate) fn form(&self) -> &dyn Form {
        match self {
            Self::Snippet(form) => form,
            Self::Connection(form) => form,
            Self::Export(form) => form,
            Self::Password(form) => form,
            Self::Import(form) => form,
            Self::ConnExport(form) => form,
            Self::Confirm(form) => form,
            Self::Copy(form) => form,
            Self::Goto(form) => form,
        }
    }

    pub(crate) fn form_mut(&mut self) -> &mut dyn Form {
        match self {
            Self::Snippet(form) => form,
            Self::Connection(form) => form,
            Self::Export(form) => form,
            Self::Password(form) => form,
            Self::Import(form) => form,
            Self::ConnExport(form) => form,
            Self::Confirm(form) => form,
            Self::Copy(form) => form,
            Self::Goto(form) => form,
        }
    }
}

/// What an accepted form asks for. The reducer turns it into effects, and
/// into what the modal waits for.
#[derive(Debug, PartialEq)]
pub(crate) enum Submission {
    /// Open this connection. The record holds no password: one typed into
    /// the form rides beside it, for the runtime to leave in the stash.
    Connect {
        record: Box<StoredConnection>,
        password: Option<Password>,
    },
    /// The password prompt's answer, for the connection the worker asked
    /// about (its id) and the session whose worker asked.
    PasswordEntered {
        session: SessionId,
        connection: String,
        password: Password,
    },
    /// Write the last result to a file.
    Export(ExportConfig),
    /// Read connections from another file.
    Import { path: String, format: Format },
    /// Write a backup of the saved connections to this file.
    ExportConnections(String),
    /// A confirm dialog's confirm button.
    Confirmed(OnConfirm),
    /// A confirm dialog's Cancel button.
    Dismissed,
    /// Copy this much of the result (cell, row, column, all) as this.
    Copy {
        scope: usize,
        format: qry_core::copy::CopyFormat,
    },
    /// Go to this row of the results: `N` or `N%`.
    Goto(String),
    /// Save or rename a snippet.
    SaveSnippet(SnippetSave),
}

/// The form helpers (`key`, `press`, `type_str`, `render`), shared by every
/// form's tests.
#[cfg(test)]
pub(crate) use form::form_tests;
