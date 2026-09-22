use ratatui::{
    prelude::*,
    widgets::Paragraph,
};
use tokio::sync::mpsc::UnboundedSender;

use super::Component;
use crate::{action::Action, config::Config};
use crate::action::StatusCode;

#[derive(Default)]
pub struct Statuspanel {
    command_tx: Option<UnboundedSender<Action>>,
    config: Config,
    statuscode: StatusCode,
}

impl Statuspanel {
    pub fn is_empty(&self) -> bool {
        self.statuscode == StatusCode::None
    }
}

impl Component for Statuspanel {
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
            Action::Status(s) => {
                self.statuscode = s;
            }
            _ => {}
        }
        Ok(None)
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) -> color_eyre::Result<()> {
        match self.statuscode {
            StatusCode::Error(ref err) => {
                frame.render_widget(
                    Paragraph::new(format!("Error: {}", err))
                        .red()
                        .centered()
                , area)
            }
            StatusCode::Success(ref msg) => {
                frame.render_widget(
                    Paragraph::new(msg.as_str())
                        .green()
                        .centered()
                    , area)
            }
            _ => {}
        }

        Ok(())
    }

    fn set_focus(&mut self, focus: bool) {
        let _ = focus;
    }
}
