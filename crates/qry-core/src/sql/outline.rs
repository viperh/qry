//! An editor buffer's tokens and statements, kept up to date edit by
//! edit: after a change only the text around it is lexed and split again.

use std::ops::Range;

use super::{
    Dialect,
    lex::{Token, lex, lex_from},
    split::{Boundary, span_at, split_from},
};

/// The text, its tokens as [`lex`] gives them and its statements' spans as
/// [`split`](fn@super::split) gives them, for one dialect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outline {
    dialect: Dialect,
    text: String,
    tokens: Vec<Token>,
    statements: Vec<Range<usize>>,
    /// Where each run of the splitter may start again, in order.
    boundaries: Vec<Boundary>,
}

impl Default for Outline {
    fn default() -> Self {
        Self::new(Dialect::Postgres, String::new())
    }
}

impl Outline {
    pub fn new(dialect: Dialect, text: String) -> Self {
        let tokens = lex(dialect, &text);
        let mut statements = Vec::new();
        let mut boundaries = Vec::new();
        split_from(
            dialect,
            &text,
            &tokens,
            Boundary::start(),
            &mut statements,
            &mut boundaries,
            |_| false,
        );
        Self {
            dialect,
            text,
            tokens,
            statements,
            boundaries,
        }
    }

    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Every token, in order.
    pub fn tokens(&self) -> &[Token] {
        &self.tokens
    }

    /// The tokens that `range` (byte offsets) overlaps, found by binary
    /// search; their spans are not clipped.
    pub fn tokens_in(&self, range: Range<usize>) -> &[Token] {
        let first = self.tokens.partition_point(|t| t.span.end <= range.start);
        let len = self.tokens[first..].partition_point(|t| t.span.start < range.end);
        &self.tokens[first..first + len]
    }

    /// The token holding byte `offset`, if the text is that long.
    pub fn token_at(&self, offset: usize) -> Option<&Token> {
        let at = self.tokens.partition_point(|t| t.span.end <= offset);
        self.tokens.get(at)
    }

    /// The statements' spans, which leave out the terminators and the
    /// trivia around them.
    pub fn statements(&self) -> &[Range<usize>] {
        &self.statements
    }

    /// [`statement_at`](super::statement_at) over these statements.
    pub fn statement_at(&self, offset: usize) -> Option<usize> {
        span_at(self.statements.iter(), offset)
    }

    /// Takes on `text` in `dialect`. Tokens and statements before the line
    /// of the first change are kept, and so are those after it once the
    /// lexer and the splitter are back in step with the old text; only
    /// the stretch between is done again.
    pub fn update(&mut self, dialect: Dialect, text: String) {
        if dialect != self.dialect {
            *self = Self::new(dialect, text);
            return;
        }
        let old = self.text.as_bytes();
        let new = text.as_bytes();
        // The bytes before and after the change, cut back to characters.
        let mut prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
        while !text.is_char_boundary(prefix) {
            prefix -= 1;
        }
        if prefix == old.len() && prefix == new.len() {
            return;
        }
        let most = old.len().min(new.len()) - prefix;
        let mut suffix = old
            .iter()
            .rev()
            .zip(new.iter().rev())
            .take(most)
            .take_while(|(a, b)| a == b)
            .count();
        while !text.is_char_boundary(new.len() - suffix) {
            suffix -= 1;
        }
        let changed_end = new.len() - suffix;
        let shift = |at: usize| at + new.len() - old.len();
        let unshift = |at: usize| at + old.len() - new.len();

        // Lexing starts again with the token holding the line break before
        // the changed line: a token wholly before that break never looked
        // past it.
        let line = self.text[..prefix].rfind('\n');
        let keep = line.map_or(0, |n| self.tokens.partition_point(|t| t.span.end <= n));
        let restart = self.tokens.get(keep).map_or(0, |t| t.span.start);
        let mut tokens = self.tokens[..keep].to_vec();
        // Lexing from where an old token started, in the unchanged text at
        // the end, gives the old tokens from there.
        let mut back_in_step = text.len();
        for token in lex_from(dialect, &text, restart) {
            if token.span.start >= changed_end
                && let Ok(i) = self
                    .tokens
                    .binary_search_by_key(&unshift(token.span.start), |t| t.span.start)
            {
                back_in_step = token.span.start;
                tokens.extend(self.tokens[i..].iter().map(|t| Token {
                    kind: t.kind,
                    span: shift(t.span.start)..shift(t.span.end),
                }));
                break;
            }
            tokens.push(token);
        }

        // Splitting starts again at the last boundary before the lexing
        // did; it is back in step at an old boundary, with the same
        // terminator, past both the change and the new tokens.
        let from = self.boundaries.partition_point(|b| b.at <= restart) - 1;
        let from_at = self.boundaries[from].at;
        let mut statements: Vec<Range<usize>> = self
            .statements
            .iter()
            .take_while(|s| s.start < from_at)
            .cloned()
            .collect();
        let mut boundaries = self.boundaries[..from].to_vec();
        let old_boundaries = &self.boundaries;
        let found = split_from(
            dialect,
            &text,
            &tokens,
            self.boundaries[from].clone(),
            &mut statements,
            &mut boundaries,
            |b| {
                b.at > changed_end
                    && b.at >= back_in_step
                    && old_boundaries
                        .binary_search_by_key(&unshift(b.at), |old| old.at)
                        .is_ok_and(|i| old_boundaries[i].delimiter == b.delimiter)
            },
        );
        if let Some(found) = found {
            let old_at = unshift(found.at);
            let first = old_boundaries.partition_point(|b| b.at < old_at);
            boundaries.extend(old_boundaries[first..].iter().map(|b| Boundary {
                at: shift(b.at),
                delimiter: b.delimiter.clone(),
            }));
            let first = self.statements.partition_point(|s| s.start < old_at);
            statements.extend(
                self.statements[first..]
                    .iter()
                    .map(|s| shift(s.start)..shift(s.end)),
            );
        }
        *self = Self {
            dialect,
            text,
            tokens,
            statements,
            boundaries,
        };
    }
}
