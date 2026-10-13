use std::sync::Arc;

use crossterm::event::KeyEvent;
use qry_core::QueryResult;
use ratatui::{
    prelude::*,
    widgets::{
        Block, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table,
        TableState,
    },
};
use tokio::sync::mpsc::UnboundedSender;

use super::Component;
use crate::{action::Action, config::Config, keymap::ResultsCommand};

/// Wider values are cut off at this many columns.
const MAX_COLUMN_WIDTH: u16 = 40;
const COLUMN_SPACING: u16 = 1;

#[derive(Default)]
pub struct Results {
    command_tx: Option<UnboundedSender<Action>>,
    config: Config,
    focus: bool,

    result: Option<Arc<QueryResult>>,
    /// Display width of each column: its widest value, capped.
    widths: Vec<u16>,
    /// The selected cell.
    row: usize,
    col: usize,
    /// First row and column in view.
    row_offset: usize,
    col_offset: usize,
    /// Data rows that fit on screen, from the last draw. Used by PageUp/PageDown.
    page_rows: usize,
}

impl Results {
    fn show(&mut self, result: Arc<QueryResult>) {
        self.widths = column_widths(&result);
        self.result = Some(result);
        (self.row, self.col, self.row_offset, self.col_offset) = (0, 0, 0, 0);
    }
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

    fn handle_key_event(&mut self, key: KeyEvent) -> color_eyre::Result<Option<Action>> {
        let Some((rows, cols)) = self
            .result
            .as_ref()
            .map(|r| (r.rows.len(), r.columns.len()))
        else {
            return Ok(None);
        };
        let Some(command) = self.config.panes.results.get(key) else {
            return Ok(None);
        };
        let (last_row, last_col) = (rows.saturating_sub(1), cols.saturating_sub(1));
        let page = self.page_rows.max(1);
        match command {
            ResultsCommand::Up => self.row = self.row.saturating_sub(1),
            ResultsCommand::Down => self.row = (self.row + 1).min(last_row),
            ResultsCommand::Left => self.col = self.col.saturating_sub(1),
            ResultsCommand::Right => self.col = (self.col + 1).min(last_col),
            ResultsCommand::PageUp => self.row = self.row.saturating_sub(page),
            ResultsCommand::PageDown => self.row = (self.row + page).min(last_row),
            ResultsCommand::FirstColumn => self.col = 0,
            ResultsCommand::LastColumn => self.col = last_col,
            ResultsCommand::FirstRow => self.row = 0,
            ResultsCommand::LastRow => self.row = last_row,
        }
        Ok(None)
    }

    fn update(&mut self, action: Action) -> color_eyre::Result<Option<Action>> {
        match action {
            Action::Tick => {}
            Action::Render => {}
            Action::QueryDone(result) => self.show(result),
            _ => {}
        }
        Ok(None)
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) -> color_eyre::Result<()> {
        let block = Block::bordered()
            .title(" Results ")
            .border_style(if self.focus {
                Style::new().green()
            } else {
                Style::new()
            });

        let Some(result) = self.result.clone() else {
            frame.render_widget(block, area);
            return Ok(());
        };
        // A statement such as CREATE or INSERT returns no columns.
        if result.columns.is_empty() {
            frame.render_widget(
                Paragraph::new("Statement executed").centered().block(block),
                area,
            );
            return Ok(());
        }

        let inner = block.inner(area);
        let total_rows = result.rows.len();
        // One line of `inner` is taken by the header.
        self.page_rows = usize::from(inner.height.saturating_sub(1)).max(1);
        self.row_offset = scroll_rows(self.row_offset, self.row, self.page_rows, total_rows);
        self.col_offset = scroll_columns(&self.widths, self.col_offset, self.col, inner.width);
        let visible = visible_columns(&self.widths, self.col_offset, inner.width);

        let header = Row::new(visible.iter().map(|&(c, _)| {
            let name = Cell::from(result.columns[c].as_str());
            if c == self.col {
                name.underlined()
            } else {
                name
            }
        }))
        .bold();
        let end = (self.row_offset + self.page_rows).min(total_rows);
        let rows =
            result.rows[self.row_offset..end].iter().map(|row| {
                Row::new(visible.iter().map(
                    |&(c, _)| match row.get(c).and_then(Option::as_deref) {
                        Some(value) => Cell::from(value),
                        None => Cell::from("NULL").dark_gray(),
                    },
                ))
            });
        let table = Table::new(rows, visible.iter().map(|&(_, w)| Constraint::Length(w)))
            .header(header)
            .column_spacing(COLUMN_SPACING)
            .cell_highlight_style(if self.focus {
                Style::new().black().on_green()
            } else {
                Style::new().reversed()
            });

        let mut state = TableState::default();
        if total_rows > 0 {
            state.select_cell(Some((
                self.row - self.row_offset,
                self.col - self.col_offset,
            )));
        }

        let position = if total_rows == 0 {
            " 0 rows ".to_string()
        } else {
            format!(
                " row {}/{total_rows} · col {}/{} ",
                self.row + 1,
                self.col + 1,
                result.columns.len()
            )
        };
        frame.render_widget(block.title(Line::from(position).right_aligned()), area);
        frame.render_stateful_widget(table, inner, &mut state);

        // Scrollbars sit on the border, and only appear when there is more to see.
        if total_rows > self.page_rows {
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(None)
                    .end_symbol(None),
                area.inner(Margin::new(0, 1)),
                &mut ScrollbarState::new(total_rows)
                    .viewport_content_length(self.page_rows)
                    .position(self.row),
            );
        }
        if total_width(&self.widths) > usize::from(inner.width) {
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::HorizontalBottom)
                    .begin_symbol(None)
                    .end_symbol(None),
                area.inner(Margin::new(1, 0)),
                &mut ScrollbarState::new(self.widths.len())
                    .viewport_content_length(visible.len())
                    .position(self.col),
            );
        }
        Ok(())
    }

    fn set_focus(&mut self, focus: bool) {
        self.focus = focus;
    }
}

fn column_widths(result: &QueryResult) -> Vec<u16> {
    let width = |s: &str| Span::raw(s).width();
    result
        .columns
        .iter()
        .enumerate()
        .map(|(c, name)| {
            let widest = result
                .rows
                .iter()
                .map(|row| {
                    row.get(c)
                        .and_then(Option::as_deref)
                        .map_or(width("NULL"), width)
                })
                .fold(width(name), usize::max);
            u16::try_from(widest)
                .unwrap_or(u16::MAX)
                .clamp(1, MAX_COLUMN_WIDTH)
        })
        .collect()
}

/// Width of all columns side by side, including the gaps between them.
fn total_width(widths: &[u16]) -> usize {
    widths
        .iter()
        .map(|&w| usize::from(w + COLUMN_SPACING))
        .sum::<usize>()
        - usize::from(COLUMN_SPACING)
}

/// Moves the first visible row as little as possible to keep `selected` in
/// view, without leaving empty space below the last row.
fn scroll_rows(offset: usize, selected: usize, page: usize, total: usize) -> usize {
    let offset = if selected < offset {
        selected
    } else if selected >= offset + page {
        selected + 1 - page
    } else {
        offset
    };
    offset.min(total.saturating_sub(page))
}

/// Moves the first visible column as little as possible so the selected
/// column fits entirely within `available`.
fn scroll_columns(widths: &[u16], offset: usize, selected: usize, available: u16) -> usize {
    let mut offset = offset.min(selected);
    while offset < selected && total_width(&widths[offset..=selected]) > usize::from(available) {
        offset += 1;
    }
    offset
}

/// The columns that fit from `offset` on, as (index, width). The last one may
/// be cut short to fill the remaining space.
fn visible_columns(widths: &[u16], offset: usize, available: u16) -> Vec<(usize, u16)> {
    let mut visible = Vec::new();
    let mut used: u16 = 0;
    for (c, &width) in widths.iter().enumerate().skip(offset) {
        let gap = if visible.is_empty() {
            0
        } else {
            COLUMN_SPACING
        };
        let room = available.saturating_sub(used + gap);
        if room == 0 {
            break;
        }
        let width = width.min(room);
        visible.push((c, width));
        used += gap + width;
    }
    visible
}

#[cfg(test)]
#[path = "results_tests.rs"]
mod results_tests;
