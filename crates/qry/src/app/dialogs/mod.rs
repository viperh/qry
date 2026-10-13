//! The modals: their fields, their keys and what submitting them asks for.
//! They are plain data in [`State`](super::state::State); the reducer opens,
//! submits and closes them, and [`crate::views::form`] draws them.

pub(crate) mod confirm;
pub(crate) mod connexportform;
pub(crate) mod connform;
pub(crate) mod exportform;
pub(crate) mod form;
pub(crate) mod importform;
pub(crate) mod passwordform;
pub(crate) mod textinput;

use qry_core::ExportConfig;

use self::{
    confirm::{Confirm, OnConfirm},
    connexportform::ConnExportForm,
    connform::ConnForm,
    exportform::ExportForm,
    form::Form,
    importform::ImportForm,
    passwordform::PasswordForm,
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
}

impl DialogState {
    pub(crate) fn form(&self) -> &dyn Form {
        match self {
            Self::Connection(form) => form,
            Self::Export(form) => form,
            Self::Password(form) => form,
            Self::Import(form) => form,
            Self::ConnExport(form) => form,
            Self::Confirm(form) => form,
        }
    }

    pub(crate) fn form_mut(&mut self) -> &mut dyn Form {
        match self {
            Self::Connection(form) => form,
            Self::Export(form) => form,
            Self::Password(form) => form,
            Self::Import(form) => form,
            Self::ConnExport(form) => form,
            Self::Confirm(form) => form,
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
}

/// The form helpers (`key`, `press`, `type_str`, `render`), shared by every
/// form's tests.
#[cfg(test)]
pub(crate) use form::form_tests;
