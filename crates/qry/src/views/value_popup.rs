//! The value popup: one cell's whole value in a teal dialog, as wrapped
//! text, pretty JSON in the editor's colours, a hex dump or `NULL`, with
//! its keys and the scroll position in the bottom border as the help
//! overlay has them.

use qry_core::inspect::{JsonTok, Rendered};
use ratatui::{prelude::*, widgets::Paragraph};

use super::{help::truncate, layout::centered_rect, overlay::dialog_frame};
use crate::{
    app::state::ValuePopup,
    theme::{Accent, Theme},
};

/// The popup's largest size.
const MAX_SIZE: (u16, u16) = (120, 40);

/// 80% of the screen, at most [`MAX_SIZE`], centred.
pub(crate) fn popup_rect(area: Rect) -> Rect {
    let width = (area.width * 4 / 5).min(MAX_SIZE.0);
    let height = (area.height * 4 / 5).min(MAX_SIZE.1);
    centered_rect(area, width, height)
}

/// The width and height the value is laid out in on a terminal of `size`:
/// inside the border, one cell in from each side.
pub(crate) fn content_size(size: (u16, u16)) -> (u16, u16) {
    let rect = popup_rect(Rect::new(0, 0, size.0, size.1));
    (rect.width.saturating_sub(4), rect.height.saturating_sub(2))
}

/// The cells of `text` from display column `skip`, at most `width` of them.
/// A wide character cut in half by either edge is left out.
fn cut(text: &str, skip: usize, width: usize) -> String {
    let mut out = String::new();
    let mut at = 0;
    for c in text.chars() {
        let w = qry_core::grid::width(c.encode_utf8(&mut [0; 4]));
        if at >= skip && at + w <= skip + width {
            out.push(c);
        }
        at += w;
        if at >= skip + width {
            break;
        }
    }
    out
}

/// One JSON line, its tokens coloured and cut to the view.
fn json_line(
    tokens: &[(JsonTok, String)],
    skip: usize,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    let mut spans = Vec::new();
    let mut at = 0;
    for (tok, text) in tokens {
        let w = qry_core::grid::width(text);
        let from = skip.saturating_sub(at);
        let room = (skip + width).saturating_sub(at.max(skip));
        if at + w > skip && room > 0 {
            let style = match tok {
                JsonTok::Key => theme.sql_identifier(),
                JsonTok::String => theme.sql_string(),
                JsonTok::Number => theme.sql_number(),
                JsonTok::Literal => theme.sql_keyword(),
                JsonTok::Punct => theme.sql_punct(),
            };
            spans.push(Span::styled(cut(text, from, room), style));
        }
        at += w;
    }
    Line::from(spans)
}

pub(crate) fn draw(popup: &ValuePopup, theme: &Theme, frame: &mut Frame, area: Rect) {
    let rect = popup_rect(area);
    if rect.width < 8 || rect.height < 3 {
        return;
    }
    let title = truncate(&popup.title, usize::from(rect.width.saturating_sub(6)));
    let inner = dialog_frame(frame, theme, rect, &title, Accent::Info);
    let content = inner.inner(Margin::new(1, 0));
    let (width, height) = (usize::from(content.width), usize::from(content.height));
    let (scroll, skip) = (popup.scroll, popup.hscroll);

    let lines: Vec<Line> = match &popup.rendered {
        Rendered::Null => {
            let at = Rect::new(content.x, content.y + content.height / 2, content.width, 1);
            frame.render_widget(
                Paragraph::new(Span::styled("NULL", theme.null_value())).centered(),
                at,
            );
            Vec::new()
        }
        Rendered::Text(lines) => lines
            .iter()
            .skip(scroll)
            .take(height)
            .map(|l| Line::raw(cut(l, skip, width)))
            .collect(),
        Rendered::Hex(lines) => lines
            .iter()
            .skip(scroll)
            .take(height)
            .map(|l| Line::styled(cut(l, skip, width), theme.muted()))
            .collect(),
        Rendered::Json(lines) => lines
            .iter()
            .skip(scroll)
            .take(height)
            .map(|l| json_line(l, skip, width, theme))
            .collect(),
    };
    frame.render_widget(Paragraph::new(lines), content);

    // The keys on the bottom border, the position at its right.
    let border = rect.bottom() - 1;
    let mut keys = vec![("y", "copy"), ("w", "wrap")];
    let total = popup.rendered.len();
    let scrollable = total > height;
    if scrollable {
        keys.push(("j/k", "scroll"));
    }
    if !popup.wrap || matches!(popup.rendered, Rendered::Json(_) | Rendered::Hex(_)) {
        keys.push(("h/l", "pan"));
    }
    if popup.json {
        keys.push(("f", "format"));
    }
    keys.push(("Esc", "close"));
    let mut spans = vec![Span::raw(" ")];
    for (i, (key, label)) in keys.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", theme.hint()));
        }
        spans.push(Span::styled(*key, theme.hint_key()));
        spans.push(Span::styled(format!(" {label}"), theme.hint()));
    }
    spans.push(Span::raw(" "));
    let keys_width = u16::try_from(spans.iter().map(Span::width).sum::<usize>()).unwrap_or(0);
    let mut room = rect.width.saturating_sub(4);
    if scrollable {
        let last = (scroll + height).min(total);
        let text = format!(" {}–{last} of {total} ", scroll + 1);
        let w = u16::try_from(Span::raw(text.as_str()).width()).unwrap_or(0);
        if w + 4 <= rect.width {
            let at = Rect::new(rect.right() - 2 - w, border, w, 1);
            frame.render_widget(Span::styled(text, theme.hint()), at);
            room = room.saturating_sub(w + 1);
        }
    }
    if keys_width <= room {
        frame.render_widget(
            Line::from(spans),
            Rect::new(rect.x + 2, border, keys_width, 1),
        );
    }
}

#[cfg(test)]
#[path = "value_popup_tests.rs"]
mod value_popup_tests;
