use std::sync::Arc;

use qry_core::{ConnectionConfig, QueryResult};
use serde::{Deserialize, Serialize};
use strum::Display;
use qry_core::ExportConfig;
use crate::app::Mode;

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
    /// Sent by the New Connection form: the connection's name (may be empty)
    /// and how to reach it.
    Connect(String, ConnectionConfig),
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