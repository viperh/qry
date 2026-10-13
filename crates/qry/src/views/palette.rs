//! The command palette, after tachy's `palette.rs`: a purple frame with no
//! title near the top of the screen, the query, and the matching commands
//! with their category and key.

use ratatui::{
    prelude::*,
    widgets::{Block, Clear},
};

use crate::{
    palette::{MAX_ROWS, PaletteState},
    theme::Theme,
};

/// How wide the palette is, at most.
const WIDTH: u16 = 96;
/// The top row of the palette.
const TOP: u16 = 3;
/// The tallest it gets: the query, the rows, the border.
const MAX_HEIGHT: u16 = 20;
/// The key column's width.
const KEY_WIDTH: usize = 14;

/// Where the palette goes in `area` with `rows` items to show: 96 wide (or
/// 8 narrower than the screen), centred, from row 3.
pub(crate) fn palette_rect(area: Rect, rows: usize) -> Rect {
    let width = if area.width >= WIDTH + 8 {
        WIDTH
    } else {
        area.width.saturating_sub(8)
    };
    let rows = u16::try_from(rows.min(MAX_ROWS)).unwrap_or(0);
    let height = (rows + 3)
        .min(MAX_HEIGHT)
        .min(area.height.saturating_sub(TOP));
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + TOP.min(area.height),
        width,
        height,
    )
}

/// `text` padded or cut to `width` display cells.
fn fit(text: &str, width: usize) -> String {
    let cut = super::help::truncate(text, width);
    let pad = width.saturating_sub(Span::raw(cut.as_str()).width());
    format!("{cut}{}", " ".repeat(pad))
}

pub(crate) fn draw(palette: &PaletteState, theme: &Theme, frame: &mut Frame, area: Rect) {
    let rect = palette_rect(area, palette.matches.len());
    if rect.width < 10 || rect.height < 3 {
        return;
    }
    let block = Block::bordered()
        .style(theme.dialog())
        .border_style(theme.palette_border());
    let inner = block.inner(rect).inner(Margin::new(1, 0));
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);

    // The query, with a block cursor, and the counter or the error.
    let counter = match &palette.error {
        Some(error) => Span::styled(error.clone(), theme.value_error()),
        None => Span::styled(
            format!("{} / {}", palette.matches.len(), palette.items.len()),
            theme.dim(),
        ),
    };
    let query: Vec<char> = palette.input.value.chars().collect();
    let cursor = palette.input.cursor.min(query.len());
    let before: String = query[..cursor].iter().collect();
    let under = query.get(cursor).copied().unwrap_or(' ');
    let after: String = query.iter().skip(cursor + 1).collect();
    let prompt = Line::from(vec![
        Span::styled("› ", theme.field_focused()),
        Span::styled(before, theme.field()),
        Span::styled(under.to_string(), theme.cursor_cell()),
        Span::styled(after, theme.field()),
    ]);
    let row = Rect::new(inner.x, inner.y, inner.width, 1);
    frame.render_widget(prompt, row);
    let counter_width = u16::try_from(counter.width()).unwrap_or(0).min(inner.width);
    frame.render_widget(
        counter,
        Rect::new(inner.right() - counter_width, inner.y, counter_width, 1),
    );

    // The rows, scrolled to keep the selection in view.
    let rows = usize::from(inner.height.saturating_sub(1));
    if rows == 0 {
        return;
    }
    let mut offset = palette.offset.get();
    if palette.selected < offset {
        offset = palette.selected;
    } else if palette.selected >= offset + rows {
        offset = palette.selected + 1 - rows;
    }
    palette.offset.set(offset);

    let width = usize::from(inner.width);
    let key_width = KEY_WIDTH.min(width / 4);
    let label_width = (width.saturating_sub(key_width) * 3 / 5).max(1);
    let category_width = width.saturating_sub(label_width + key_width);
    for (r, (index, m)) in palette.matches.iter().enumerate().skip(offset).take(rows) {
        let item = &palette.items[*index];
        let selected = r == palette.selected;
        let base = if selected {
            theme.palette_item_selected()
        } else {
            theme.dialog()
        };
        let mut spans = Vec::new();
        let label = fit(&item.label, label_width);
        for (i, c) in label.chars().enumerate() {
            let style = if m.positions.contains(&i) {
                base.patch(theme.palette_match())
            } else {
                base
            };
            spans.push(Span::styled(c.to_string(), style));
        }
        spans.push(Span::styled(
            fit(item.category, category_width),
            base.patch(theme.dim()),
        ));
        let key = item.key.as_deref().unwrap_or("");
        let key = super::help::truncate(key, key_width);
        let pad = key_width.saturating_sub(Span::raw(key.as_str()).width());
        spans.push(Span::styled(" ".repeat(pad), base));
        spans.push(Span::styled(key, base.patch(theme.hint_key())));
        let y = inner.y + 1 + u16::try_from(r - offset).unwrap_or(0);
        frame.render_widget(Line::from(spans), Rect::new(inner.x, y, inner.width, 1));
    }
}

#[cfg(test)]
#[path = "palette_view_tests.rs"]
mod palette_view_tests;
