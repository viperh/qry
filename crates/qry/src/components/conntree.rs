use ratatui::{
    prelude::*,
    widgets::{Block, Paragraph},
};
use tokio::sync::mpsc::UnboundedSender;

use super::Component;
use crate::{
    action::{Action, StatusCode},
    config::Config,
    connections::{Secret, StoredConnection},
    import::Imported,
    keymap::TreeCommand,
};

/// A connection the tree knows about, and what it has learned of it.
struct Node {
    record: StoredConnection,
    /// What the database called itself; `None` until it has connected once.
    label: Option<String>,
    expanded: bool,
    tables: Option<Vec<String>>,
    loading: bool,
    /// Added this session and not written to the file yet.
    unsaved: bool,
}

impl Node {
    fn title(&self) -> &str {
        self.record.title()
    }
}

/// One drawn line of the tree.
enum Row {
    Connection(usize),
    Table(usize, usize),
    /// A word under a connection, such as while its tables load.
    Note(&'static str),
}

#[derive(Default)]
pub struct ConnTree {
    command_tx: Option<UnboundedSender<Action>>,
    config: Config,
    focus: bool,

    connections: Vec<Node>,
    /// The connection the worker is on.
    active: Option<usize>,
    /// The connection whose `Connect` was sent and has not been answered.
    attempting: Option<usize>,
    /// Expand this one as soon as it is connected.
    expand_when_connected: Option<usize>,
    selected: usize,
    offset: usize,
}

impl ConnTree {
    fn send(&self, action: Action) -> color_eyre::Result<()> {
        if let Some(tx) = &self.command_tx {
            tx.send(action)?;
        }
        Ok(())
    }

    fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for (i, connection) in self.connections.iter().enumerate() {
            rows.push(Row::Connection(i));
            if !connection.expanded {
                continue;
            }
            match (&connection.tables, connection.loading) {
                (_, true) => rows.push(Row::Note("loading…")),
                (Some(tables), _) if tables.is_empty() => rows.push(Row::Note("(no tables)")),
                (Some(tables), _) => rows.extend((0..tables.len()).map(|t| Row::Table(i, t))),
                (None, _) => rows.push(Row::Note("(not connected)")),
            }
        }
        rows
    }

    /// Remembers a connection the app is about to open, adding it to the
    /// tree if its id is new and taking on its fields if it was edited. The
    /// entry stays once it has connected.
    fn attempt(&mut self, record: &StoredConnection) -> color_eyre::Result<()> {
        let known = self
            .connections
            .iter()
            .position(|n| n.record.id == record.id);
        let Some(i) = known else {
            self.connections.push(Node {
                record: record.clone(),
                label: None,
                expanded: false,
                tables: None,
                loading: false,
                unsaved: !record.ephemeral,
            });
            self.attempting = Some(self.connections.len() - 1);
            return Ok(());
        };

        self.attempting = Some(i);
        let previous = std::mem::replace(&mut self.connections[i].record, record.clone());
        if previous == *record {
            return Ok(());
        }
        // Edited: worth writing once it opens, and its tables may differ.
        self.connections[i].unsaved = !record.ephemeral;
        self.connections[i].tables = None;
        if matches!(previous.secret, Secret::Keyring) && !matches!(record.secret, Secret::Keyring) {
            // Its password is no longer kept, so it should not linger.
            self.send(Action::ForgetSecret(previous.id))?;
        }
        Ok(())
    }

    /// Adds what an import found that the tree does not have, saves once,
    /// and says in the status bar what happened to the rest.
    fn import(&mut self, imported: Imported) -> color_eyre::Result<()> {
        let found = imported.connections.len();
        let mut added = 0;
        for record in imported.connections {
            let known = self
                .connections
                .iter()
                .any(|n| n.record.id == record.id || n.record.same_target(&record));
            if known {
                continue;
            }
            self.connections.push(Node {
                record,
                label: None,
                expanded: false,
                tables: None,
                loading: false,
                unsaved: false,
            });
            added += 1;
        }
        if added > 0 {
            self.save()?;
        }

        let mut parts = vec![format!(
            "Imported {added} connection{} from {}",
            if added == 1 { "" } else { "s" },
            imported.format
        )];
        if found > added {
            parts.push(format!("{} already here", found - added));
        }
        for skipped in &imported.skipped {
            parts.push(format!("skipped {}: {}", skipped.name, skipped.reason));
        }
        parts.extend(imported.warnings);
        let message = parts.join("; ");
        let status = if imported.skipped.is_empty() {
            StatusCode::Success(message)
        } else {
            StatusCode::Error(message)
        };
        self.send(Action::Status(status))
    }

    /// The records worth writing: everything but the scratch database.
    fn to_save(&self) -> Vec<StoredConnection> {
        self.connections
            .iter()
            .filter(|node| !node.record.ephemeral)
            .map(|node| node.record.clone())
            .collect()
    }

    fn save(&self) -> color_eyre::Result<()> {
        self.send(Action::SaveConnections(self.to_save()))
    }

    /// Asks the worker for the tables of whatever is connected.
    fn load_tables(&mut self, connection: usize) -> color_eyre::Result<()> {
        self.connections[connection].loading = true;
        self.send(Action::ListTables)
    }

    fn command(&mut self, command: TreeCommand) -> color_eyre::Result<()> {
        let rows = self.rows();
        if rows.is_empty() {
            return Ok(());
        }
        match command {
            TreeCommand::Up => self.selected = self.selected.saturating_sub(1),
            TreeCommand::Down => self.selected = (self.selected + 1).min(rows.len() - 1),
            TreeCommand::First => self.selected = 0,
            TreeCommand::Last => self.selected = rows.len() - 1,
            TreeCommand::Toggle => {
                if let Some(Row::Connection(i)) = rows.get(self.selected) {
                    let i = *i;
                    self.connections[i].expanded = !self.connections[i].expanded;
                    let needs_tables = self.connections[i].expanded
                        && self.connections[i].tables.is_none()
                        && !self.connections[i].loading;
                    if needs_tables {
                        if self.active == Some(i) {
                            self.load_tables(i)?;
                        } else {
                            // Its tables can only be read once it is open.
                            self.expand_when_connected = Some(i);
                            self.send(Action::Connect(Box::new(
                                self.connections[i].record.clone(),
                            )))?;
                        }
                    }
                }
            }
            TreeCommand::Connect => {
                if let Some(Row::Connection(i)) = rows.get(self.selected) {
                    self.send(Action::Connect(Box::new(
                        self.connections[*i].record.clone(),
                    )))?;
                }
            }
            TreeCommand::Edit => {
                if let Some(Row::Connection(i)) = rows.get(self.selected) {
                    self.send(Action::EditConnection(Box::new(
                        self.connections[*i].record.clone(),
                    )))?;
                }
            }
            TreeCommand::Delete => {
                if let Some(Row::Connection(i)) = rows.get(self.selected) {
                    let node = &self.connections[*i];
                    // Its password goes with it, so nothing is orphaned.
                    let forget = matches!(node.record.secret, Secret::Keyring)
                        .then(|| Action::ForgetSecret(node.record.id.clone()));
                    let saved = !node.record.ephemeral;
                    self.forget(*i);
                    if let Some(action) = forget {
                        self.send(action)?;
                    }
                    if saved {
                        self.save()?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Drops a connection that never opened, keeping the other indexes right.
    fn forget(&mut self, connection: usize) {
        self.connections.remove(connection);
        let shift = |index: &mut Option<usize>| {
            if let Some(i) = index
                && *i > connection
            {
                *index = Some(*i - 1);
            }
        };
        shift(&mut self.active);
        shift(&mut self.expand_when_connected);
        self.selected = self.selected.min(self.rows().len().saturating_sub(1));
    }

    fn line(&self, row: &Row) -> Line<'static> {
        match row {
            Row::Connection(i) => {
                let connection = &self.connections[*i];
                let marker = if connection.expanded { "▾" } else { "▸" };
                let line = Line::from(format!("{marker} {}", connection.title()));
                // The one the worker is on stands out without taking the
                // colour, which belongs to the selection.
                if self.active == Some(*i) {
                    line.bold()
                } else {
                    line
                }
            }
            Row::Table(i, t) => {
                let table = &self.connections[*i].tables.as_ref().expect("expanded")[*t];
                Line::from(format!("    {table}"))
            }
            Row::Note(note) => Line::from(format!("    {note}")),
        }
    }
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

    fn handle_key_event(
        &mut self,
        key: crossterm::event::KeyEvent,
    ) -> color_eyre::Result<Option<Action>> {
        if let Some(command) = self.config.panes.tree.get(key) {
            self.command(command)?;
        }
        Ok(None)
    }

    fn update(&mut self, action: Action) -> color_eyre::Result<Option<Action>> {
        match action {
            // Every connection the app opens passes through here, whether it
            // came from the New Connection modal or from this tree.
            Action::Connect(ref record) => self.attempt(record)?,
            Action::Connected(label) => {
                if let Some(i) = self.attempting.take() {
                    self.connections[i].label = Some(label);
                    self.active = Some(i);
                    // Its tables are stale, or were never read.
                    self.connections[i].tables = None;
                    if self.expand_when_connected.take() == Some(i) {
                        self.connections[i].expanded = true;
                    }
                    if self.connections[i].expanded {
                        self.load_tables(i)?;
                    }
                    // It works, so it is worth keeping.
                    if self.connections[i].unsaved {
                        self.connections[i].unsaved = false;
                        self.save()?;
                    }
                }
            }
            Action::ConnectFailed(_) => {
                if let Some(i) = self.attempting.take()
                    // Only one added this session and never written: a saved
                    // connection stays, whatever the database says today.
                    && self.connections[i].unsaved
                {
                    self.forget(i);
                }
                self.expand_when_connected = None;
            }
            Action::ConnectionsLoaded(records) => {
                // Whatever is already here was opened during startup; the
                // file fills in the rest.
                for record in records {
                    if !self.connections.iter().any(|n| n.record.id == record.id) {
                        self.connections.push(Node {
                            record,
                            label: None,
                            expanded: false,
                            tables: None,
                            loading: false,
                            unsaved: false,
                        });
                    }
                }
            }
            Action::Imported(imported) => self.import(*imported)?,
            // The same list a save writes: everything but the scratch database.
            Action::ExportConnections(path) => {
                let connections = self.to_save();
                if connections.is_empty() {
                    self.send(Action::ConnectionsExportFailed(
                        "There are no saved connections to export".into(),
                    ))?;
                } else {
                    self.send(Action::WriteConnections { path, connections })?;
                }
            }
            Action::SecretNotStored(id) => {
                if let Some(node) = self.connections.iter_mut().find(|n| n.record.id == id) {
                    // The keychain would not take it, so the record says so
                    // rather than pretending the password is there.
                    node.record.secret = Secret::Prompt;
                    self.save()?;
                }
            }
            Action::TablesLoaded(tables) => {
                if let Some(i) = self.active {
                    self.connections[i].tables = Some(tables);
                    self.connections[i].loading = false;
                }
            }
            Action::TablesFailed(_) => {
                if let Some(i) = self.active {
                    // Left unread, so Space can try again; the status bar says why.
                    self.connections[i].loading = false;
                    self.connections[i].expanded = false;
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) -> color_eyre::Result<()> {
        let block = Block::bordered()
            .title(" Connection Tree ")
            .border_style(if self.focus {
                Style::new().yellow()
            } else {
                Style::new()
            });

        let rows = self.rows();
        if rows.is_empty() {
            frame.render_widget(
                Paragraph::new("No connections yet").centered().block(block),
                area,
            );
            return Ok(());
        }

        let inner = block.inner(area);
        let page = usize::from(inner.height).max(1);
        self.selected = self.selected.min(rows.len() - 1);
        // Scroll only as far as it takes to keep the selected row in view.
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + page {
            self.offset = self.selected + 1 - page;
        }
        self.offset = self.offset.min(rows.len().saturating_sub(page));

        let lines: Vec<Line> = rows
            .iter()
            .enumerate()
            .skip(self.offset)
            .take(page)
            .map(|(i, row)| {
                // The colour carries the selection: green for it, grey for
                // the rest, and no background behind either.
                let line = self.line(row);
                if i == self.selected {
                    line.green()
                } else {
                    line.gray()
                }
            })
            .collect();

        frame.render_widget(Paragraph::new(lines).block(block), area);
        Ok(())
    }

    fn set_focus(&mut self, focus: bool) {
        self.focus = focus;
    }
}

#[cfg(test)]
#[path = "conntree_tests.rs"]
mod conntree_tests;
