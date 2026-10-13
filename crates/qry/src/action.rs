use serde::{Deserialize, Serialize};
use strum::Display;

use crate::config::KeySection;

/// What a key in the `keybindings` config can be bound to. The reducer
/// ([`App::handle`](crate::app::App::handle)) interprets these; nothing sends
/// them around as messages.
#[derive(Debug, Clone, PartialEq, Eq, Display, Serialize, Deserialize)]
pub enum Action {
    Quit,
    Suspend,
    ClearScreen,
    Help,
    FocusNext,
    FocusPrev,
    /// Opens the modal whose keys are in that section, freshly filled in;
    /// `Home` closes whatever modal is open.
    ChangeMode(KeySection),
    /// Runs this SQL on the open connection. The editor's run key sends
    /// whatever has been typed.
    Execute(String),
}
