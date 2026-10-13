use ratatui::{
    layout::Flex,
    prelude::*,
    widgets::{Block, Clear, Padding, Paragraph},
};

use crate::{
    action::Action,
    app::Mode,
    config::Config,
    keymap::{HelpCommand, display_key},
};

/// The help popup. Built from the config, so it always lists the keys that
/// are actually bound, including a user's own.
#[derive(Default)]
pub struct Help {
    /// `(title, [(keys, description)])`, in display order.
    sections: Vec<(String, Vec<(String, String)>)>,
    /// First line in view.
    scroll: usize,
    /// Lines that fit, and the largest useful `scroll`, from the last draw.
    page: usize,
    max_scroll: usize,
}

impl Help {
    pub fn from_config(config: &Config) -> Self {
        let panes = &config.panes;
        let sections = [
            ("Global", mode_bindings(config, Mode::Home)),
            ("Connection tree", pane_bindings(panes.tree.describe())),
            ("Editor", pane_bindings(panes.editor.describe())),
            ("Results", pane_bindings(panes.results.describe())),
            // Both modals share these keys, so they are listed once.
            ("Forms", pane_bindings(panes.form.describe())),
            ("New Connection", mode_bindings(config, Mode::AddConnModal)),
            ("Export", mode_bindings(config, Mode::ExpoModal)),
            ("Password", mode_bindings(config, Mode::PasswordModal)),
            ("Import", mode_bindings(config, Mode::ImportModal)),
            (
                "Export Connections",
                mode_bindings(config, Mode::ConnExportModal),
            ),
            ("Help", pane_bindings(panes.help.describe())),
        ];
        Self {
            sections: sections
                .into_iter()
                .filter(|(_, binds)| !binds.is_empty())
                .map(|(title, binds)| (title.to_string(), binds))
                .collect(),
            ..Self::default()
        }
    }

    pub fn scroll_to_top(&mut self) {
        self.scroll = 0;
    }

    pub fn scroll(&mut self, command: HelpCommand) {
        let page = self.page.max(1);
        self.scroll = match command {
            HelpCommand::ScrollUp => self.scroll.saturating_sub(1),
            HelpCommand::ScrollDown => self.scroll + 1,
            HelpCommand::PageUp => self.scroll.saturating_sub(page),
            HelpCommand::PageDown => self.scroll + page,
            HelpCommand::Close => self.scroll,
        }
        .min(self.max_scroll);
    }

    fn lines(&self) -> Vec<Line<'static>> {
        let key_width = self
            .sections
            .iter()
            .flat_map(|(_, binds)| binds)
            .map(|(keys, _)| keys.chars().count())
            .max()
            .unwrap_or(0);

        let mut lines = Vec::new();
        for (i, (title, binds)) in self.sections.iter().enumerate() {
            if i > 0 {
                lines.push(Line::default());
            }
            lines.push(Line::from(format!("{title}:")).bold());
            for (keys, desc) in binds {
                lines.push(Line::from(format!("{keys:<key_width$} : {desc}")));
            }
        }
        lines
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect) -> color_eyre::Result<()> {
        let lines = self.lines();
        let total = lines.len();

        let width = lines.iter().map(|l| l.width()).max().unwrap_or(0) as u16 + 4;
        let height = total as u16 + 2;

        let [popup] = Layout::vertical([Constraint::Length(height.min(area.height))])
            .flex(Flex::Center)
            .areas(area);
        let [popup] = Layout::horizontal([Constraint::Length(width.min(area.width))])
            .flex(Flex::Center)
            .areas(popup);

        // Taller than the screen: remember what fits so the keys can scroll.
        self.page = usize::from(popup.height.saturating_sub(2));
        self.max_scroll = total.saturating_sub(self.page);
        self.scroll = self.scroll.min(self.max_scroll);

        let mut block = Block::bordered()
            .title(" Help ")
            .padding(Padding::horizontal(1));
        if self.max_scroll > 0 {
            let last = (self.scroll + self.page).min(total);
            block = block.title_bottom(
                Line::from(format!(" {}-{last} of {total} ", self.scroll + 1)).right_aligned(),
            );
        }

        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(lines)
                .block(block)
                .scroll((u16::try_from(self.scroll).unwrap_or(u16::MAX), 0)),
            popup,
        );
        Ok(())
    }
}

fn pane_bindings(binds: Vec<(String, &'static str)>) -> Vec<(String, String)> {
    binds
        .into_iter()
        .map(|(keys, desc)| (keys, desc.to_string()))
        .collect()
}

/// A mode's global bindings as help lines, keys for the same action on one
/// line, sorted by description.
fn mode_bindings(config: &Config, mode: Mode) -> Vec<(String, String)> {
    let Some(bindings) = config.keybindings.0.get(&mode) else {
        return Vec::new();
    };
    let mut by_action: Vec<(String, Vec<String>)> = Vec::new();
    for (sequence, action) in bindings {
        let desc = describe_action(action);
        let keys = sequence
            .iter()
            .map(display_key)
            .collect::<Vec<_>>()
            .join(" ");
        match by_action.iter_mut().find(|(d, _)| *d == desc) {
            Some((_, all)) => all.push(keys),
            None => by_action.push((desc, vec![keys])),
        }
    }
    by_action.sort();
    by_action
        .into_iter()
        .map(|(desc, mut keys)| {
            keys.sort_by_key(|k| (k.len() == 1, k.clone()));
            (keys.join(" / "), desc)
        })
        .collect()
}

fn describe_action(action: &Action) -> String {
    match action {
        Action::Quit => "Quit".into(),
        Action::Help => "Show / hide this help".into(),
        Action::FocusNext => "Next pane".into(),
        Action::FocusPrev => "Previous pane".into(),
        Action::ChangeMode(Mode::AddConnModal) => "New connection".into(),
        Action::ChangeMode(Mode::Home) => "Close".into(),
        Action::ChangeMode(Mode::ExpoModal) => "Export to file".into(),
        Action::ChangeMode(Mode::PasswordModal) => "Ask for a password".into(),
        Action::ChangeMode(Mode::ImportModal) => "Import connections".into(),
        Action::ChangeMode(Mode::ConnExportModal) => "Export connections".into(),
        Action::Suspend => "Suspend".into(),
        Action::ClearScreen => "Redraw the screen".into(),
        Action::Execute(sql) => format!("Run `{sql}`"),
        other => other.to_string(),
    }
}

#[cfg(test)]
#[path = "help_tests.rs"]
mod help_tests;
