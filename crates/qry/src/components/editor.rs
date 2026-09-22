use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    prelude::*,
    widgets::Block,
};
use ratatui_textarea::TextArea;
use tokio::sync::mpsc::UnboundedSender;

use super::Component;
use crate::{action::Action, config::Config};




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
        if key.code == KeyCode::F(5) {
            return Ok(Some(Action::Execute(self.query())));
        }
        self.textarea.input(key);
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
