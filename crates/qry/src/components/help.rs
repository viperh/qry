use std::collections::HashMap;

use ratatui::{
    layout::Flex,
    prelude::*,
    widgets::{Block, Clear, Padding, Paragraph},
};


pub struct Help {
    info: HashMap<String, HashMap<String, String>>,
}

/// Every key qry reacts to, by pane. Hard-coded, so keep it in step with
/// `.config/config.json` and each component's `handle_key_event`.
impl Default for Help {
    fn default() -> Self {
        let section = |binds: &[(&str, &str)]| {
            binds
                .iter()
                .map(|&(key, desc)| (key.to_string(), desc.to_string()))
                .collect::<HashMap<_, _>>()
        };

        let info = HashMap::from([
            (
                "Global".to_string(),
                section(&[
                    ("q / Ctrl-c", "Quit"),
                    ("Ctrl-h", "Show this help"),
                    ("Tab / Shift-Tab", "Next / previous pane"),
                    ("Ctrl-n", "New connection"),
                    ("Ctrl-e", "Export to file"),
                ]),
            ),
            (
                "Editor".to_string(),
                section(&[
                    ("F8", "Run query"),
                    ("Shift + move", "Select text"),
                    ("Ctrl-← / Ctrl-→", "Move by word"),
                    ("Ctrl-u / Ctrl-r", "Undo / redo"),
                    ("Ctrl-x / Ctrl-y", "Cut / paste"),
                    ("Ctrl-w / Ctrl-k", "Delete word / to end of line"),
                    ("PageUp / PageDown", "Scroll"),
                ]),
            ),
            (
                "Results".to_string(),
                section(&[
                    ("↑ ↓ ← → or i k j l", "Move the selected cell"),
                    ("PageUp / PageDown", "Up / down a page"),
                    ("Home / End", "First / last column"),
                    ("g / G", "First / last row"),
                ]),
            ),
            (
                "New Connection".to_string(),
                section(&[
                    ("Tab / Shift-Tab", "Next / previous field"),
                    ("← / →", "Move cursor, or change a choice"),
                    ("Space", "Next choice (Type, SSL mode)"),
                    ("Enter", "Connect"),
                    ("Esc", "Cancel"),
                ]),
            ),
        ]);

        Self { info }
    }
}

impl Help {

    pub fn draw(&mut self, frame: &mut Frame, area: Rect) -> color_eyre::Result<()> {
        let key_width = self
            .info
            .values()
            .flat_map(|b| b.keys())
            .map(|k| k.chars().count())
            .max()
            .unwrap_or(0);

        let mut categories: Vec<_> = self.info.iter().collect();
        categories.sort_by(|a, b| a.0.cmp(b.0));

        let mut lines = Vec::new();
        for (i, (category, binds)) in categories.into_iter().enumerate() {
            if i > 0 {
                lines.push(Line::default());
            }
            lines.push(Line::from(format!("{category}:")).bold());

            let mut binds: Vec<_> = binds.iter().collect();
            binds.sort_by(|a, b| a.0.cmp(b.0));
            for (key, desc) in binds {
                lines.push(Line::from(format!("{key:<key_width$} : {desc}")));
            }
        }

        let width = lines.iter().map(|l| l.width()).max().unwrap_or(0) as u16 + 4;
        let height = lines.len() as u16 + 2;

        let [popup] = Layout::vertical([Constraint::Length(height.min(area.height))])
            .flex(Flex::Center)
            .areas(area);
        let [popup] = Layout::horizontal([Constraint::Length(width.min(area.width))])
            .flex(Flex::Center)
            .areas(popup);

        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(lines).block(
                Block::bordered()
                    .title(" Help ")
                    .padding(Padding::horizontal(1)),
            ),
            popup,
        );
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn every_pane_has_its_keys() {
        let help = Help::default();
        let mut sections: Vec<_> = help.info.keys().map(String::as_str).collect();
        sections.sort();
        assert_eq!(sections, ["Editor", "Global", "New Connection", "Results"]);
        assert!(help.info.values().all(|binds| !binds.is_empty()));
    }

    #[test]
    fn popup_shows_every_binding() {
        let mut help = Help::default();
        let mut terminal = Terminal::new(TestBackend::new(80, 40)).unwrap();
        terminal.draw(|frame| help.draw(frame, frame.area()).unwrap()).unwrap();
        let buffer = terminal.backend().buffer();
        let text: String = (0..buffer.area.height)
            .map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>() + "\n")
            .collect();

        for binds in help.info.values() {
            for (key, desc) in binds {
                assert!(text.contains(key.as_str()) && text.contains(desc.as_str()), "{key}: {desc}");
            }
        }
    }
}
