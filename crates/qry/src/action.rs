use std::sync::Arc;

use qry_core::QueryResult;
use serde::{Deserialize, Serialize};
use strum::Display;
use qry_core::ExportConfig;
use crate::app::Mode;
use crate::connections::StoredConnection;

/// Messages passed between the event loop, [`App`](crate::app::App) and every
/// [`Component`](crate::components::Component).
///
/// Add your own variants here; components return them from `update` /
/// `handle_key_event` and `App::handle_actions` dispatches them.
#[derive(Debug, Clone, PartialEq, Eq, Display, Serialize, Deserialize)]
pub enum Action {
    Tick,
    Render,
    Resize(u16, u16),
    Suspend,
    Resume,
    Quit,
    ClearScreen,
    Error(String),
    Help,
    Info(String),
    Status(StatusCode),
    FocusNext,
    FocusPrev,
    ChangeMode(Mode),
    Execute(String),
    /// Sent by the New Connection form and by the tree. It carries the
    /// stored record, which holds no password: the worker resolves the
    /// secret itself, so no password ever travels in an action.
    Connect(Box<StoredConnection>),
    /// The worker needs a password typed before it can open this connection.
    /// The prompt leaves it in `secrets::stash`, never in an action.
    NeedPassword { id: String, name: String },
    /// A password is waiting in the stash for the connection that asked.
    PasswordEntered,
    /// Open this connection in the New Connection form, to change it.
    EditConnection(Box<StoredConnection>),
    /// Remove this connection's password from the keychain.
    ForgetSecret(String),
    /// The keychain refused a password, so the connection must ask for one
    /// every time instead.
    SecretNotStored(String),
    /// The connections read from disk at startup.
    ConnectionsLoaded(Vec<StoredConnection>),
    /// Write these to disk; sent by the tree when its list changes.
    SaveConnections(Vec<StoredConnection>),
    /// The worker's answer to a [`Action::Connect`]: the label it connected
    /// to, or why it could not. The New Connection modal stays open until one
    /// of these arrives, so a typo does not cost the whole form.
    Connected(String),
    ConnectFailed(String),
    /// The worker's answer to an [`Action::Export`], the same way round: the
    /// path it wrote, or why it could not.
    Exported(String),
    ExportFailed(String),
    /// Asked for by the tree when a connection is expanded, and answered by
    /// the worker with the tables of whatever is connected.
    ListTables,
    TablesLoaded(Vec<String>),
    TablesFailed(String),
    /// Sent by the database worker. Wrapped in `Arc` because every action is
    /// cloned once per component.
    QueryDone(Arc<QueryResult>),
    Export(ExportConfig),
}


#[derive(Debug, Clone, Default, PartialEq, Eq, Display, Serialize, Deserialize)]
pub enum StatusCode {
    #[default]
    None,
    Success(String),
    Error(String),
}