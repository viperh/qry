//! The help overlay, ported from tachy's `components/help.rs`: the whole
//! screen, the keymap's groups packed into one to three columns, a pinned
//! cheat-sheet at the bottom and the help's own keys in the border.

use ratatui::{prelude::*, widgets::Paragraph};

use super::{confirm::wrap, overlay::dialog_frame};
use crate::{
    app::state::HelpState,
    help_model::{FooterItem, HelpGroup, Piece},
    theme::{Accent, Theme},
};

/// Cells between columns.
pub(crate) const COLUMN_GAP: u16 = 3;

/// How many columns fit: 3 from 160 cells, 2 from 100, else 1.
pub(crate) fn column_count(width: u16) -> u16 {
    match width {
        160.. => 3,
        100.. => 2,
        _ => 1,
    }
}

/// `text` cut to `width` display cells, with `…` when anything was cut.
pub(crate) fn truncate(text: &str, width: usize) -> String {
    if Span::raw(text).width() <= width {
        return text.to_string();
    }
    let mut out = String::new();
    for c in text.chars() {
        if Span::raw(format!("{out}{c}…")).width() > width {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}

/// Groups that must stay together: one, and the continuations under it.
fn blocks(groups: &[HelpGroup]) -> Vec<Vec<&HelpGroup>> {
    let mut blocks: Vec<Vec<&HelpGroup>> = Vec::new();
    for group in groups {
        match blocks.last_mut() {
            Some(block) if group.continues => block.push(group),
            _ => blocks.push(vec![group]),
        }
    }
    blocks
}

/// Lines a group takes in a column `width` wide.
fn group_height(group: &HelpGroup, width: u16) -> usize {
    1 + group.entries.len()
        + group
            .notes
            .iter()
            .map(|note| wrap(note, width).len().max(1))
            .sum::<usize>()
}

/// Puts blocks of these heights into at most `columns` columns, in order,
/// as tachy does. The limit on a column's lines (blank lines between blocks
/// not counted) starts at an even share and grows until they fit, so the
/// tallest column is as short as it can be. A block taller than the limit
/// has a column to itself. Returns the block indexes of each column, and
/// the limit.
pub(crate) fn pack(heights: &[usize], columns: u16) -> (Vec<Vec<usize>>, usize) {
    let columns = usize::from(columns.max(1));
    let total = heights.iter().sum::<usize>();
    let mut limit = total.div_ceil(columns).max(1);
    loop {
        let mut packed: Vec<Vec<usize>> = vec![Vec::new()];
        let mut used = 0;
        for (i, &h) in heights.iter().enumerate() {
            let column = packed.last_mut().unwrap_or_else(|| unreachable!());
            if column.is_empty() {
                column.push(i);
                used = h;
            } else if used + h <= limit {
                column.push(i);
                used += h;
            } else {
                packed.push(vec![i]);
                used = h;
            }
        }
        if packed.len() <= columns {
            return (packed, limit);
        }
        limit += 1;
    }
}

/// One column's lines, its key column as wide as its widest key, capped
/// at half the column.
fn column_lines(blocks: &[&Vec<&HelpGroup>], width: u16, theme: &Theme) -> Vec<Line<'static>> {
    let widest = blocks
        .iter()
        .flat_map(|block| block.iter())
        .flat_map(|group| &group.entries)
        .map(|(keys, _)| Span::raw(keys.as_str()).width())
        .max()
        .unwrap_or(0);
    let key_width = widest.min(usize::from(width) / 2);
    let desc_width = usize::from(width).saturating_sub(key_width + 2);
    let title_style = theme.dialog_heading(theme.teal);
    let mut lines = Vec::new();
    for (b, block) in blocks.iter().enumerate() {
        if b > 0 {
            lines.push(Line::default());
        }
        for group in block.iter() {
            lines.push(Line::styled(
                truncate(group.title, usize::from(width)),
                title_style,
            ));
            for (keys, desc) in &group.entries {
                let keys = truncate(keys, key_width);
                let pad = key_width.saturating_sub(Span::raw(keys.as_str()).width());
                lines.push(Line::from(vec![
                    Span::styled(keys, theme.hint_key()),
                    Span::raw(" ".repeat(pad + 2)),
                    Span::styled(truncate(desc, desc_width), theme.hint()),
                ]));
            }
            for note in &group.notes {
                for line in wrap(note, width) {
                    lines.push(Line::styled(line, theme.hint()));
                }
            }
        }
    }
    lines
}

/// The cheat-sheet's lines, items wrapped as whole units.
fn footer_lines(items: &[FooterItem], width: u16, theme: &Theme) -> Vec<Line<'static>> {
    let title_style = theme.dialog_heading(theme.teal);
    let mut lines: Vec<Vec<Span<'static>>> = vec![Vec::new()];
    for item in items {
        let mut spans = vec![Span::styled(item.label, title_style)];
        for piece in &item.pieces {
            // Two spaces before each key, one before its words.
            spans.push(Span::raw(if matches!(piece, Piece::Key(_)) {
                "  "
            } else {
                " "
            }));
            spans.push(match piece {
                Piece::Key(key) => Span::styled(key.clone(), theme.hint_key()),
                Piece::Text(text) => Span::styled(*text, theme.hint()),
            });
        }
        let item_width: usize = spans.iter().map(Span::width).sum();
        let line = lines.last_mut().unwrap_or_else(|| unreachable!());
        let used: usize = line.iter().map(Span::width).sum();
        if !line.is_empty() && used + 2 + item_width > usize::from(width) {
            lines.push(spans);
        } else {
            if !line.is_empty() {
                line.push(Span::raw("  "));
            }
            line.extend(spans);
        }
    }
    lines
        .into_iter()
        .filter(|spans| !spans.is_empty())
        .map(Line::from)
        .collect()
}

pub(crate) fn draw(help: &HelpState, theme: &Theme, frame: &mut Frame, area: Rect) {
    if area.width < 4 || area.height < 3 {
        return;
    }
    let title = format!("Help — qry {}", env!("CARGO_PKG_VERSION"));
    let inner = dialog_frame(frame, theme, area, &title, Accent::Info);
    let content = inner.inner(Margin::new(1, 0));

    // The cheat-sheet sits at the bottom and never scrolls.
    let footer = footer_lines(&help.footer, content.width, theme);
    let footer_rows = u16::try_from(footer.len())
        .unwrap_or(0)
        .min(content.height / 2);
    let body = Rect {
        height: content
            .height
            .saturating_sub(footer_rows + u16::from(footer_rows > 0)),
        ..content
    };
    if footer_rows > 0 {
        let at = Rect::new(
            content.x,
            content.bottom() - footer_rows,
            content.width,
            footer_rows,
        );
        frame.render_widget(Paragraph::new(footer), at);
    }

    let n = column_count(area.width);
    let col_width = (body.width.saturating_sub(COLUMN_GAP * (n - 1))) / n;
    let blocks = blocks(&help.groups);
    let heights: Vec<usize> = blocks
        .iter()
        .map(|block| block.iter().map(|g| group_height(g, col_width)).sum())
        .collect();
    let (packed, _) = pack(&heights, n);
    let columns: Vec<Vec<Line>> = packed
        .iter()
        .map(|indexes| {
            let chosen: Vec<&Vec<&HelpGroup>> = indexes.iter().map(|&i| &blocks[i]).collect();
            column_lines(&chosen, col_width, theme)
        })
        .collect();
    let total = columns.iter().map(Vec::len).max().unwrap_or(0);

    let page = usize::from(body.height);
    let max_scroll = total.saturating_sub(page);
    let scroll = help.scroll.get().min(max_scroll);
    help.page.set(page.max(1));
    help.max_scroll.set(max_scroll);
    help.scroll.set(scroll);

    for (c, lines) in columns.into_iter().enumerate() {
        let x = body.x + u16::try_from(c).unwrap_or(0) * (col_width + COLUMN_GAP);
        let at = Rect::new(
            x,
            body.y,
            col_width.min(body.right().saturating_sub(x)),
            body.height,
        );
        let shown: Vec<Line> = lines.into_iter().skip(scroll).take(page).collect();
        frame.render_widget(Paragraph::new(shown), at);
    }

    // The help's own keys on the bottom border, the position at its right.
    let border = area.bottom() - 1;
    let mut spans = vec![Span::raw(" ")];
    for (i, (keys, label)) in help.keys.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", theme.hint()));
        }
        spans.push(Span::styled(keys.clone(), theme.hint_key()));
        spans.push(Span::styled(format!(" {label}"), theme.hint()));
    }
    spans.push(Span::raw(" "));
    let keys_width = u16::try_from(spans.iter().map(Span::width).sum::<usize>()).unwrap_or(0);
    let mut room = area.width.saturating_sub(4);
    if max_scroll > 0 {
        let last = (scroll + page).min(total);
        let text = format!(" {}–{last} of {total} ", scroll + 1);
        let w = u16::try_from(Span::raw(text.as_str()).width()).unwrap_or(0);
        if w + 4 <= area.width {
            let at = Rect::new(area.right() - 2 - w, border, w, 1);
            frame.render_widget(Span::styled(text, theme.hint()), at);
            room = room.saturating_sub(w + 1);
        }
    }
    if keys_width <= room {
        frame.render_widget(
            Line::from(spans),
            Rect::new(area.x + 2, border, keys_width, 1),
        );
    }
}

#[cfg(test)]
#[path = "help_tests.rs"]
mod help_tests;
