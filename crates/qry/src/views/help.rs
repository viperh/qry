//! The help popup.

use ratatui::{
    layout::Flex,
    prelude::*,
    widgets::{Block, Clear, Padding, Paragraph},
};

use crate::{app::state::HelpState, theme::Theme};

/// Group titles in the help's teal accent, keys in amber.
fn lines(help: &HelpState, theme: &Theme) -> Vec<Line<'static>> {
    let key_width = help
        .sections
        .iter()
        .flat_map(|(_, binds)| binds)
        .map(|(keys, _)| keys.chars().count())
        .max()
        .unwrap_or(0);

    let mut lines = Vec::new();
    for (i, (title, binds)) in help.sections.iter().enumerate() {
        if i > 0 {
            lines.push(Line::default());
        }
        lines.push(Line::styled(
            format!("{title}:"),
            theme.dialog_heading(theme.teal),
        ));
        for (keys, desc) in binds {
            lines.push(Line::from(vec![
                Span::styled(format!("{keys:<key_width$}"), theme.hint_key()),
                Span::raw(format!(" : {desc}")),
            ]));
        }
    }
    lines
}

pub(crate) fn draw(help: &HelpState, theme: &Theme, frame: &mut Frame, area: Rect) {
    let lines = lines(help, theme);
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
    let page = usize::from(popup.height.saturating_sub(2));
    let max_scroll = total.saturating_sub(page);
    let scroll = help.scroll.get().min(max_scroll);
    help.page.set(page);
    help.max_scroll.set(max_scroll);
    help.scroll.set(scroll);

    let mut block = Block::bordered()
        .title(Line::styled(" Help ", theme.dialog_title(theme.teal)))
        .style(theme.dialog())
        .border_style(theme.dialog_border(theme.teal))
        .padding(Padding::horizontal(1));
    if max_scroll > 0 {
        let last = (scroll + page).min(total);
        block = block.title_bottom(
            Line::styled(format!(" {}-{last} of {total} ", scroll + 1), theme.hint())
                .right_aligned(),
        );
    }

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .scroll((u16::try_from(scroll).unwrap_or(u16::MAX), 0)),
        popup,
    );
}
