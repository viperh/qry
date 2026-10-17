//! The delete confirm: a coral [`Confirm`] with a checkbox for the keychain
//! password (only for a connection that keeps one there) and one for the
//! query history. The reducer's `tree::delete` carries it out.

use super::confirm::{Check, Confirm, OnConfirm};
use crate::connections::{Secret, StoredConnection};

/// Asks before forgetting `record`.
pub(crate) fn delete_connection(record: &StoredConnection) -> Confirm {
    let name = record.title();
    let confirm = Confirm::new(
        "Delete Connection",
        format!("Delete \"{name}\"? This cannot be undone."),
        "Delete",
        true,
        OnConfirm::DeleteConnection {
            id: record.id.clone(),
            forget_secret: false,
            delete_history: false,
        },
    );
    let confirm = if record.secret == Secret::Keyring {
        confirm.with_check(
            Check::ForgetSecret,
            "also remove its password from the keychain",
            true,
        )
    } else {
        confirm
    };
    confirm.with_check(Check::DeleteHistory, "also delete its query history", true)
}

#[cfg(test)]
#[path = "deleteconfirm_tests.rs"]
mod deleteconfirm_tests;
