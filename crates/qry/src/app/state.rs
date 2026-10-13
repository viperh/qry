//! The app's state, as plain data.

use serde::{Deserialize, Serialize};
use strum::Display;

/// One open connection's worker. There is a single session for now, so the
/// id is always [`SessionId::FIRST`]; several come with multiple sessions.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionId(pub u32);

impl SessionId {
    pub const FIRST: Self = Self(0);
}

/// The status bar's line.
#[derive(Debug, Clone, Default, PartialEq, Eq, Display, Serialize, Deserialize)]
pub enum StatusCode {
    #[default]
    None,
    Success(String),
    Error(String),
}
