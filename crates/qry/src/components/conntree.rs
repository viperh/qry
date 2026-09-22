use ratatui::{
    prelude::*,
    widgets::{Block, Paragraph},
};
use tokio::sync::mpsc::UnboundedSender;

use super::Component;
use crate::{action::Action, config::Config};

#[derive(Default)]
pub struct ConnTree {
    command_tx: Option<UnboundedSender<Action>>,
    config: Config,
    focus: bool
}

impl Component for ConnTree {
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
            _ => {}
        }
        Ok(None)
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) -> color_eyre::Result<()> {
        frame.render_widget(
            Paragraph::new("No connections yet")
                .centered()
                .block(Block::bordered().title(" Connection Tree ").border_style(if self.focus { Style::new().yellow() } else { Style::new() })),
            area,
        );
        Ok(())
    }

    fn set_focus(&mut self, focus: bool) {
        self.focus = focus;
    }
}
