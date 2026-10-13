//! The lexer: SQL text into tokens whose spans tile it exactly.

use std::ops::Range;

use super::{Dialect, keywords::is_keyword};

/// What a token is, for highlighting and splitting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    Keyword,
    Ident,
    QuotedIdent,
    String,
    Number,
    Comment,
    Operator,
    Punct,
    Whitespace,
    /// A string, quoted identifier or comment that never ends: the rest of
    /// the text.
    Unterminated,
    /// `;`.
    Delimiter,
}

/// A token and where it is, as byte offsets into the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Range<usize>,
}

struct Lexer<'a> {
    dialect: Dialect,
    src: &'a str,
    at: usize,
}

impl Lexer<'_> {
    fn rest(&self) -> &str {
        &self.src[self.at..]
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        self.rest().chars().nth(n)
    }

    /// Moves past `c`, which must be the next character.
    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.at += c.len_utf8();
        Some(c)
    }

    fn eat_while(&mut self, mut keep: impl FnMut(char) -> bool) {
        while let Some(c) = self.peek() {
            if !keep(c) {
                break;
            }
            self.bump();
        }
    }

    /// Up to the end of the line, not including it.
    fn line_comment(&mut self) -> TokenKind {
        self.eat_while(|c| c != '\n');
        TokenKind::Comment
    }

    /// After `/*`: up to the matching `*/`, nesting if the dialect does.
    fn block_comment(&mut self) -> TokenKind {
        let nests = self.dialect == Dialect::Postgres;
        let mut depth = 1;
        while let Some(c) = self.bump() {
            match c {
                '*' if self.peek() == Some('/') => {
                    self.bump();
                    depth -= 1;
                    if depth == 0 {
                        return TokenKind::Comment;
                    }
                }
                '/' if nests && self.peek() == Some('*') => {
                    self.bump();
                    depth += 1;
                }
                _ => {}
            }
        }
        TokenKind::Unterminated
    }

    /// After the opening `quote`: up to the closing one, a doubled quote
    /// standing for itself, and with backslash escapes if asked.
    fn quoted(&mut self, quote: char, backslash: bool, kind: TokenKind) -> TokenKind {
        while let Some(c) = self.bump() {
            if backslash && c == '\\' {
                self.bump();
            } else if c == quote {
                if self.peek() == Some(quote) {
                    self.bump();
                } else {
                    return kind;
                }
            }
        }
        TokenKind::Unterminated
    }

    /// After `[` in SQLite: up to `]`.
    fn bracketed(&mut self) -> TokenKind {
        while let Some(c) = self.bump() {
            if c == ']' {
                return TokenKind::QuotedIdent;
            }
        }
        TokenKind::Unterminated
    }

    /// At `$`: a Postgres dollar quote `$tag$ … $tag$`, if one starts here.
    fn dollar_quote(&mut self) -> Option<TokenKind> {
        let rest = self.rest();
        // The tag runs to the first character that cannot be in it, which
        // must be the closing `$`; a digit cannot start it (`$1`).
        let (end, close) = rest[1..]
            .char_indices()
            .find(|&(i, c)| !(c == '_' || c.is_alphanumeric()) || (i == 0 && c.is_ascii_digit()))?;
        if close != '$' {
            return None;
        }
        let tag = rest[..end + 2].to_string();
        self.at += tag.len();
        match self.rest().find(&tag) {
            Some(end) => {
                self.at += end + tag.len();
                Some(TokenKind::String)
            }
            None => {
                self.at = self.src.len();
                Some(TokenKind::Unterminated)
            }
        }
    }

    /// At `q'`: an Oracle alternative quote such as `q'[ … ]'`.
    fn q_quote(&mut self) -> Option<TokenKind> {
        let mut chars = self.rest().chars();
        chars.next();
        if chars.next() != Some('\'') {
            return None;
        }
        let open = chars.next()?;
        if open.is_whitespace() {
            return None;
        }
        let close = match open {
            '[' => ']',
            '{' => '}',
            '<' => '>',
            '(' => ')',
            c => c,
        };
        self.at += 2 + open.len_utf8();
        let ending = format!("{close}'");
        match self.rest().find(&ending) {
            Some(end) => {
                self.at += end + ending.len();
                Some(TokenKind::String)
            }
            None => {
                self.at = self.src.len();
                Some(TokenKind::Unterminated)
            }
        }
    }

    fn number(&mut self) -> TokenKind {
        if self.rest().starts_with("0x") || self.rest().starts_with("0X") {
            self.at += 2;
            self.eat_while(|c| c.is_ascii_hexdigit());
            return TokenKind::Number;
        }
        self.eat_while(|c| c.is_ascii_digit());
        if self.peek() == Some('.')
            && self
                .peek_at(1)
                .is_none_or(|c| c.is_ascii_digit() || !c.is_alphabetic())
        {
            self.bump();
            self.eat_while(|c| c.is_ascii_digit());
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            let sign = matches!(self.peek_at(1), Some('+' | '-'));
            let digit_at = if sign { 2 } else { 1 };
            if self.peek_at(digit_at).is_some_and(|c| c.is_ascii_digit()) {
                self.bump();
                if sign {
                    self.bump();
                }
                self.eat_while(|c| c.is_ascii_digit());
            }
        }
        TokenKind::Number
    }

    fn word(&mut self) -> TokenKind {
        let start = self.at;
        self.eat_while(|c| c == '_' || c == '$' || c.is_alphanumeric());
        if is_keyword(self.dialect, &self.src[start..self.at]) {
            TokenKind::Keyword
        } else {
            TokenKind::Ident
        }
    }

    fn next_token(&mut self) -> Option<Token> {
        let start = self.at;
        let c = self.peek()?;
        let mysql = self.dialect == Dialect::MySql;
        let kind = match c {
            c if c.is_whitespace() => {
                self.eat_while(char::is_whitespace);
                TokenKind::Whitespace
            }
            '-' if self.peek_at(1) == Some('-') => self.line_comment(),
            '#' if mysql => self.line_comment(),
            '/' if self.peek_at(1) == Some('*') => {
                self.at += 2;
                self.block_comment()
            }
            '\'' => {
                self.bump();
                self.quoted('\'', mysql, TokenKind::String)
            }
            'e' | 'E' if self.dialect == Dialect::Postgres && self.peek_at(1) == Some('\'') => {
                self.at += 2;
                self.quoted('\'', true, TokenKind::String)
            }
            'q' | 'Q' if self.dialect == Dialect::Oracle && self.peek_at(1) == Some('\'') => {
                match self.q_quote() {
                    Some(kind) => kind,
                    None => self.word(),
                }
            }
            '"' => {
                self.bump();
                if mysql {
                    self.quoted('"', true, TokenKind::String)
                } else {
                    self.quoted('"', false, TokenKind::QuotedIdent)
                }
            }
            '`' if matches!(self.dialect, Dialect::MySql | Dialect::Sqlite) => {
                self.bump();
                self.quoted('`', false, TokenKind::QuotedIdent)
            }
            '[' if self.dialect == Dialect::Sqlite => {
                self.bump();
                self.bracketed()
            }
            '$' if self.dialect == Dialect::Postgres => match self.dollar_quote() {
                Some(kind) => kind,
                None => {
                    // A parameter such as `$1`.
                    self.bump();
                    self.eat_while(|c| c.is_ascii_digit());
                    TokenKind::Operator
                }
            },
            '0'..='9' => self.number(),
            '.' if self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) => self.number(),
            ';' => {
                self.bump();
                TokenKind::Delimiter
            }
            '(' | ')' | ',' | '.' | '[' | ']' | '{' | '}' => {
                self.bump();
                TokenKind::Punct
            }
            c if c == '_' || c.is_alphabetic() => self.word(),
            '+' | '-' | '*' | '/' | '%' | '<' | '>' | '=' | '!' | '|' | '&' | '^' | '~' | ':'
            | '@' | '?' => {
                self.bump();
                TokenKind::Operator
            }
            _ => {
                self.bump();
                TokenKind::Punct
            }
        };
        Some(Token {
            kind,
            span: start..self.at,
        })
    }
}

/// The tokens of `src`, in order. Their spans cover every byte exactly
/// once; text that never closes (a string, a comment) is one
/// [`TokenKind::Unterminated`] token to the end.
pub fn lex(dialect: Dialect, src: &str) -> Vec<Token> {
    let mut lexer = Lexer {
        dialect,
        src,
        at: 0,
    };
    let mut tokens = Vec::new();
    while let Some(token) = lexer.next_token() {
        tokens.push(token);
    }
    tokens
}
