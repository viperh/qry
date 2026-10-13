//! The query editor, drawn from the text area's lines rather than by the
//! text area itself: SQL highlighting from qry-core's lexer, a gutter with
//! line numbers and a `▌` on the statement under the cursor (coral on one
//! whose last run failed), matching brackets, the selection and a block
//! cursor while the editor has focus.

use std::ops::Range;

use qry_core::sql::{Token, TokenKind, lex, split, statement_at};
use ratatui::prelude::*;

use crate::{
    app::state::{Focus, State},
    theme::Theme,
};

/// A tab is drawn as this many spaces.
const TAB: usize = 4;

/// `ln 4, col 12 · 3 statements` for the title row, counted from 1.
pub(crate) fn position(state: &State) -> String {
    let session = state.sessions.active();
    let editor = &session.editor;
    let cursor = editor.textarea.cursor();
    let statements = split(session.dialect(), &editor.query()).len();
    let mut text = format!("ln {}, col {}", cursor.0 + 1, cursor.1 + 1);
    if statements > 1 {
        text.push_str(&format!(" · {statements} statements"));
    }
    text
}

fn token_style(kind: TokenKind, theme: &Theme) -> Style {
    match kind {
        TokenKind::Keyword => theme.sql_keyword(),
        TokenKind::Ident => theme.sql_identifier(),
        TokenKind::QuotedIdent => theme.sql_quoted_ident(),
        TokenKind::String => theme.sql_string(),
        TokenKind::Number => theme.sql_number(),
        TokenKind::Comment => theme.sql_comment(),
        TokenKind::Operator => theme.sql_operator(),
        TokenKind::Punct | TokenKind::Delimiter => theme.sql_punct(),
        TokenKind::Whitespace => Style::new(),
        TokenKind::Unterminated => theme.sql_error(),
    }
}

/// The bracket at or just before `offset` and the one that matches it, as
/// byte offsets; the second is `None` when nothing matches.
fn brackets(text: &str, tokens: &[Token], offset: usize) -> Option<(usize, Option<usize>)> {
    let parens: Vec<(usize, char)> = tokens
        .iter()
        .filter(|t| t.kind == TokenKind::Punct)
        .filter_map(|t| match &text[t.span.clone()] {
            "(" => Some((t.span.start, '(')),
            ")" => Some((t.span.start, ')')),
            _ => None,
        })
        .collect();
    let at = parens
        .iter()
        .position(|&(o, _)| o == offset)
        .or_else(|| parens.iter().position(|&(o, _)| o + 1 == offset))?;
    let (start, c) = parens[at];
    let mut depth = 0i32;
    let found = if c == '(' {
        parens[at..].iter().find(|&&(_, p)| {
            depth += if p == '(' { 1 } else { -1 };
            depth == 0
        })
    } else {
        parens[..=at].iter().rev().find(|&&(_, p)| {
            depth += if p == ')' { 1 } else { -1 };
            depth == 0
        })
    };
    Some((start, found.map(|&(o, _)| o)))
}

/// How wide `c` is drawn.
fn cells(c: char) -> usize {
    if c == '\t' {
        TAB
    } else {
        Span::raw(c.to_string()).width()
    }
}

pub(crate) fn draw(state: &State, theme: &Theme, frame: &mut Frame, area: Rect) {
    if area.is_empty() {
        return;
    }
    let session = state.sessions.active();
    let editor = &session.editor;
    let focused = state.focus == Focus::Editor;
    let lines = editor.textarea.lines();
    let text = editor.query();
    let dialect = session.dialect();
    let tokens = lex(dialect, &text);
    let cursor = editor.textarea.cursor();
    let cursor_offset = editor.cursor_offset();

    // Which rows get a marker, and what is underlined or highlighted.
    let statements = split(dialect, &text);
    let current: Option<Range<usize>> = statement_at(&statements, cursor_offset).map(|i| {
        let span = &statements[i].span;
        editor.position_of(span.start).0..editor.position_of(span.end).0 + 1
    });
    let failed = session.failed.as_ref();
    let failed_rows =
        failed.map(|f| editor.position_of(f.span.start).0..editor.position_of(f.span.end).0 + 1);
    let error_span: Option<Range<usize>> = failed.and_then(|f| f.position).and_then(|p| {
        tokens
            .iter()
            .find(|t| t.span.contains(&p) && t.kind != TokenKind::Whitespace)
            .map(|t| t.span.clone())
    });
    let bracket = focused
        .then(|| brackets(&text, &tokens, cursor_offset))
        .flatten();
    let selection = editor
        .textarea
        .selection_range()
        .map(|(a, b)| editor.offset_of(a)..editor.offset_of(b));

    let digits = lines.len().to_string().len().max(2);
    let gutter = u16::try_from(digits + 4)
        .unwrap_or(u16::MAX)
        .min(area.width);
    let text_width = usize::from(area.width - gutter);
    let height = usize::from(area.height);
    editor.page.set(height.max(1));

    // Scroll as little as keeps the cursor in view.
    let (mut top, mut left) = editor.scroll.get();
    if cursor.0 < top {
        top = cursor.0;
    } else if cursor.0 >= top + height {
        top = cursor.0 + 1 - height;
    }
    let cursor_x: usize = lines
        .get(cursor.0)
        .map_or(0, |l| l.chars().take(cursor.1).map(cells).sum());
    if cursor_x < left {
        left = cursor_x;
    } else if text_width > 0 && cursor_x >= left + text_width {
        left = cursor_x + 1 - text_width;
    }
    editor.scroll.set((top, left));

    let mut token = 0;
    let mut line_start: usize = lines.iter().take(top).map(|l| l.len() + 1).sum();
    for (screen_row, row) in (top..lines.len()).take(height).enumerate() {
        let line = &lines[row];
        let y = area.y + u16::try_from(screen_row).unwrap_or(0);

        // The gutter: the marker, the number, the rule.
        let marker = if failed_rows.as_ref().is_some_and(|r| r.contains(&row)) {
            Span::styled("▌", theme.value_error())
        } else if current.as_ref().is_some_and(|r| r.contains(&row)) {
            Span::styled("▌", theme.gutter_selected())
        } else {
            Span::raw(" ")
        };
        let number_style = if row == cursor.0 {
            theme.line_number_current()
        } else {
            theme.line_number()
        };
        let gutter_line = Line::from(vec![
            marker,
            Span::styled(format!("{:>digits$} ", row + 1), number_style),
            Span::styled("│", theme.border()),
            Span::raw(" "),
        ]);
        frame.render_widget(gutter_line, Rect::new(area.x, y, gutter, 1));

        // The text, a cell at a time.
        let mut spans = Vec::new();
        let mut x = 0;
        let mut chars = line.char_indices().peekable();
        loop {
            let (byte, c) = match chars.next() {
                Some((byte, c)) => (line_start + byte, c),
                // One cell past the end, for the cursor.
                None => (line_start + line.len(), ' '),
            };
            let at_end = byte == line_start + line.len();
            while token < tokens.len() && tokens[token].span.end <= byte {
                token += 1;
            }
            let mut style = if at_end {
                Style::new()
            } else {
                tokens
                    .get(token)
                    .map_or(Style::new(), |t| token_style(t.kind, theme))
            };
            if error_span.as_ref().is_some_and(|e| e.contains(&byte)) {
                style = style.patch(theme.sql_error());
            }
            if let Some((a, b)) = bracket
                && (byte == a || Some(byte) == b)
            {
                style = style.patch(if b.is_some() {
                    theme.hint_key()
                } else {
                    theme.value_error()
                });
            }
            if selection.as_ref().is_some_and(|s| s.contains(&byte)) {
                style = style.patch(theme.editor_selection());
            }
            let is_cursor = focused && row == cursor.0 && byte == cursor_offset;
            if is_cursor {
                style = style.patch(theme.editor_cursor());
            }
            let width = cells(c);
            let shown = if c == '\t' {
                " ".repeat(TAB)
            } else {
                c.to_string()
            };
            if x + width > left && x < left + text_width && (!at_end || is_cursor) {
                spans.push(Span::styled(shown, style));
            }
            x += width;
            if at_end || x >= left + text_width {
                break;
            }
        }
        frame.render_widget(
            Line::from(spans),
            Rect::new(area.x + gutter, y, area.width - gutter, 1),
        );
        line_start += line.len() + 1;
    }
}

#[cfg(test)]
#[path = "editor_tests.rs"]
mod editor_tests;
