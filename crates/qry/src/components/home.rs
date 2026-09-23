use crossterm::event::KeyEvent;
use ratatui::layout::Size;
use ratatui::prelude::*;
use tokio::sync::mpsc::UnboundedSender;

use super::Component;
use crate::app::Mode;
use crate::components::connform::ConnForm;
use crate::components::conntree::ConnTree;
use crate::components::editor::Editor;
use crate::components::exportform::ExportForm;
use crate::components::form::Form;
use crate::components::help::Help;
use crate::components::results::Results;
use crate::components::statuspanel::Statuspanel;
use crate::keymap::{FormCommand, HelpCommand};
use crate::{action::Action, config::Config};

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

/// An answer a modal is waiting for, so a late one meant for a modal that has
/// since been closed cannot act on whatever is open now.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Awaiting {
    Connect,
    Export,
}

/// The main screen: the panes, and whatever is on top of them.
#[derive(Default)]
pub struct Home {
    command_tx: Option<UnboundedSender<Action>>,
    config: Config,

    conntree: ConnTree,
    editor: Editor,
    results: Results,
    statuspanel: Statuspanel,
    focus: Pane,

    /// The open modal, if any; the mode decides which one.
    modal: Option<Box<dyn Form>>,
    /// What that modal is waiting for the worker to answer, if anything.
    awaiting: Option<Awaiting>,
    help: Help,
    helpvisible: bool,
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

    fn children(&mut self) -> [&mut dyn Component; 4] {
        [
            &mut self.conntree,
            &mut self.editor,
            &mut self.results,
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

    /// The modal a mode shows, freshly filled in. `Home` shows none.
    fn modal_for(mode: Mode) -> Option<Box<dyn Form>> {
        match mode {
            Mode::Home => None,
            Mode::AddConnModal => Some(Box::new(ConnForm::default())),
            Mode::ExpoModal => Some(Box::new(ExportForm::default())),
        }
    }

    /// Sends what the open modal produced, or keeps it open showing why it
    /// was refused. Connecting and exporting are only known to have worked
    /// once the worker answers, so those modals wait for the answer instead
    /// of closing and losing everything that was typed.
    fn submit_modal(&mut self) -> color_eyre::Result<()> {
        if self.awaiting.is_some() {
            return Ok(());
        }
        let Some(modal) = &mut self.modal else {
            return Ok(());
        };
        match modal.submit() {
            Ok(action) => {
                let waiting = match action {
                    Action::Connect(..) => Some((Awaiting::Connect, "Connecting…")),
                    Action::Export(..) => Some((Awaiting::Export, "Exporting…")),
                    _ => None,
                };
                if let Some(tx) = &self.command_tx {
                    tx.send(action)?;
                    match waiting {
                        Some((answer, pending)) => {
                            modal.set_error(None);
                            modal.set_pending(Some(pending));
                            self.awaiting = Some(answer);
                        }
                        None => tx.send(Action::ChangeMode(Mode::Home))?,
                    }
                }
            }
            Err(error) => modal.set_error(Some(error)),
        }
        Ok(())
    }

    /// The worker answered the modal: it is done, so close it and let `App`
    /// leave the modal's mode behind too.
    fn close_modal(&mut self) -> color_eyre::Result<()> {
        self.modal = None;
        self.awaiting = None;
        if let Some(tx) = &self.command_tx {
            tx.send(Action::ChangeMode(Mode::Home))?;
        }
        Ok(())
    }

    /// The worker refused: keep the modal, with everything in it, and say why.
    fn fail_modal(&mut self, message: &str) {
        self.awaiting = None;
        if let Some(modal) = &mut self.modal {
            modal.set_pending(None);
            modal.set_error(Some(message.to_string()));
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
        self.help = Help::from_config(&config);
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
        if self.helpvisible {
            match self.config.panes.help.get(key) {
                Some(HelpCommand::Close) => self.helpvisible = false,
                Some(command) => self.help.scroll(command),
                None => {}
            }
            return Ok(None);
        }
        if self.modal.is_some() {
            let command = self.config.panes.form.get(key);
            if command == Some(FormCommand::Submit) {
                self.submit_modal()?;
            } else if let Some(modal) = &mut self.modal {
                modal.handle_key(key, command);
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
            Action::Help => {
                self.helpvisible = !self.helpvisible;
                if self.helpvisible {
                    self.help.scroll_to_top();
                }
            }
            Action::ChangeMode(mode) => {
                self.modal = Self::modal_for(*mode);
                self.awaiting = None;
            }
            // The worker's answer, but only to the modal that asked for it.
            Action::Connected(_) if self.awaiting == Some(Awaiting::Connect) => self.close_modal()?,
            Action::Exported(_) if self.awaiting == Some(Awaiting::Export) => self.close_modal()?,
            Action::ConnectFailed(message) if self.awaiting == Some(Awaiting::Connect) => {
                self.fail_modal(message);
            }
            Action::ExportFailed(message) if self.awaiting == Some(Awaiting::Export) => {
                self.fail_modal(message);
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
        let status_height = if self.statuspanel.is_empty() { 0 } else { 3 };

        let [all, statuspanel] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(status_height)]).areas(area);
        let [connpanel, rest] =
            Layout::horizontal([Constraint::Percentage(25), Constraint::Percentage(75)]).areas(all);
        let [querypane, resultspane] =
            Layout::vertical([Constraint::Percentage(40), Constraint::Fill(1)]).areas(rest);

        self.conntree.draw(frame, connpanel)?;
        self.editor.draw(frame, querypane)?;
        self.results.draw(frame, resultspane)?;
        self.statuspanel.draw(frame, statuspanel)?;

        if let Some(modal) = &self.modal {
            modal.render(frame, area);
        }
        if self.helpvisible {
            self.help.draw(frame, area)?;
        }
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
    use crate::components::form::tests::key;
    use crossterm::event::{KeyCode, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

    fn render(home: &mut Home, width: u16, height: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| home.draw(frame, frame.area()).unwrap())
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn row_text(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
    }

    fn shown(home: &mut Home, width: u16, height: u16) -> String {
        let buffer = render(home, width, height);
        (0..buffer.area.height)
            .map(|y| row_text(&buffer, y) + "\n")
            .collect()
    }

    /// A Home set up with the built-in config, as `App` would.
    fn home() -> Home {
        let mut home = Home::new();
        home.register_config_handler(Config::embedded()).unwrap();
        home
    }

    fn open(home: &mut Home, mode: Mode) {
        home.update(Action::ChangeMode(mode)).unwrap();
    }

    #[test]
    fn each_mode_opens_its_own_modal() {
        let mut home = home();
        assert!(home.modal.is_none());

        open(&mut home, Mode::AddConnModal);
        assert!(shown(&mut home, 80, 50).contains("New Connection"));

        open(&mut home, Mode::ExpoModal);
        let text = shown(&mut home, 80, 50);
        assert!(text.contains("Export") && !text.contains("New Connection"), "{text}");

        open(&mut home, Mode::Home);
        assert!(home.modal.is_none());
        assert!(!shown(&mut home, 80, 50).contains("Export"));
    }

    #[test]
    fn keys_reach_the_open_modal_and_it_reopens_empty() {
        let mut home = home();
        open(&mut home, Mode::ExpoModal);
        type_str_home(&mut home, "rows.csv");
        assert!(shown(&mut home, 80, 50).contains("rows.csv"));

        // Reopening builds a fresh form.
        open(&mut home, Mode::Home);
        open(&mut home, Mode::ExpoModal);
        assert!(!shown(&mut home, 80, 50).contains("rows.csv"));
    }

    fn type_str_home(home: &mut Home, text: &str) {
        for c in text.chars() {
            home.handle_key_event(key(KeyCode::Char(c))).unwrap();
        }
    }

    #[test]
    fn a_failed_export_keeps_the_modal_and_the_path() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut home = home();
        home.register_action_handler(tx).unwrap();
        open(&mut home, Mode::ExpoModal);
        type_str_home(&mut home, "no/such/dir/rows.csv");

        home.handle_key_event(key(KeyCode::Enter)).unwrap();
        let Ok(Action::Export(config)) = rx.try_recv() else {
            panic!("expected an export action");
        };
        assert_eq!(config.path, "no/such/dir/rows.csv");
        assert!(rx.try_recv().is_err(), "the mode must not change yet");
        assert!(shown(&mut home, 80, 50).contains("Exporting"));

        home.update(Action::ExportFailed("could not write no/such/dir/rows.csv".into())).unwrap();
        let text = shown(&mut home, 80, 50);
        assert!(text.contains("could not write"), "{text}");
        assert!(text.contains("no/such/dir/rows.csv"), "{text}");
    }

    #[test]
    fn a_written_export_closes_the_modal() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut home = home();
        home.register_action_handler(tx).unwrap();
        open(&mut home, Mode::ExpoModal);
        type_str_home(&mut home, "rows.csv");

        home.handle_key_event(key(KeyCode::Enter)).unwrap();
        assert!(matches!(rx.try_recv(), Ok(Action::Export(_))));

        home.update(Action::Exported("rows.csv".into())).unwrap();
        assert!(home.modal.is_none());
        assert_eq!(rx.try_recv().unwrap(), Action::ChangeMode(Mode::Home));
    }

    #[test]
    fn an_answer_meant_for_another_modal_is_ignored() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut home = home();
        home.register_action_handler(tx).unwrap();

        // An export is waiting, and a connection's answer arrives late.
        open(&mut home, Mode::ExpoModal);
        type_str_home(&mut home, "rows.csv");
        home.handle_key_event(key(KeyCode::Enter)).unwrap();
        home.update(Action::Connected(":memory:".into())).unwrap();
        home.update(Action::ConnectFailed("nope".into())).unwrap();

        let text = shown(&mut home, 80, 50);
        assert!(home.modal.is_some(), "the export modal was closed by a connection");
        assert!(text.contains("Exporting") && !text.contains("nope"), "{text}");
    }

    #[test]
    fn a_refused_modal_stays_open_with_its_error() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut home = home();
        home.register_action_handler(tx).unwrap();
        open(&mut home, Mode::ExpoModal);

        home.handle_key_event(key(KeyCode::Enter)).unwrap();
        assert!(rx.try_recv().is_err());
        assert!(shown(&mut home, 80, 50).contains("Path is required"));
    }

    /// Fills in enough of the New Connection form to be submittable.
    fn fill_connection(home: &mut Home) {
        for (field, value) in [(2, "db.local"), (4, "alice"), (6, "app")] {
            home.modal.as_mut().unwrap().set_focus(field);
            type_str_home(home, value);
        }
    }

    #[test]
    fn a_failed_connection_keeps_the_modal_and_everything_typed() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut home = home();
        home.register_action_handler(tx).unwrap();
        open(&mut home, Mode::AddConnModal);
        fill_connection(&mut home);

        // Submitting sends the connection but leaves the modal waiting.
        home.handle_key_event(key(KeyCode::Enter)).unwrap();
        assert!(matches!(rx.try_recv(), Ok(Action::Connect(..))));
        assert!(rx.try_recv().is_err(), "the mode must not change yet");
        assert!(shown(&mut home, 80, 50).contains("Connecting"));

        // A second Enter while waiting does not try again.
        home.handle_key_event(key(KeyCode::Enter)).unwrap();
        assert!(rx.try_recv().is_err());

        home.update(Action::ConnectFailed("password authentication failed".into())).unwrap();
        let text = shown(&mut home, 80, 50);
        assert!(text.contains("password authentication failed"), "{text}");
        assert!(text.contains("db.local") && text.contains("alice"), "{text}");
        assert!(!text.contains("Connecting"), "{text}");

        // Correcting and submitting again sends a second attempt.
        home.handle_key_event(key(KeyCode::Char('x'))).unwrap();
        home.handle_key_event(key(KeyCode::Enter)).unwrap();
        assert!(matches!(rx.try_recv(), Ok(Action::Connect(..))));
    }

    #[test]
    fn a_working_connection_closes_the_modal() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut home = home();
        home.register_action_handler(tx).unwrap();
        open(&mut home, Mode::AddConnModal);
        fill_connection(&mut home);

        home.handle_key_event(key(KeyCode::Enter)).unwrap();
        assert!(matches!(rx.try_recv(), Ok(Action::Connect(..))));

        home.update(Action::Connected("alice@db.local:5432/app".into())).unwrap();
        assert!(home.modal.is_none());
        assert_eq!(rx.try_recv().unwrap(), Action::ChangeMode(Mode::Home));
        assert!(!shown(&mut home, 80, 50).contains("New Connection"));
    }

    #[test]
    fn a_connect_answer_without_a_waiting_modal_is_ignored() {
        let mut home = home();
        // The in-memory connection at startup answers with nobody waiting.
        home.update(Action::Connected(":memory:".into())).unwrap();
        home.update(Action::ConnectFailed("nope".into())).unwrap();
        assert!(home.modal.is_none());

        // An export modal is not touched by a connection's answer either.
        open(&mut home, Mode::ExpoModal);
        home.update(Action::ConnectFailed("nope".into())).unwrap();
        let text = shown(&mut home, 80, 50);
        assert!(text.contains("Export") && !text.contains("nope"), "{text}");
    }

    #[test]
    fn help_opens_scrolls_and_closes_with_configured_keys() {
        let mut home = home();
        home.update(Action::Help).unwrap();
        assert!(home.helpvisible);
        assert!(shown(&mut home, 80, 20).contains("Global:"));

        // While help is open, keys go to it, not to the focused pane.
        home.focus = Pane::Results;
        home.handle_key_event(key(KeyCode::PageDown)).unwrap();
        assert!(!shown(&mut home, 80, 20).contains("Global:"));

        home.handle_key_event(key(KeyCode::Esc)).unwrap();
        assert!(!home.helpvisible);

        home.update(Action::Help).unwrap();
        assert!(shown(&mut home, 80, 20).contains("Global:"));
        home.update(Action::Help).unwrap();
        assert!(!home.helpvisible);
    }

    #[test]
    fn update_is_forwarded_to_children() {
        let mut home = home();
        assert!(home.statuspanel.is_empty());
        home.update(Action::Status(StatusCode::Error("e".into()))).unwrap();
        assert!(!home.statuspanel.is_empty());
    }

    #[test]
    fn tree_is_focused_and_highlighted_at_startup() {
        let mut home = home();
        assert!(home.focus == Pane::Tree);

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
    fn keys_reach_the_focused_pane_when_nothing_is_on_top() {
        let mut home = home();
        home.focus = Pane::Editor;
        home.sync_focus();
        type_str_home(&mut home, "select 1");
        assert_eq!(
            home.handle_key_event(KeyEvent::new(KeyCode::F(8), KeyModifiers::NONE)).unwrap(),
            Some(Action::Execute("select 1".into()))
        );
    }
}
