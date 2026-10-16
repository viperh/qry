//! The key-hint line, after tachy's `hints.rs`: a few `key label` pairs for
//! where the keys go now, with the help's key pinned at the right.
//!
//! Keys come from the live keymap, so a remapped key shows its new chord
//! and an action bound to nothing has no pair.

use ratatui::{prelude::*, widgets::Paragraph};

use crate::{
    action::Action,
    app::state::State,
    keymap::{KeyContext, display_chord},
    theme::Theme,
};

/// A hint: the first of its actions that has a key, and what it does.
type Hint = (&'static [Action], &'static str);

/// The hints of each context, most useful first.
pub(crate) fn table(context: KeyContext) -> &'static [Hint] {
    match context {
        KeyContext::Tree => &[
            (&[Action::Open], "open"),
            (&[Action::Toggle], "expand"),
            (&[Action::NewConnection], "new conn"),
            (&[Action::FocusNext], "next pane"),
            (&[Action::Palette], "commands"),
            (&[Action::Quit], "quit"),
        ],
        KeyContext::Editor => &[
            (&[Action::RunStatement], "run"),
            (&[Action::RunAll], "run all"),
            (&[Action::FocusNext], "next pane"),
            (&[Action::Palette], "commands"),
            (&[Action::NewConnection], "new conn"),
            (&[Action::Quit], "quit"),
        ],
        KeyContext::Results => &[
            (&[Action::Inspect], "inspect"),
            (&[Action::CopyCell], "copy"),
            (&[Action::ExportResults], "export"),
            (&[Action::FocusNext], "next pane"),
            (&[Action::Palette], "commands"),
            (&[Action::Quit], "quit"),
        ],
        KeyContext::Jobs => &[
            (&[Action::Down, Action::Up], "move"),
            (&[Action::CancelQuery], "cancel"),
            (&[Action::Open], "show"),
            (&[Action::ToggleJobs], "close"),
        ],
        KeyContext::Inspector => &[
            (&[Action::Down, Action::Up], "field"),
            (&[Action::Inspect], "value"),
            (&[Action::CopyCell], "copy"),
            (&[Action::FocusResults], "grid"),
            (&[Action::Quit], "quit"),
        ],
        KeyContext::Dialog => &[
            (&[Action::Submit], "submit"),
            (&[Action::NextField], "next field"),
            (&[Action::Cancel], "cancel"),
        ],
        KeyContext::Help => &[
            (&[Action::Down, Action::Up], "scroll"),
            (&[Action::Cancel, Action::Help], "close"),
        ],
        KeyContext::Palette => &[
            (&[Action::Submit], "run"),
            (&[Action::Down, Action::Up], "move"),
            (&[Action::Cancel], "close"),
        ],
        KeyContext::Global => &[(&[Action::Quit], "quit")],
    }
}

/// The key the context stack reaches `action` with, as the help shows it.
fn key_for(state: &State, contexts: &[KeyContext], action: Action) -> Option<String> {
    let keymap = &state.config.keymap;
    contexts.iter().find_map(|context| {
        let chords = keymap.chords_for(*context, &action);
        (!chords.is_empty()).then(|| display_chord(&chords))
    })
}

/// The `(key, label)` pairs for where the keys go now, and the help's.
pub(crate) fn pairs(state: &State) -> (Vec<(String, &'static str)>, Option<String>) {
    let contexts = state.contexts();
    // While a statement runs, how to stop it comes first.
    let running = matches!(contexts[0], KeyContext::Editor | KeyContext::Results)
        && state.jobs.running(state.sessions.active().id).is_some();
    let cancel: &[Hint] = if running {
        &[(&[Action::CancelQuery], "cancel")]
    } else {
        &[]
    };
    let pairs = cancel
        .iter()
        .chain(table(contexts[0]))
        .filter_map(|(actions, label)| {
            let key = actions.iter().find_map(|a| key_for(state, &contexts, *a))?;
            Some((key, *label))
        })
        .collect();
    (pairs, key_for(state, &contexts, Action::Help))
}

fn pair(key: &str, label: &str, theme: &Theme) -> [Span<'static>; 2] {
    [
        Span::styled(key.to_string(), theme.hint_key()),
        Span::styled(format!(" {label}"), theme.hint()),
    ]
}

pub(crate) fn draw(state: &State, theme: &Theme, frame: &mut Frame, area: Rect) {
    let (pairs, help) = pairs(state);
    let help: Vec<Span> = help
        .map(|key| {
            let mut spans = pair(&key, "help", theme).to_vec();
            spans.push(Span::raw(" "));
            spans
        })
        .unwrap_or_default();
    let help_width: usize = help.iter().map(Span::width).sum();
    let room = usize::from(area.width).saturating_sub(help_width + 2);

    let mut spans = vec![Span::raw(" ")];
    let mut used = 1;
    for (key, label) in &pairs {
        let p = pair(key, label, theme);
        let w: usize = p.iter().map(Span::width).sum::<usize>() + 2;
        // Whole pairs go from the right when it is narrow.
        if used + w > room {
            break;
        }
        spans.extend(p);
        spans.push(Span::raw("  "));
        used += w;
    }
    frame.render_widget(Paragraph::new(Line::from(spans)).style(theme.base()), area);
    let help_width = u16::try_from(help_width).unwrap_or(u16::MAX);
    if help_width > 0 && help_width <= area.width {
        let at = Rect::new(area.right() - help_width, area.y, help_width, 1);
        frame.render_widget(Line::from(help), at);
    }
}

#[cfg(test)]
#[path = "hints_tests.rs"]
mod hints_tests;
