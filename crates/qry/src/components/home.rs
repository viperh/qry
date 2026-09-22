
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::
prelude::*;
use ratatui::layout::{Flex, Size};

use qry_core::{
    ConnectionConfig, SslMode, mariadb::MariadbConfig, mysql::MySqlConfig,
    postgres::PostgresConfig, sqlite::SqliteConfig,
};
use tokio::sync::mpsc::UnboundedSender;

use super::Component;
use crate::app::Mode;
use crate::components::conntree::ConnTree;
use crate::components::editor::Editor;
use crate::components::infopanel::Infopanel;
use crate::components::results::Results;
use crate::components::statuspanel::Statuspanel;
use crate::{action::Action, config::Config};
use ratatui::widgets::{Clear, Block, Paragraph};

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Pane {
    #[default]
    Tree,
    Editor,
    Results,
}

impl Pane {
    fn next(self) -> Self {
        match self {
            Pane::Tree => Pane::Editor,
            Pane::Editor => Pane::Results,
            Pane::Results => Pane::Tree,
        }
    }

    fn prev(self) -> Self {
        match self {
            Pane::Tree => Pane::Results,
            Pane::Editor => Pane::Tree,
            Pane::Results => Pane::Editor,
        }
    }
}

/// Single-line text input. `cursor` counts characters, not bytes.
#[derive(Default)]
struct TextInput {
    value: String,
    cursor: usize,
    masked: bool,
}

impl TextInput {
    fn handle_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char(c) if !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {
                let at = self.byte_index();
                self.value.insert(at, c);
                self.cursor += 1;
            }
            KeyCode::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                let at = self.byte_index();
                self.value.remove(at);
            }
            KeyCode::Delete if self.cursor < self.len() => {
                let at = self.byte_index();
                self.value.remove(at);
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.len()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.len(),
            _ => {}
        }
    }

    fn len(&self) -> usize {
        self.value.chars().count()
    }

    fn byte_index(&self) -> usize {
        self.value
            .char_indices()
            .nth(self.cursor)
            .map_or(self.value.len(), |(i, _)| i)
    }

    /// Draws the input, scrolled so the cursor stays visible. The terminal
    /// cursor is only placed on the focused input.
    fn render(&self, frame: &mut Frame, area: Rect, focused: bool) {
        let width = usize::from(area.width).max(1);
        let offset = (self.cursor + 1).saturating_sub(width);
        let visible: String = if self.masked {
            "•".repeat(self.len().saturating_sub(offset).min(width))
        } else {
            self.value.chars().skip(offset).take(width).collect()
        };

        frame.render_widget(Paragraph::new(visible), area);
        if focused {
            let x = area.x + u16::try_from(self.cursor - offset).unwrap_or(0);
            frame.set_cursor_position((x, area.y));
        }
    }
}

const LABELS: [&str; 9] = [
    "Name", "Type", "Host", "Port", "User", "Password", "Database", "Schema", "SSL mode",
];
// Indexes into `LABELS` and `ConnForm::fields`.
const NAME: usize = 0;
const TYPE: usize = 1;
const HOST: usize = 2;
const PORT: usize = 3;
const USER: usize = 4;
const PASSWORD: usize = 5;
const DATABASE: usize = 6;
const SCHEMA: usize = 7;
const SSL_MODE: usize = 8;

const DB_TYPES: [&str; 4] = ["PostgreSQL", "MySQL", "MariaDB", "SQLite"];
/// Same order as `SslMode::ALL`.
const SSL_MODES: [&str; 6] = ["disable", "allow", "prefer", "require", "verify-ca", "verify-full"];

/// Each field is a bordered box, with a blank row between boxes.
const FIELD_HEIGHT: u16 = 3;
const FIELD_GAP: u16 = 1;
const MODAL_WIDTH: u16 = 60;
/// All fields, plus one row of padding and the popup border above and below.
const MODAL_HEIGHT: u16 = LABELS.len() as u16 * (FIELD_HEIGHT + FIELD_GAP) - FIELD_GAP + 4;

enum Field {
    Text(TextInput),
    /// Cycled with ← / → (or Space) instead of typed.
    Choice {
        options: &'static [&'static str],
        selected: usize,
    },
}

impl Field {
    fn handle_key(&mut self, key: KeyEvent) {
        match self {
            Field::Text(input) => input.handle_key(key),
            Field::Choice { options, selected } => match key.code {
                KeyCode::Right | KeyCode::Char(' ') => *selected = (*selected + 1) % options.len(),
                KeyCode::Left => *selected = (*selected + options.len() - 1) % options.len(),
                _ => {}
            },
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect, focused: bool) {
        match self {
            Field::Text(input) => input.render(frame, area, focused),
            Field::Choice { options, selected } => {
                let option = options[*selected];
                let line = if focused {
                    Line::from(format!("‹ {option} ›")).cyan()
                } else {
                    Line::from(format!("  {option}"))
                };
                frame.render_widget(line, area);
            }
        }
    }
}

/// The New Connection form. `fields` and `focus` both index `LABELS`.
struct ConnForm {
    fields: [Field; 9],
    focus: usize,
    /// Why the last submit was refused. Cleared by the next edit.
    error: Option<String>,
}

impl Default for ConnForm {
    fn default() -> Self {
        let text = || Field::Text(TextInput::default());
        Self {
            fields: [
                text(),
                Field::Choice { options: &DB_TYPES, selected: 0 },
                text(),
                text(),
                text(),
                Field::Text(TextInput { masked: true, ..TextInput::default() }),
                text(),
                text(),
                // "prefer", libpq's own default
                Field::Choice { options: &SSL_MODES, selected: 2 },
            ],
            focus: 0,
            error: None,
        }
    }
}

impl ConnForm {
    fn handle_key(&mut self, key: KeyEvent) {
        let fields = LABELS.len();
        match key.code {
            KeyCode::Tab => self.focus = (self.focus + 1) % fields,
            KeyCode::BackTab => self.focus = (self.focus + fields - 1) % fields,
            _ => {
                self.error = None;
                self.fields[self.focus].handle_key(key);
            }
        }
    }

    fn text(&self, field: usize) -> &str {
        match &self.fields[field] {
            Field::Text(input) => &input.value,
            Field::Choice { .. } => "",
        }
    }

    fn selected(&self, field: usize) -> usize {
        match &self.fields[field] {
            Field::Choice { selected, .. } => *selected,
            Field::Text(_) => 0,
        }
    }

    /// Turns the form into a connection name and config, or explains what is
    /// wrong in a message short enough for the popup's bottom border.
    fn to_config(&self) -> Result<(String, ConnectionConfig), String> {
        let trimmed = |field: usize| self.text(field).trim().to_string();
        let required = |field: usize| {
            let value = trimmed(field);
            if value.is_empty() {
                Err(format!("{} is required", LABELS[field]))
            } else {
                Ok(value)
            }
        };
        let name = trimmed(NAME);
        let schema = trimmed(SCHEMA);
        let db_type = DB_TYPES[self.selected(TYPE)];

        if db_type == "SQLite" {
            if !schema.is_empty() {
                return Err("SQLite has no schemas; leave Schema empty".into());
            }
            let path = required(DATABASE)
                .map_err(|_| "Database (the SQLite file path) is required".to_string())?;
            return Ok((name, ConnectionConfig::Sqlite(SqliteConfig::new(path, false))));
        }

        let host = required(HOST)?;
        let port = match trimmed(PORT).as_str() {
            "" if db_type == "PostgreSQL" => 5432,
            "" => 3306,
            port => port
                .parse::<u16>()
                .ok()
                .filter(|&port| port != 0)
                .ok_or("Port must be a number from 1 to 65535")?,
        };
        let user = required(USER)?;
        // Not trimmed: leading or trailing spaces can be part of a password.
        let password = self.text(PASSWORD).to_string();
        let database = required(DATABASE)?;
        let ssl_mode = SslMode::ALL[self.selected(SSL_MODE)];

        let config = match db_type {
            "PostgreSQL" => ConnectionConfig::Postgres(PostgresConfig {
                host,
                port,
                user,
                password,
                database,
                schema: (!schema.is_empty()).then_some(schema),
                ssl_mode,
            }),
            _ if !schema.is_empty() => {
                return Err(format!("{db_type} has no separate schemas; use Database"));
            }
            "MySQL" => ConnectionConfig::Mysql(MySqlConfig { host, port, user, password, database, ssl_mode }),
            _ => ConnectionConfig::MariaDb(MariadbConfig { host, port, user, password, database, ssl_mode }),
        };
        Ok((name, config))
    }

    /// Draws as many fields as fit, scrolled so the focused one is visible.
    fn render(&self, frame: &mut Frame, area: Rect) {
        let fits = usize::from((area.height + FIELD_GAP) / (FIELD_HEIGHT + FIELD_GAP))
            .clamp(1, LABELS.len());
        let first = (self.focus + 1).saturating_sub(fits);
        let rows = Layout::vertical(vec![Constraint::Length(FIELD_HEIGHT); fits])
            .spacing(FIELD_GAP)
            .split(area);

        for (row, i) in rows.iter().zip(first..) {
            if row.height < FIELD_HEIGHT {
                continue;
            }
            let focused = i == self.focus;
            let [label_area, box_area] =
                Layout::horizontal([Constraint::Length(12), Constraint::Fill(1)]).areas(*row);

            // The label sits on the box's middle row, level with the text.
            let label_area = Rect { y: label_area.y + 1, height: 1, ..label_area };
            let label = if focused {
                Line::from(format!("› {}", LABELS[i])).cyan().bold()
            } else {
                Line::from(format!("  {}", LABELS[i]))
            };
            frame.render_widget(label, label_area);

            let block = Block::bordered().border_style(if focused {
                Style::new().cyan()
            } else {
                Style::new().dark_gray()
            });
            let input_area = block.inner(box_area).inner(Margin::new(1, 0));
            frame.render_widget(block, box_area);

            self.fields[i].render(frame, input_area, focused);
        }
    }
}

#[derive(Default)]
pub struct Home {
    command_tx: Option<UnboundedSender<Action>>,
    config: Config,

    conntree: ConnTree,
    editor: Editor,
    results: Results,
    infopanel: Infopanel,
    statuspanel: Statuspanel,
    focus: Pane,
    modal_active: bool,
    form: ConnForm,

}

impl Home {
    pub fn new() -> Self {
        let mut home = Self::default();
        home.sync_focus();
        home
    }

    pub fn sync_focus(&mut self) {
        self.conntree.set_focus(self.focus == Pane::Tree);
        self.editor.set_focus(self.focus == Pane::Editor);
        self.results.set_focus(self.focus == Pane::Results);
    }

    fn children(&mut self) -> [&mut dyn Component; 5] {
        [
            &mut self.conntree,
            &mut self.editor,
            &mut self.results,
            &mut self.infopanel,
            &mut self.statuspanel,
        ]
    }

    fn focused(&mut self) -> &mut dyn Component {
        match self.focus {
            Pane::Tree => &mut self.conntree,
            Pane::Editor => &mut self.editor,
            Pane::Results => &mut self.results,
        }
    }

    /// Sends the form's connection to the database worker and closes the
    /// modal, or keeps it open showing why the form was refused.
    fn submit(&mut self) -> color_eyre::Result<()> {
        match self.form.to_config() {
            Ok((name, config)) => {
                if let Some(tx) = &self.command_tx {
                    tx.send(Action::Connect(name, config))?;
                    tx.send(Action::ChangeMode(Mode::Home))?;
                }
            }
            Err(error) => self.form.error = Some(error),
        }
        Ok(())
    }

    fn render_modal(&self, frame: &mut Frame, area: Rect) {
        if self.modal_active {
            // Shrinks to fit short terminals; the form then scrolls.
            let height = MODAL_HEIGHT.min(area.height);
            let [popup] = Layout::horizontal([Constraint::Length(MODAL_WIDTH)]).flex(Flex::Center).areas(area);
            let [popup] = Layout::vertical([Constraint::Length(height)]).flex(Flex::Center).areas(popup);
            frame.render_widget(Clear, popup);
            let footer = match &self.form.error {
                Some(error) => Line::from(format!(" {error} ")).red(),
                None => Line::from(" Enter connect · Esc cancel ").dark_gray(),
            };
            let block = Block::bordered().title("New Connection").title_bottom(footer);
            let inner = block.inner(popup).inner(Margin::new(2, 1));
            frame.render_widget(block, popup);
            self.form.render(frame, inner);

        }


    }

}

impl Component for Home {
    fn register_action_handler(&mut self, tx: UnboundedSender<Action>) -> color_eyre::Result<()> {
        for child in self.children() {
            child.register_action_handler(tx.clone())?;
        }
        self.command_tx = Some(tx);
        Ok(())
    }

    fn register_config_handler(&mut self, config: Config) -> color_eyre::Result<()> {
        for child in self.children() {
            child.register_config_handler(config.clone())?;
        }
        self.config = config;
        Ok(())
    }

    fn init(&mut self, area: Size) -> color_eyre::Result<()> {
        for child in self.children() {
            child.init(area)?;
        }
        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> color_eyre::Result<Option<Action>> {
        if self.modal_active {
            if key.code == KeyCode::Enter {
                self.submit()?;
            } else {
                self.form.handle_key(key);
            }
            return Ok(None);
        }
        self.focused().handle_key_event(key)
    }

    fn update(&mut self, action: Action) -> color_eyre::Result<Option<Action>> {
        match &action {
            Action::Tick => {}
            Action::Render => {}
            Action::FocusNext => {
                self.focus = self.focus.next();
                self.sync_focus()
            }
            Action::FocusPrev => {
                self.focus = self.focus.prev();
                self.sync_focus()
            }
            Action::ChangeMode(m) => {
                self.modal_active = *m == Mode::AddConnModal;
                if self.modal_active {
                    self.form = ConnForm::default();
                }
            }
            _ => {}
        }

        let mut result = None;
        for child in self.children() {
            let child_result = child.update(action.clone())?;
            if result.is_none() {
                result = child_result;
            }
        }
        Ok(result)
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) -> color_eyre::Result<()> {

        let info_height = if self.infopanel.is_empty() {0} else {3};
        let status_height = if self.statuspanel.is_empty() {0} else {3};


        let [all, statuspanel] = Layout::vertical([Constraint::Fill(1), Constraint::Length(status_height)]).areas(area);
        let [connpanel, rest] = Layout::horizontal([Constraint::Percentage(25), Constraint::Percentage(75)]).areas(all);

        let [querypane, infopane, resultspane] = Layout::vertical([
            Constraint::Percentage(40),
            Constraint::Length(info_height),
            Constraint::Fill(1)])
            .areas(rest);


         self.conntree.draw(frame, connpanel)?;
         self.editor.draw(frame, querypane)?;
        self.infopanel.draw(frame, infopane)?;
        self.results.draw(frame, resultspane)?;
        self.statuspanel.draw(frame, statuspanel)?;

        self.render_modal(frame, area);

        Ok(())
    }

    fn set_focus(&mut self, focus: bool) {
        let _ = focus;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::StatusCode;
    use crossterm::event::{KeyCode, KeyModifiers};
    use ratatui::{backend::TestBackend, buffer::Buffer, Terminal};

    fn render(home: &mut Home, width: u16, height: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| home.draw(frame, frame.area()).unwrap())
            .unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn change_mode_toggles_modal() {
        let mut home = Home::new();
        home.update(Action::ChangeMode(Mode::AddConnModal)).unwrap();
        assert!(home.modal_active);
        home.update(Action::ChangeMode(Mode::Home)).unwrap();
        assert!(!home.modal_active);
    }

    #[test]
    fn key_events_ignored_while_modal_shown() {
        let mut home = Home::new();
        home.update(Action::ChangeMode(Mode::AddConnModal)).unwrap();
        let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        assert_eq!(home.handle_key_event(key).unwrap(), None);
    }

    #[test]
    fn update_is_forwarded_to_children() {
        let mut home = Home::new();
        assert!(home.infopanel.is_empty());
        assert!(home.statuspanel.is_empty());
        home.update(Action::Info("x".into())).unwrap();
        assert!(!home.infopanel.is_empty());
        home.update(Action::Status(StatusCode::Error("e".into()))).unwrap();
        assert!(!home.statuspanel.is_empty());
    }

    #[test]
    fn tree_is_focused_and_highlighted_at_startup() {
        let mut home = Home::new();
        assert!(home.focus == Pane::Tree);

        // Only the tree pane's border (top-left corner of the frame) is highlighted.
        let buf = render(&mut home, 80, 24);
        assert_eq!(buf[(0, 0)].fg, Color::Yellow);
        assert_ne!(buf[(20, 0)].fg, Color::Yellow);

        home.update(Action::FocusNext).unwrap();
        assert!(home.focus == Pane::Editor);
        let buf = render(&mut home, 80, 24);
        assert_ne!(buf[(0, 0)].fg, Color::Yellow);
        assert_eq!(buf[(20, 0)].fg, Color::Cyan);

        home.update(Action::FocusNext).unwrap();
        assert!(home.focus == Pane::Results);
        home.update(Action::FocusNext).unwrap();
        assert!(home.focus == Pane::Tree);
        home.update(Action::FocusPrev).unwrap();
        assert!(home.focus == Pane::Results);
    }

    #[test]
    fn popup_fits_all_fields_and_is_centered() {
        let mut home = Home::new();
        home.update(Action::ChangeMode(Mode::AddConnModal)).unwrap();
        let buf = render(&mut home, 80, 50);

        let (left, right) = (10, 10 + MODAL_WIDTH - 1);
        let top = (0..50)
            .find(|&y| buf[(left, y)].symbol() == "┌")
            .expect("popup top-left corner not found");
        let bottom = top + MODAL_HEIGHT - 1;

        assert_eq!(MODAL_HEIGHT, 39);
        assert!(top.abs_diff(50 - (bottom + 1)) <= 1, "top={top}");
        assert_eq!(buf[(right, top)].symbol(), "┐");
        assert_eq!(buf[(left, bottom)].symbol(), "└");
        assert_eq!(buf[(right, bottom)].symbol(), "┘");
        let mid = top + MODAL_HEIGHT / 2;
        assert_eq!(buf[(left, mid)].symbol(), "│");
        assert_eq!(buf[(right, mid)].symbol(), "│");
        assert_eq!(buf[(left - 1, mid)].symbol(), " ");
        assert_eq!(buf[(right + 1, mid)].symbol(), " ");
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn type_str(target: &mut impl FnMut(KeyEvent), text: &str) {
        text.chars().for_each(|c| target(key(KeyCode::Char(c))));
    }

    fn row_text(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
    }

    #[test]
    fn text_input_edits_at_the_cursor() {
        let mut input = TextInput::default();
        type_str(&mut |k| input.handle_key(k), "hélo");
        input.handle_key(key(KeyCode::Left));
        input.handle_key(key(KeyCode::Char('l')));
        assert_eq!(input.value, "héllo");

        input.handle_key(key(KeyCode::Home));
        input.handle_key(key(KeyCode::Delete));
        input.handle_key(key(KeyCode::End));
        input.handle_key(key(KeyCode::Backspace));
        assert_eq!(input.value, "éll");
        assert_eq!(input.cursor, 3);

        // Shortcuts such as Ctrl-C are not typed into the field.
        input.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(input.value, "éll");
    }

    #[test]
    fn tab_and_backtab_cycle_through_all_nine_fields() {
        let mut form = ConnForm::default();
        for expected in (1..LABELS.len()).chain([0]) {
            form.handle_key(key(KeyCode::Tab));
            assert_eq!(form.focus, expected);
        }
        form.handle_key(key(KeyCode::BackTab));
        assert_eq!(form.focus, SSL_MODE);
    }

    #[test]
    fn keys_reach_the_focused_field_while_the_modal_is_open() {
        let mut home = Home::new();
        home.update(Action::ChangeMode(Mode::AddConnModal)).unwrap();
        type_str(&mut |k| { home.handle_key_event(k).unwrap(); }, "prod");
        home.handle_key_event(key(KeyCode::Tab)).unwrap();
        home.handle_key_event(key(KeyCode::Tab)).unwrap();
        type_str(&mut |k| { home.handle_key_event(k).unwrap(); }, "db.local");

        assert_eq!(home.form.text(NAME), "prod");
        assert_eq!(home.form.text(HOST), "db.local");
        assert_eq!(home.form.focus, 2);
    }

    #[test]
    fn ssl_mode_is_a_choice_not_text() {
        let mut form = ConnForm { focus: SSL_MODE, ..ConnForm::default() };
        assert_eq!(SSL_MODES[form.selected(SSL_MODE)], "prefer");
        form.handle_key(key(KeyCode::Right));
        assert_eq!(SSL_MODES[form.selected(SSL_MODE)], "require");
        form.handle_key(key(KeyCode::Char('x')));
        assert_eq!(SSL_MODES[form.selected(SSL_MODE)], "require");
        form.handle_key(key(KeyCode::Left));
        form.handle_key(key(KeyCode::Left));
        form.handle_key(key(KeyCode::Left));
        assert_eq!(SSL_MODES[form.selected(SSL_MODE)], "disable");
        form.handle_key(key(KeyCode::Left));
        assert_eq!(SSL_MODES[form.selected(SSL_MODE)], "verify-full");
    }

    #[test]
    fn modal_shows_labels_and_masks_the_password() {
        let mut home = Home::new();
        home.update(Action::ChangeMode(Mode::AddConnModal)).unwrap();
        home.form.focus = PASSWORD;
        type_str(&mut |k| { home.handle_key_event(k).unwrap(); }, "hunter2");
        let buf = render(&mut home, 80, 50);

        let rows: Vec<String> = (0..50).map(|y| row_text(&buf, y)).collect();
        for label in LABELS {
            assert!(rows.iter().any(|r| r.contains(label)), "missing label {label}");
        }
        let password_row = rows.iter().find(|r| r.contains("Password")).unwrap();
        assert!(password_row.contains("› Password"));
        assert!(password_row.contains("•••••••"));
        assert!(!rows.iter().any(|r| r.contains("hunter2")));
        assert!(rows.iter().any(|r| r.contains("prefer")));
    }

    #[test]
    fn every_field_has_its_own_box_with_a_gap_after_it() {
        let mut home = Home::new();
        home.update(Action::ChangeMode(Mode::AddConnModal)).unwrap();
        let buf = render(&mut home, 80, 50);
        let rows: Vec<String> = (0..50).map(|y| row_text(&buf, y)).collect();
        println!("{}", rows.join("\n"));

        // Box column: popup left edge (10) + border + 2 padding + 12 label columns.
        let x = 10 + 1 + 2 + 12;
        let tops: Vec<u16> = (0..50).filter(|&y| buf[(x, y)].symbol() == "┌").collect();
        assert_eq!(tops.len(), LABELS.len(), "{tops:?}");
        for pair in tops.windows(2) {
            // 3-row box, then 1 blank row.
            assert_eq!(pair[1] - pair[0], FIELD_HEIGHT + FIELD_GAP);
            assert_eq!(buf[(x, pair[0] + 2)].symbol(), "└");
            assert_eq!(buf[(x, pair[0] + 3)].symbol(), " ");
        }
    }

    #[test]
    fn short_terminal_scrolls_to_the_focused_field() {
        let mut home = Home::new();
        home.update(Action::ChangeMode(Mode::AddConnModal)).unwrap();
        home.form.focus = SSL_MODE;
        let buf = render(&mut home, 80, 24);
        let rows: Vec<String> = (0..24).map(|y| row_text(&buf, y)).collect();

        assert!(rows.iter().any(|r| r.contains("› SSL mode")), "{rows:#?}");
        assert!(!rows.iter().any(|r| r.contains("Name")), "{rows:#?}");
    }

    #[test]
    fn reopening_the_modal_starts_a_fresh_form() {
        let mut home = Home::new();
        home.update(Action::ChangeMode(Mode::AddConnModal)).unwrap();
        type_str(&mut |k| { home.handle_key_event(k).unwrap(); }, "old");
        home.update(Action::ChangeMode(Mode::Home)).unwrap();
        home.update(Action::ChangeMode(Mode::AddConnModal)).unwrap();
        assert_eq!(home.form.text(NAME), "");
    }

    #[test]
    fn type_is_a_choice_not_text() {
        let mut form = ConnForm { focus: TYPE, ..ConnForm::default() };
        assert_eq!(DB_TYPES[form.selected(TYPE)], "PostgreSQL");
        form.handle_key(key(KeyCode::Right));
        assert_eq!(DB_TYPES[form.selected(TYPE)], "MySQL");
        form.handle_key(key(KeyCode::Char('x')));
        assert_eq!(DB_TYPES[form.selected(TYPE)], "MySQL");
        form.handle_key(key(KeyCode::Left));
        form.handle_key(key(KeyCode::Left));
        assert_eq!(DB_TYPES[form.selected(TYPE)], "SQLite");
    }

    #[test]
    fn ssl_mode_labels_line_up_with_core() {
        let core: Vec<String> = SslMode::ALL.iter().map(ToString::to_string).collect();
        assert_eq!(core, SSL_MODES);
    }

    fn set(form: &mut ConnForm, field: usize, value: &str) {
        let Field::Text(input) = &mut form.fields[field] else {
            panic!("{} is not a text field", LABELS[field]);
        };
        input.value = value.into();
        input.cursor = input.len();
    }

    fn choose(form: &mut ConnForm, field: usize, option: &str) {
        let Field::Choice { options, selected } = &mut form.fields[field] else {
            panic!("{} is not a choice", LABELS[field]);
        };
        *selected = options.iter().position(|o| *o == option).unwrap();
    }

    fn filled_postgres_form() -> ConnForm {
        let mut form = ConnForm::default();
        set(&mut form, NAME, " prod ");
        set(&mut form, HOST, "db.local");
        set(&mut form, USER, "alice");
        set(&mut form, PASSWORD, " s3cret ");
        set(&mut form, DATABASE, "app");
        set(&mut form, SCHEMA, "reporting");
        form
    }

    #[test]
    fn postgres_form_builds_its_config() {
        let (name, config) = filled_postgres_form().to_config().unwrap();
        assert_eq!(name, "prod");
        assert_eq!(
            config,
            ConnectionConfig::Postgres(PostgresConfig {
                host: "db.local".into(),
                port: 5432,
                user: "alice".into(),
                password: " s3cret ".into(),
                database: "app".into(),
                schema: Some("reporting".into()),
                ssl_mode: SslMode::Prefer,
            })
        );
    }

    #[test]
    fn mysql_and_mariadb_default_to_port_3306_and_reject_a_schema() {
        let mut form = filled_postgres_form();
        choose(&mut form, TYPE, "MySQL");
        choose(&mut form, SSL_MODE, "disable");
        assert_eq!(form.to_config().unwrap_err(), "MySQL has no separate schemas; use Database");

        set(&mut form, SCHEMA, "");
        let (_, config) = form.to_config().unwrap();
        let ConnectionConfig::Mysql(mysql) = config else { panic!("{config:?}") };
        assert_eq!((mysql.port, mysql.ssl_mode), (3306, SslMode::Disable));

        choose(&mut form, TYPE, "MariaDB");
        set(&mut form, PORT, "3307");
        let (_, config) = form.to_config().unwrap();
        let ConnectionConfig::MariaDb(mariadb) = config else { panic!("{config:?}") };
        assert_eq!(mariadb.port, 3307);
    }

    #[test]
    fn sqlite_only_needs_a_file_path() {
        let mut form = ConnForm::default();
        choose(&mut form, TYPE, "SQLite");
        assert_eq!(
            form.to_config().unwrap_err(),
            "Database (the SQLite file path) is required"
        );
        set(&mut form, DATABASE, "C:/data/app.db");
        assert_eq!(
            form.to_config().unwrap().1,
            ConnectionConfig::Sqlite(SqliteConfig::new("C:/data/app.db", false))
        );
    }

    #[test]
    fn invalid_forms_explain_what_is_wrong() {
        let mut form = filled_postgres_form();
        set(&mut form, HOST, "   ");
        assert_eq!(form.to_config().unwrap_err(), "Host is required");

        let mut form = filled_postgres_form();
        for port in ["abc", "0", "70000"] {
            set(&mut form, PORT, port);
            assert_eq!(form.to_config().unwrap_err(), "Port must be a number from 1 to 65535");
        }
    }

    #[test]
    fn enter_sends_the_connection_and_closes_the_modal() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut home = Home::new();
        home.register_action_handler(tx).unwrap();
        home.update(Action::ChangeMode(Mode::AddConnModal)).unwrap();
        home.form = filled_postgres_form();

        home.handle_key_event(key(KeyCode::Enter)).unwrap();
        let expected = home.form.to_config().unwrap();
        assert_eq!(rx.try_recv().unwrap(), Action::Connect(expected.0, expected.1));
        assert_eq!(rx.try_recv().unwrap(), Action::ChangeMode(Mode::Home));
    }

    #[test]
    fn enter_on_an_invalid_form_shows_the_error_and_sends_nothing() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut home = Home::new();
        home.register_action_handler(tx).unwrap();
        home.update(Action::ChangeMode(Mode::AddConnModal)).unwrap();

        home.handle_key_event(key(KeyCode::Enter)).unwrap();
        assert!(rx.try_recv().is_err());
        let buf = render(&mut home, 80, 50);
        assert!((0..50).any(|y| row_text(&buf, y).contains("Host is required")));

        // The next edit clears the message.
        home.handle_key_event(key(KeyCode::Char('x'))).unwrap();
        assert_eq!(home.form.error, None);
    }

    #[test]
    fn long_values_scroll_to_keep_the_cursor_visible() {
        let mut input = TextInput::default();
        type_str(&mut |k| input.handle_key(k), "abcdefghij");
        let mut terminal = Terminal::new(TestBackend::new(5, 1)).unwrap();
        terminal
            .draw(|frame| input.render(frame, frame.area(), true))
            .unwrap();
        // Cursor sits after "j", so the last four characters plus the cursor cell show.
        assert_eq!(row_text(terminal.backend().buffer(), 0), "ghij ");
    }
}
