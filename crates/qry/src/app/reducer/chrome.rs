//! The screen's chrome: the editor/results split and maximising a region.

use crate::{
    action::Action,
    app::state::{Focus, Region, State},
    views::layout::{SPLIT_RANGE, SPLIT_STEP},
};

pub(super) fn run(state: &mut State, action: Action) {
    let chrome = &mut state.chrome;
    match action {
        Action::GrowEditor | Action::ShrinkEditor => {
            let split = if action == Action::GrowEditor {
                chrome.split_percent + SPLIT_STEP
            } else {
                chrome.split_percent.saturating_sub(SPLIT_STEP)
            };
            chrome.split_percent = split.clamp(SPLIT_RANGE.0, SPLIT_RANGE.1);
            // Moving the split shows both regions again.
            chrome.maximised = None;
        }
        Action::ToggleMaximise => {
            chrome.maximised = match (chrome.maximised, state.focus) {
                (Some(_), _) => None,
                (None, Focus::Editor) => Some(Region::Editor),
                (None, Focus::Results) => Some(Region::Results),
                // The tree is not a right-hand region.
                (None, Focus::Tree) => None,
            };
        }
        _ => {}
    }
}
