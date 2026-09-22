use std::sync::Arc;

use qry_core::{ConnectionConfig, QueryResult};
use serde::{Deserialize, Serialize};
use strum::Display;
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
    /// Sent by the database worker. Wrapped in `Arc` because every action is
    /// cloned once per component.
    QueryDone(Arc<QueryResult>),
    Export
}


#[derive(Debug, Clone, Default, PartialEq, Eq, Display, Serialize, Deserialize)]
pub enum StatusCode {
    #[default]
    None,
    Success(String),
    Error(String),
}