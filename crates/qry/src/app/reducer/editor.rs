//! The query editor's keys.

use ratatui::{style::Style, text::Line, widgets::Block};
use ratatui_textarea::{CursorMove, Scrolling};

use super::execute;
use crate::{
    action::Action,
    app::{
        effects::Effect,
        state::{EditorState, State},
    },
    theme::Theme,
};

/// What an `Editor` binding does.
pub(super) fn run(state: &mut State, action: Action, fx: &mut Vec<Effect>) {
    match action {
        // Until statements are split (T302), the statement is everything.
        Action::RunStatement | Action::RunAll => {
            let sql = state.editor().query();
            execute(state, sql, fx);
        }
        Action::RunSelection => {
            let sql = selection(state.editor()).unwrap_or_else(|| state.editor().query());
            execute(state, sql, fx);
        }
        // No query can be cancelled until there are jobs (T305), so it
        // always copies.
        Action::CancelQuery => apply(state.editor_mut(), Action::Copy),
        action => apply(state.editor_mut(), action),
    }
}

/// The selected text, if anything is selected.
fn selection(editor: &EditorState) -> Option<String> {
    let ((start_row, start_col), (end_row, end_col)) = editor.textarea.selection_range()?;
    let lines = editor.textarea.lines();
    let mut text = String::new();
    for row in start_row..=end_row {
        let line = lines.get(row)?;
        let from = if row == start_row { start_col } else { 0 };
        let to = if row == end_row {
            end_col
        } else {
            line.chars().count()
        };
        text.extend(line.chars().skip(from).take(to.saturating_sub(from)));
        if row != end_row {
            text.push('\n');
        }
    }
    Some(text)
}

/// Styles the text area for the focus and the theme.
pub(super) fn restyle(editor: &mut EditorState, focused: bool, theme: &Theme) {
    let (border, title) = if focused {
        (theme.focused_border(), theme.pane_title_focused())
    } else {
        (theme.border(), theme.pane_title())
    };
    let textarea = &mut editor.textarea;
    textarea.set_block(
        Block::bordered()
            .border_style(border)
            .title(Line::styled("Query", title)),
    );
    textarea.set_cursor_style(if focused {
        theme.editor_cursor()
    } else {
        Style::new()
    });
    textarea.set_cursor_line_style(Style::new());
    textarea.set_selection_style(theme.editor_selection());
    textarea.set_line_number_style(theme.line_number());
}

fn apply(editor: &mut EditorState, action: Action) {
    use Action as C;
    let t = &mut editor.textarea;
    match action {
        C::NewLine => t.insert_newline(),
        C::Indent => _ = t.insert_tab(),
        C::Left => move_cursor(editor, CursorMove::Back, false),
        C::Right => move_cursor(editor, CursorMove::Forward, false),
        C::Up => move_cursor(editor, CursorMove::Up, false),
        C::Down => move_cursor(editor, CursorMove::Down, false),
        C::WordLeft => move_cursor(editor, CursorMove::WordBack, false),
        C::WordRight => move_cursor(editor, CursorMove::WordForward, false),
        C::LineStart => move_cursor(editor, CursorMove::Head, false),
        C::LineEnd => move_cursor(editor, CursorMove::End, false),
        C::Top => move_cursor(editor, CursorMove::Top, false),
        C::Bottom => move_cursor(editor, CursorMove::Bottom, false),
        C::PageUp => t.scroll(Scrolling::PageUp),
        C::PageDown => t.scroll(Scrolling::PageDown),
        C::SelectLeft => move_cursor(editor, CursorMove::Back, true),
        C::SelectRight => move_cursor(editor, CursorMove::Forward, true),
        C::SelectUp => move_cursor(editor, CursorMove::Up, true),
        C::SelectDown => move_cursor(editor, CursorMove::Down, true),
        C::SelectWordLeft => move_cursor(editor, CursorMove::WordBack, true),
        C::SelectWordRight => move_cursor(editor, CursorMove::WordForward, true),
        C::SelectLineStart => move_cursor(editor, CursorMove::Head, true),
        C::SelectLineEnd => move_cursor(editor, CursorMove::End, true),
        C::SelectAll => t.select_all(),
        C::DeleteBack => _ = t.delete_char(),
        C::DeleteForward => _ = t.delete_next_char(),
        C::DeleteWordBack => _ = t.delete_word(),
        C::DeleteWordForward => _ = t.delete_next_word(),
        C::DeleteToLineEnd => _ = t.delete_line_by_end(),
        C::Copy => t.copy(),
        C::Cut => _ = t.cut(),
        C::Paste => _ = t.paste(),
        C::Undo => _ = t.undo(),
        C::Redo => _ = t.redo(),
        _ => {}
    }
}

/// Moves the cursor; with `select` the selection grows, without it any
/// selection is dropped, as in most editors.
fn move_cursor(editor: &mut EditorState, to: CursorMove, select: bool) {
    let t = &mut editor.textarea;
    if !select {
        t.cancel_selection();
    } else if !t.is_selecting() {
        t.start_selection();
    }
    t.move_cursor(to);
}

#[cfg(test)]
#[path = "editor_tests.rs"]
mod editor_tests;
