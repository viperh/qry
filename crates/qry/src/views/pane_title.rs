//! The `LABEL  detail` row over each pane. Focus shows here: the label is
//! bold amber on the focused pane and dim on the others.

use ratatui::{prelude::*, widgets::Paragraph};

use crate::theme::Theme;

pub(crate) fn draw(
    label: &str,
    detail: &str,
    focused: bool,
    theme: &Theme,
    frame: &mut Frame,
    area: Rect,
) {
    let label_style = if focused {
        theme.pane_title_focused()
    } else {
        theme.pane_title()
    };
    let mut spans = vec![Span::styled(format!(" {label}"), label_style)];
    if !detail.is_empty() {
        spans.push(Span::styled(format!("  {detail}"), theme.pane_title()));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(theme.pane_title()),
        area,
    );
}
