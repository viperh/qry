//! Clicks and the wheel. The layout is worked out again from the size, the
//! same pure function the views draw with, so a click lands on what was
//! drawn there.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use super::{results, sessions, tree};
use crate::{
    action::Action,
    app::{
        effects::Effect,
        state::{Focus, State},
    },
    views::{
        layout::{Hit, compute_layout, hit_test},
        top_bar,
    },
};

/// How many rows one notch of the wheel moves.
const WHEEL_ROWS: usize = 3;

pub(super) fn mouse(state: &mut State, mouse: MouseEvent, fx: &mut Vec<Effect>) {
    // An overlay takes the keys alone, and the mouse with them.
    if !state.overlays.is_empty() {
        return;
    }
    let area = Rect::new(0, 0, state.size.0, state.size.1);
    let Some(layout) = compute_layout(&state.layout_input(area)) else {
        return;
    };
    let tabs = top_bar::tab_hits(state, layout.top_bar);
    let Some(hit) = hit_test(&layout, &tabs, mouse.column, mouse.row) else {
        return;
    };
    match (mouse.kind, hit) {
        (MouseEventKind::Down(MouseButton::Left), Hit::Tab(i)) => {
            if let Some(id) = state.sessions.order.get(i).copied() {
                sessions::switch(state, id);
            }
        }
        (MouseEventKind::Down(MouseButton::Left), Hit::Tree) => state.focus = Focus::Tree,
        (MouseEventKind::Down(MouseButton::Left), Hit::Editor) => state.focus = Focus::Editor,
        (MouseEventKind::Down(MouseButton::Left), Hit::Results) => state.focus = Focus::Results,
        (MouseEventKind::ScrollDown | MouseEventKind::ScrollUp, hit) => {
            let action = if mouse.kind == MouseEventKind::ScrollDown {
                Action::Down
            } else {
                Action::Up
            };
            for _ in 0..WHEEL_ROWS {
                match hit {
                    Hit::Tree => tree::run(state, action, fx),
                    Hit::Results => results::run(state, action, fx),
                    Hit::Editor | Hit::Tab(_) => {}
                }
            }
        }
        _ => {}
    }
}
