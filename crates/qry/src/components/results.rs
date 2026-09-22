use std::sync::Arc;

use qry_core::QueryResult;
use ratatui::{
    prelude::*,
    widgets::{Block, Cell, Paragraph, Row, Table},
};
use tokio::sync::mpsc::UnboundedSender;

use super::Component;
use crate::{action::Action, config::Config};



#[derive(Default)]
pub struct Results {
    command_tx: Option<UnboundedSender<Action>>,
    config: Config,
    focus: bool,

    result: Option<Arc<QueryResult>>,
}

impl Component for Results {
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
            Action::QueryDone(result) => {
                self.result = Some(result);
            }
            _ => {}
        }
        Ok(None)
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) -> color_eyre::Result<()> {
        let block = Block::bordered().title(" Results ").border_style(if self.focus { Style::new().green() } else { Style::new() });

        match &self.result {
            Some(result) if !result.columns.is_empty() => {
                let header = Row::new(result.columns.iter().map(String::as_str)).bold();
                let rows = result.rows.iter().map(|row| {
                    Row::new(row.iter().map(|cell| match cell {
                        Some(value) => Cell::from(value.as_str()),
                        None => Cell::from("NULL").dark_gray(),
                    }))
                });
                let widths = vec![Constraint::Fill(1); result.columns.len()];
                frame.render_widget(Table::new(rows, widths).header(header).block(block), area);
            }
            // A statement such as CREATE or INSERT returns no columns.
            Some(_) => frame.render_widget(
                Paragraph::new("Statement executed").centered().block(block),
                area,
            ),
            None => frame.render_widget(Paragraph::new("").block(block), area),
        }
        Ok(())
    }

    fn set_focus(&mut self, focus: bool) {
        self.focus = focus;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn render(results: &mut Results) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(40, 6)).unwrap();
        terminal
            .draw(|frame| results.draw(frame, frame.area()).unwrap())
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect())
            .collect()
    }

    #[test]
    fn query_done_renders_header_rows_and_null() {
        let mut results = Results::default();
        results
            .update(Action::QueryDone(Arc::new(QueryResult {
                columns: vec!["id".into(), "name".into()],
                rows: vec![
                    vec![Some("1".into()), Some("alice".into())],
                    vec![Some("2".into()), None],
                ],
                affected: Some(2),
            })))
            .unwrap();

        let lines = render(&mut results);
        assert!(lines[1].contains("id") && lines[1].contains("name"), "{lines:#?}");
        assert!(lines[2].contains('1') && lines[2].contains("alice"), "{lines:#?}");
        assert!(lines[3].contains('2') && lines[3].contains("NULL"), "{lines:#?}");
    }

    #[test]
    fn statement_without_columns_says_executed() {
        let mut results = Results::default();
        results
            .update(Action::QueryDone(Arc::new(QueryResult {
                columns: vec![],
                rows: vec![],
                affected: Some(0),
            })))
            .unwrap();

        assert!(render(&mut results).iter().any(|line| line.contains("Statement executed")));
    }
}
