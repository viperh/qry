//! The query editor's keys.

use ratatui_textarea::CursorMove;

use super::run;
use crate::{
    action::Action,
    app::{
        effects::Effect,
        state::{EditorState, State},
    },
};

/// What an `Editor` binding does.
pub(super) fn run(state: &mut State, action: Action, fx: &mut Vec<Effect>) {
    match action {
        Action::RunStatement => run::statement(state, fx),
        Action::RunAll => run::all(state, fx),
        Action::RunSelection => run::selection(state, fx),
        // No query can be cancelled until there are jobs (T305), so it
        // always copies.
        Action::CancelQuery => {
            if !run::cancel(state, fx) {
                apply(state.editor_mut(), Action::Copy);
            }
        }
        action => apply(state.editor_mut(), action),
    }
}

fn apply(editor: &mut EditorState, action: Action) {
    use Action as C;
    let t = &mut editor.textarea;
    match action {
        // The new line starts as indented as the one it breaks.
        C::NewLine => {
            let row = t.cursor().0;
            let indent: String = t.lines()[row]
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect();
            t.insert_newline();
            t.insert_str(indent);
        }
        C::Indent => _ = t.insert_tab(),
        C::Dedent => dedent(editor),
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
        C::PageUp | C::PageDown => {
            let to = if action == C::PageUp {
                CursorMove::Up
            } else {
                CursorMove::Down
            };
            let page = editor.page.get().max(1);
            editor.textarea.cancel_selection();
            for _ in 0..page {
                editor.textarea.move_cursor(to);
            }
        }
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

/// Takes up to one indent (4 spaces, or a tab) off the start of the line.
fn dedent(editor: &mut EditorState) {
    let t = &mut editor.textarea;
    let cursor = t.cursor();
    let (row, col) = (cursor.0, cursor.1);
    let line = &t.lines()[row];
    let n = if line.starts_with('\t') {
        1
    } else {
        line.chars().take(4).take_while(|c| *c == ' ').count()
    };
    if n == 0 {
        return;
    }
    t.cancel_selection();
    t.move_cursor(CursorMove::Jump(u16::try_from(row).unwrap_or(u16::MAX), 0));
    for _ in 0..n {
        t.delete_next_char();
    }
    t.move_cursor(CursorMove::Jump(
        u16::try_from(row).unwrap_or(u16::MAX),
        u16::try_from(col.saturating_sub(n)).unwrap_or(u16::MAX),
    ));
}

/// Pasted text, with Windows line ends made plain.
pub(super) fn paste(editor: &mut EditorState, text: &str) {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    editor.textarea.insert_str(text);
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
