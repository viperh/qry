//! SQL text, without a database: tokens for highlighting, statements for
//! running one at a time, and which statements change the schema.
//!
//! Everything here is total: any text, however broken, lexes and splits
//! without panicking, and the token spans tile the input exactly.

mod keywords;
mod lex;
mod split;

pub use lex::{Token, TokenKind, lex};
pub use split::{Statement, is_ddl, is_read_only, split, statement_at};

/// The SQL flavour, for the quoting and comment rules that differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dialect {
    Postgres,
    MySql,
    Sqlite,
    Oracle,
}

impl Dialect {
    pub const ALL: [Dialect; 4] = [
        Dialect::Postgres,
        Dialect::MySql,
        Dialect::Sqlite,
        Dialect::Oracle,
    ];
}

#[cfg(test)]
#[path = "lex_tests.rs"]
mod lex_tests;

#[cfg(test)]
#[path = "split_tests.rs"]
mod split_tests;
