use crossterm::event::KeyEvent;
use ratatui::{
    prelude::*,
    widgets::Block,
};
use ratatui_textarea::{CursorMove, Scrolling, TextArea};
use tokio::sync::mpsc::UnboundedSender;

use super::Component;
use super::textinput::typed_char;
use crate::{action::Action, config::Config, keymap::EditorCommand};




#[derive(Default)]
pub struct Editor {
    command_tx: Option<UnboundedSender<Action>>,
    config: Config,

    focus: bool,

    textarea: TextArea<'static>,

}


impl Editor {
    pub fn query(&self) -> String {
        self.textarea.lines().join("\n")
    }

    fn restyle(&mut self) {
        let border = if self.focus {
            Style::new().cyan()
        } else {
            Style::new().dark_gray()
        };
        self.textarea
            .set_block(Block::bordered().border_style(border).title("Query"));
        self.textarea.set_cursor_style(if self.focus {
            Style::new().reversed()
        } else {
            Style::new()
        });
        self.textarea.set_cursor_line_style(Style::new());
        self.textarea.set_line_number_style(Style::new().dark_gray());
    }

    fn apply(&mut self, command: EditorCommand) {
        use EditorCommand as C;
        let t = &mut self.textarea;
        match command {
            C::RunQuery => {}
            C::NewLine => t.insert_newline(),
            C::Left => self.move_cursor(CursorMove::Back, false),
            C::Right => self.move_cursor(CursorMove::Forward, false),
            C::Up => self.move_cursor(CursorMove::Up, false),
            C::Down => self.move_cursor(CursorMove::Down, false),
            C::WordLeft => self.move_cursor(CursorMove::WordBack, false),
            C::WordRight => self.move_cursor(CursorMove::WordForward, false),
            C::LineStart => self.move_cursor(CursorMove::Head, false),
            C::LineEnd => self.move_cursor(CursorMove::End, false),
            C::Top => self.move_cursor(CursorMove::Top, false),
            C::Bottom => self.move_cursor(CursorMove::Bottom, false),
            C::PageUp => t.scroll(Scrolling::PageUp),
            C::PageDown => t.scroll(Scrolling::PageDown),
            C::SelectLeft => self.move_cursor(CursorMove::Back, true),
            C::SelectRight => self.move_cursor(CursorMove::Forward, true),
            C::SelectUp => self.move_cursor(CursorMove::Up, true),
            C::SelectDown => self.move_cursor(CursorMove::Down, true),
            C::SelectWordLeft => self.move_cursor(CursorMove::WordBack, true),
            C::SelectWordRight => self.move_cursor(CursorMove::WordForward, true),
            C::SelectLineStart => self.move_cursor(CursorMove::Head, true),
            C::SelectLineEnd => self.move_cursor(CursorMove::End, true),
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
        }
    }

    /// Moves the cursor; with `select` the selection grows, without it any
    /// selection is dropped, as in most editors.
    fn move_cursor(&mut self, to: CursorMove, select: bool) {
        if !select {
            self.textarea.cancel_selection();
        } else if !self.textarea.is_selecting() {
            self.textarea.start_selection();
        }
        self.textarea.move_cursor(to);
    }
}

impl Component for Editor {
    fn register_action_handler(&mut self, tx: UnboundedSender<Action>) -> color_eyre::Result<()> {
        self.command_tx = Some(tx);
        Ok(())
    }
    fn register_config_handler(&mut self, config: Config) -> color_eyre::Result<()> {
        self.config = config;
        self.restyle();
        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> color_eyre::Result<Option<Action>> {
        match self.config.panes.editor.get(key) {
            Some(EditorCommand::RunQuery) => return Ok(Some(Action::Execute(self.query()))),
            Some(command) => self.apply(command),
            None => {
                if let Some(c) = typed_char(key) {
                    self.textarea.insert_char(c);
                }
            }
        }
        Ok(None)
    }
    fn update(&mut self, action: Action) -> color_eyre::Result<Option<Action>> {
        match action {
            Action::Tick => {}
            Action::Render => {}

            _ => {}
        }
        Ok(None)
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) -> color_eyre::Result<()> {
        frame.render_widget(&self.textarea, area);
        Ok(())
    }

    fn set_focus(&mut self, focus: bool) {
        self.focus = focus;
        self.restyle()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    fn editor() -> Editor {
        let mut editor = Editor::default();
        editor.register_config_handler(Config::embedded()).unwrap();
        editor
    }

    fn press(editor: &mut Editor, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
        editor.handle_key_event(KeyEvent::new(code, modifiers)).unwrap()
    }

    fn type_str(editor: &mut Editor, text: &str) {
        for c in text.chars() {
            press(editor, KeyCode::Char(c), KeyModifiers::NONE);
        }
    }

    #[test]
    fn typing_and_the_run_key() {
        let mut editor = editor();
        type_str(&mut editor, "select 1");
        press(&mut editor, KeyCode::Enter, KeyModifiers::NONE);
        type_str(&mut editor, "q");
        assert_eq!(
            press(&mut editor, KeyCode::F(8), KeyModifiers::NONE),
            Some(Action::Execute("select 1\nq".into()))
        );
    }

    #[test]
    fn unbound_shortcuts_type_nothing() {
        let mut editor = editor();
        type_str(&mut editor, "ab");
        press(&mut editor, KeyCode::Char('b'), KeyModifiers::CONTROL);
        press(&mut editor, KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(editor.query(), "ab");
    }

    #[test]
    fn undo_redo_and_delete_word() {
        let mut editor = editor();
        type_str(&mut editor, "select name");
        press(&mut editor, KeyCode::Char('w'), KeyModifiers::CONTROL);
        assert_eq!(editor.query(), "select ");
        press(&mut editor, KeyCode::Char('u'), KeyModifiers::CONTROL);
        assert_eq!(editor.query(), "select name");
        press(&mut editor, KeyCode::Char('r'), KeyModifiers::CONTROL);
        assert_eq!(editor.query(), "select ");
    }

    #[test]
    fn shift_selects_and_cut_paste_moves_text() {
        let mut editor = editor();
        type_str(&mut editor, "abcd");
        press(&mut editor, KeyCode::Left, KeyModifiers::SHIFT);
        press(&mut editor, KeyCode::Left, KeyModifiers::SHIFT);
        press(&mut editor, KeyCode::Char('x'), KeyModifiers::CONTROL);
        assert_eq!(editor.query(), "ab");

        press(&mut editor, KeyCode::Home, KeyModifiers::NONE);
        press(&mut editor, KeyCode::Char('y'), KeyModifiers::CONTROL);
        assert_eq!(editor.query(), "cdab");
    }

    #[test]
    fn plain_movement_drops_the_selection() {
        let mut editor = editor();
        type_str(&mut editor, "abcd");
        press(&mut editor, KeyCode::Left, KeyModifiers::SHIFT);
        press(&mut editor, KeyCode::Left, KeyModifiers::NONE);
        // Nothing is selected any more, so typing inserts instead of replacing.
        type_str(&mut editor, "X");
        assert_eq!(editor.query(), "abXcd");
    }
}
