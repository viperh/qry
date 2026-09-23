use ratatui::{
    prelude::*,
    widgets::{Block, Paragraph},
};
use tokio::sync::mpsc::UnboundedSender;

use super::Component;
use crate::connections::{Secret, StoredConnection};
use crate::keymap::TreeCommand;
use crate::{action::Action, config::Config};

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
    /// tree if its id is new. The entry stays once it has connected.
    fn attempt(&mut self, record: &StoredConnection) {
        let known = self.connections.iter().position(|n| n.record.id == record.id);
        self.attempting = Some(known.unwrap_or_else(|| {
            self.connections.push(Node {
                record: record.clone(),
                label: None,
                expanded: false,
                tables: None,
                loading: false,
                unsaved: !record.ephemeral,
            });
            self.connections.len() - 1
        }));
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
                            self.send(Action::Connect(Box::new(self.connections[i].record.clone())))?;
                        }
                    }
                }
            }
            TreeCommand::Connect => {
                if let Some(Row::Connection(i)) = rows.get(self.selected) {
                    self.send(Action::Connect(Box::new(self.connections[*i].record.clone())))?;
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
                if self.active == Some(*i) { line.bold() } else { line }
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

    fn handle_key_event(&mut self, key: crossterm::event::KeyEvent) -> color_eyre::Result<Option<Action>> {
        if let Some(command) = self.config.panes.tree.get(key) {
            self.command(command)?;
        }
        Ok(None)
    }

    fn update(&mut self, action: Action) -> color_eyre::Result<Option<Action>> {
        match action {
            // Every connection the app opens passes through here, whether it
            // came from the New Connection modal or from this tree.
            Action::Connect(ref record) => self.attempt(record),
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
                    && self.connections[i].label.is_none()
                {
                    // It never opened, so it does not belong in the tree.
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
        let block = Block::bordered().title(" Connection Tree ").border_style(if self.focus { Style::new().yellow() } else { Style::new() });

        let rows = self.rows();
        if rows.is_empty() {
            frame.render_widget(Paragraph::new("No connections yet").centered().block(block), area);
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
                if i == self.selected { line.green() } else { line.gray() }
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
mod tests {
    use super::*;
    use crate::components::form::tests::key;
    use crossterm::event::KeyCode;
    use ratatui::{Terminal, backend::TestBackend};
    use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

    /// A saved SQLite connection, as the form would build it.
    fn record(name: &str, path: &str) -> StoredConnection {
        let mut record = StoredConnection::new(name, crate::connections::Driver::Sqlite);
        record.path = Some(path.to_string());
        record.secret = Secret::None;
        record
    }

    fn tree() -> (ConnTree, UnboundedReceiver<Action>) {
        let (tx, rx) = unbounded_channel();
        let mut tree = ConnTree::default();
        tree.register_action_handler(tx).unwrap();
        tree.register_config_handler(Config::embedded()).unwrap();
        (tree, rx)
    }

    /// Adds a connection the way the app does: the action, then the answer.
    #[allow(clippy::let_underscore_untyped)]
    fn connect(tree: &mut ConnTree, name: &str, path: &str) -> StoredConnection {
        let record = record(name, path);
        tree.update(Action::Connect(Box::new(record.clone()))).unwrap();
        tree.update(Action::Connected(path.into())).unwrap();
        record
    }

    fn press(tree: &mut ConnTree, code: KeyCode) {
        tree.handle_key_event(key(code)).unwrap();
    }

    fn lines(tree: &mut ConnTree, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| tree.draw(frame, frame.area()).unwrap()).unwrap();
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect())
            .collect()
    }

    #[test]
    fn a_connection_joins_the_tree_once_it_opens() {
        let (mut tree, _rx) = tree();
        assert!(lines(&mut tree, 30, 8).iter().any(|l| l.contains("No connections yet")));

        connect(&mut tree, "prod", "app.db");
        let shown = lines(&mut tree, 30, 8).join("\n");
        assert!(shown.contains("▸ prod"), "{shown}");
        assert_eq!(tree.active, Some(0));

        // The same connection again is not added twice: the id is what
        // counts, not the name.
        let again = tree.connections[0].record.clone();
        tree.update(Action::Connect(Box::new(again))).unwrap();
        tree.update(Action::Connected("app.db".into())).unwrap();
        assert_eq!(tree.connections.len(), 1);
    }

    #[test]
    fn a_connection_that_never_opens_is_not_kept() {
        let (mut tree, _rx) = tree();
        connect(&mut tree, "prod", "app.db");

        tree.update(Action::Connect(Box::new(record("typo", "nope.db")))).unwrap();
        assert_eq!(tree.connections.len(), 2, "added while it is being tried");
        tree.update(Action::ConnectFailed("no such file".into())).unwrap();

        assert_eq!(tree.connections.len(), 1);
        assert_eq!(tree.active, Some(0), "the working one is still active");
        assert!(!lines(&mut tree, 30, 8).join("\n").contains("typo"));
    }

    #[test]
    fn the_saved_connections_join_the_tree_at_startup() {
        let (mut tree, _rx) = tree();
        connect(&mut tree, "scratch", ":memory:");
        let saved = record("prod", "app.db");
        tree.update(Action::ConnectionsLoaded(vec![saved.clone()])).unwrap();

        let shown = lines(&mut tree, 30, 8).join("\n");
        assert!(shown.contains("scratch") && shown.contains("prod"), "{shown}");

        // Loading again does not double anything up.
        tree.update(Action::ConnectionsLoaded(vec![saved])).unwrap();
        assert_eq!(tree.connections.len(), 2);
    }

    #[test]
    fn a_connection_that_opens_is_saved_but_the_scratch_one_is_not() {
        let (mut tree, mut rx) = tree();

        let mut scratch = record("scratch", ":memory:");
        scratch.ephemeral = true;
        tree.update(Action::Connect(Box::new(scratch))).unwrap();
        tree.update(Action::Connected(":memory:".into())).unwrap();
        assert!(rx.try_recv().is_err(), "the scratch database is never written");

        connect(&mut tree, "prod", "app.db");
        let Ok(Action::SaveConnections(saved)) = rx.try_recv() else {
            panic!("expected a save");
        };
        assert_eq!(saved.len(), 1, "only the real connection");
        assert_eq!(saved[0].name, "prod");
    }

    #[test]
    fn deleting_a_connection_saves_and_forgets_its_password() {
        let (mut tree, mut rx) = tree();
        let mut kept = record("prod", "app.db");
        kept.secret = Secret::Keyring;
        let id = kept.id.clone();
        tree.update(Action::ConnectionsLoaded(vec![kept])).unwrap();
        while rx.try_recv().is_ok() {}

        press(&mut tree, KeyCode::Delete);
        assert_eq!(rx.try_recv().unwrap(), Action::ForgetSecret(id));
        let Ok(Action::SaveConnections(saved)) = rx.try_recv() else {
            panic!("expected a save");
        };
        assert!(saved.is_empty());
        assert!(lines(&mut tree, 30, 8).join("\n").contains("No connections yet"));
    }

    #[test]
    fn a_keychain_that_refused_downgrades_the_record_and_saves() {
        let (mut tree, mut rx) = tree();
        let mut record = record("prod", "app.db");
        record.secret = Secret::Keyring;
        let id = record.id.clone();
        tree.update(Action::ConnectionsLoaded(vec![record])).unwrap();
        while rx.try_recv().is_ok() {}

        tree.update(Action::SecretNotStored(id)).unwrap();
        let Ok(Action::SaveConnections(saved)) = rx.try_recv() else {
            panic!("expected a save");
        };
        assert_eq!(saved[0].secret, Secret::Prompt, "it asks each time now");
    }

    /// The cells of one rendered row, for checking its colours.
    fn row_style(tree: &mut ConnTree, width: u16, height: u16, y: u16) -> (Color, Color) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| tree.draw(frame, frame.area()).unwrap()).unwrap();
        let cell = &terminal.backend().buffer()[(1, y)];
        (cell.fg, cell.bg)
    }

    #[test]
    fn the_selection_is_green_and_the_rest_grey_with_no_background() {
        let (mut tree, _rx) = tree();
        connect(&mut tree, "one", "one.db");
        connect(&mut tree, "two", "two.db");

        // Row 0 is the border, so the connections are on rows 1 and 2.
        assert_eq!(row_style(&mut tree, 30, 8, 1), (Color::Green, Color::Reset));
        assert_eq!(row_style(&mut tree, 30, 8, 2), (Color::Gray, Color::Reset));

        press(&mut tree, KeyCode::Down);
        assert_eq!(row_style(&mut tree, 30, 8, 1), (Color::Gray, Color::Reset));
        assert_eq!(row_style(&mut tree, 30, 8, 2), (Color::Green, Color::Reset));
    }

    #[test]
    fn arrows_move_the_selection_and_stop_at_the_ends() {
        let (mut tree, _rx) = tree();
        connect(&mut tree, "one", "one.db");
        connect(&mut tree, "two", "two.db");

        press(&mut tree, KeyCode::Up);
        assert_eq!(tree.selected, 0);
        press(&mut tree, KeyCode::Down);
        assert_eq!(tree.selected, 1);
        press(&mut tree, KeyCode::Down);
        assert_eq!(tree.selected, 1, "stops at the last row");
        press(&mut tree, KeyCode::Home);
        assert_eq!(tree.selected, 0);
        press(&mut tree, KeyCode::End);
        assert_eq!(tree.selected, 1);
    }

    #[test]
    fn enter_connects_to_the_selected_connection() {
        let (mut tree, mut rx) = tree();
        connect(&mut tree, "one", "one.db");
        connect(&mut tree, "two", "two.db");
        while rx.try_recv().is_ok() {}

        press(&mut tree, KeyCode::Down);
        press(&mut tree, KeyCode::Enter);
        let Ok(Action::Connect(sent)) = rx.try_recv() else {
            panic!("expected a connection");
        };
        assert_eq!(sent.name, "two");
    }

    #[test]
    fn space_expands_the_active_connection_and_shows_its_tables() {
        let (mut tree, mut rx) = tree();
        connect(&mut tree, "prod", "app.db");
        while rx.try_recv().is_ok() {}

        press(&mut tree, KeyCode::Char(' '));
        assert_eq!(rx.try_recv().unwrap(), Action::ListTables);
        let shown = lines(&mut tree, 30, 8).join("\n");
        assert!(shown.contains("▾ prod") && shown.contains("loading"), "{shown}");

        tree.update(Action::TablesLoaded(vec!["users".into(), "orders".into()])).unwrap();
        let shown = lines(&mut tree, 30, 8).join("\n");
        assert!(shown.contains("users") && shown.contains("orders"), "{shown}");

        // Tables are rows of their own, so the selection walks through them.
        press(&mut tree, KeyCode::End);
        assert_eq!(tree.selected, 2);

        // Collapsing hides them again.
        press(&mut tree, KeyCode::Home);
        press(&mut tree, KeyCode::Char(' '));
        assert!(!lines(&mut tree, 30, 8).join("\n").contains("users"));
    }

    #[test]
    fn expanding_another_connection_opens_it_first() {
        let (mut tree, mut rx) = tree();
        connect(&mut tree, "one", "one.db");
        connect(&mut tree, "two", "two.db");
        tree.update(Action::TablesLoaded(vec!["t".into()])).unwrap();
        while rx.try_recv().is_ok() {}

        // "one" is not the active connection, so Space connects to it first.
        press(&mut tree, KeyCode::Home);
        press(&mut tree, KeyCode::Char(' '));
        let sent = rx.try_recv().unwrap();
        let Action::Connect(ref asked) = sent else { panic!("expected a connection") };
        assert_eq!(asked.name, "one");
        assert!(rx.try_recv().is_err(), "tables come after it is connected");
        // `App` hands every action back to the components, this one included.
        tree.update(sent).unwrap();

        tree.update(Action::Connected("one.db".into())).unwrap();
        assert_eq!(rx.try_recv().unwrap(), Action::ListTables);
        assert_eq!(tree.active, Some(0));

        tree.update(Action::TablesLoaded(vec!["people".into()])).unwrap();
        assert!(lines(&mut tree, 30, 8).join("\n").contains("people"));
    }

    #[test]
    fn a_failed_table_list_closes_the_branch_so_it_can_be_tried_again() {
        let (mut tree, mut rx) = tree();
        connect(&mut tree, "prod", "app.db");
        press(&mut tree, KeyCode::Char(' '));
        while rx.try_recv().is_ok() {}

        tree.update(Action::TablesFailed("connection lost".into())).unwrap();
        let shown = lines(&mut tree, 30, 8).join("\n");
        assert!(shown.contains("▸ prod"), "{shown}");
        assert!(!shown.contains("loading"), "{shown}");

        press(&mut tree, KeyCode::Char(' '));
        assert_eq!(rx.try_recv().unwrap(), Action::ListTables);
    }

    #[test]
    fn long_trees_scroll_to_keep_the_selection_in_view() {
        let (mut tree, _rx) = tree();
        for i in 0..20 {
            connect(&mut tree, &format!("c{i}"), &format!("{i}.db"));
        }
        press(&mut tree, KeyCode::End);

        // 8 rows tall: 2 border rows leave 6 connections in view.
        let shown = lines(&mut tree, 30, 8).join("\n");
        assert!(shown.contains("c19"), "{shown}");
        assert!(!shown.contains("c0 ") && !shown.contains("▸ c0"), "{shown}");
    }
}
