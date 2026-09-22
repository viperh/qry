use ratatui::{
    prelude::*,
    widgets::{Block, Paragraph},
};
use tokio::sync::mpsc::UnboundedSender;

use super::Component;
use crate::{action::Action, config::Config};





#[derive(Default)]
pub struct Infopanel {
    command_tx: Option<UnboundedSender<Action>>,
    config: Config,
    info: String

}

impl Infopanel {
    pub fn is_empty(&self) -> bool {
        self.info.is_empty()
    }

}

impl Component for Infopanel {
    fn register_action_handler(&mut self, tx: UnboundedSender<Action>) -> color_eyre::Result<()> {
        self.command_tx = Some(tx);
        Ok(())
    }

    fn register_config_handler(&mut self, config: Config) -> color_eyre::Result<()> {
        self.config = config;
        Ok(())
    }

    fn update(&mut self, action: Action) -> color_eyre::Result<Option<Action>> {
        match action {
            Action::Tick => {}
            Action::Render => {}
            Action::Info(s) => {
                self.info = s;
            }
            _ => {}
        }
        Ok(None)
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) -> color_eyre::Result<()> {
        frame.render_widget(
            Paragraph::new(self.info.as_str())
                .centered()
                .block(Block::bordered()),
            area,
        );
        Ok(())
    }

    fn set_focus(&mut self, focus: bool) {
        let _ = focus;
    }
}
