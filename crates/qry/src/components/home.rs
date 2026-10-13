use crossterm::event::KeyEvent;
use ratatui::{layout::Size, prelude::*};
use tokio::sync::mpsc::UnboundedSender;

use super::Component;
use crate::{
    action::Action,
    app::Mode,
    components::{
        connexportform::ConnExportForm, connform::ConnForm, conntree::ConnTree, editor::Editor,
        exportform::ExportForm, form::Form, help::Help, importform::ImportForm,
        passwordform::PasswordForm, results::Results, statuspanel::Statuspanel,
    },
    config::Config,
    keymap::{FormCommand, HelpCommand},
    theme::Theme,
};

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
    Import,
    ExportConnections,
}

/// The main screen: the panes, and whatever is on top of them.
#[derive(Default)]
pub(crate) struct Home {
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
    /// Whose password the worker asked for, for the prompt's title.
    asked: Option<String>,
    /// The connection the tree asked to change, for the form to open with.
    editing: Option<Box<crate::connections::StoredConnection>>,
    help: Help,
    helpvisible: bool,
}

impl Home {
    pub(crate) fn new() -> Self {
        let mut home = Self::default();
        home.sync_focus();
        home
    }

    pub(crate) fn sync_focus(&mut self) {
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
    fn modal_for(&mut self, mode: Mode) -> Option<Box<dyn Form>> {
        match mode {
            Mode::Home => None,
            Mode::AddConnModal => Some(match self.editing.take() {
                Some(record) => Box::new(ConnForm::editing(&record)),
                None => Box::new(ConnForm::default()),
            }),
            Mode::ExpoModal => Some(Box::new(ExportForm::default())),
            Mode::PasswordModal => Some(Box::new(PasswordForm::new(
                self.asked.clone().unwrap_or_default(),
            ))),
            Mode::ImportModal => Some(Box::new(ImportForm::default())),
            Mode::ConnExportModal => Some(Box::new(ConnExportForm::default())),
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
                    // A typed password is the rest of a connection attempt,
                    // so the prompt waits for the same answer.
                    Action::Connect(..) | Action::PasswordEntered => {
                        Some((Awaiting::Connect, "Connecting…"))
                    }
                    Action::Export(..) => Some((Awaiting::Export, "Exporting…")),
                    Action::Import { .. } => Some((Awaiting::Import, "Importing…")),
                    Action::ExportConnections(..) => {
                        Some((Awaiting::ExportConnections, "Exporting…"))
                    }
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
                self.modal = self.modal_for(*mode);
                self.awaiting = None;
            }
            // The tree asked for a connection to be changed.
            Action::EditConnection(record) => {
                self.editing = Some(record.clone());
                if let Some(tx) = &self.command_tx {
                    tx.send(Action::ChangeMode(Mode::AddConnModal))?;
                }
            }
            // The worker cannot prompt, so it asks here.
            Action::NeedPassword { name, .. } => {
                self.asked = Some(name.clone());
                if let Some(tx) = &self.command_tx {
                    tx.send(Action::ChangeMode(Mode::PasswordModal))?;
                }
            }
            // The worker's answer, but only to the modal that asked for it.
            Action::Connected(_) if self.awaiting == Some(Awaiting::Connect) => {
                self.close_modal()?
            }
            Action::Exported(_) if self.awaiting == Some(Awaiting::Export) => self.close_modal()?,
            Action::ConnectFailed(message) if self.awaiting == Some(Awaiting::Connect) => {
                self.fail_modal(message);
            }
            Action::ExportFailed(message) if self.awaiting == Some(Awaiting::Export) => {
                self.fail_modal(message);
            }
            Action::Imported(_) if self.awaiting == Some(Awaiting::Import) => self.close_modal()?,
            Action::ImportFailed(message) if self.awaiting == Some(Awaiting::Import) => {
                self.fail_modal(message);
            }
            Action::ConnectionsExported(_)
                if self.awaiting == Some(Awaiting::ExportConnections) =>
            {
                self.close_modal()?
            }
            Action::ConnectionsExportFailed(message)
                if self.awaiting == Some(Awaiting::ExportConnections) =>
            {
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

    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) -> color_eyre::Result<()> {
        // The app background and default text, under everything else.
        frame.render_widget(ratatui::widgets::Block::new().style(theme.base()), area);
        let status_height = if self.statuspanel.is_empty() { 0 } else { 3 };

        let [all, statuspanel] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(status_height)]).areas(area);
        let [connpanel, rest] =
            Layout::horizontal([Constraint::Percentage(25), Constraint::Percentage(75)]).areas(all);
        let [querypane, resultspane] =
            Layout::vertical([Constraint::Percentage(40), Constraint::Fill(1)]).areas(rest);

        self.conntree.draw(frame, connpanel, theme)?;
        self.editor.draw(frame, querypane, theme)?;
        self.results.draw(frame, resultspane, theme)?;
        self.statuspanel.draw(frame, statuspanel, theme)?;

        if let Some(modal) = &self.modal {
            modal.render(frame, area, theme);
        }
        if self.helpvisible {
            self.help.draw(frame, area, theme)?;
        }
        Ok(())
    }

    fn set_focus(&mut self, focus: bool) {
        let _ = focus;
    }
}

#[cfg(test)]
#[path = "home_tests.rs"]
mod home_tests;
