//! The splitter: SQL text into the statements a server runs one at a time.

use std::ops::Range;

use super::{
    Dialect,
    lex::{Token, TokenKind, lex},
};

/// One statement: where it is in the text, and what to send. The span
/// leaves out the terminator and any comments or blank lines around it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Statement {
    pub span: Range<usize>,
    pub text: String,
}

fn is_trivia(kind: TokenKind) -> bool {
    matches!(kind, TokenKind::Whitespace | TokenKind::Comment)
}

/// Whether a byte is code, where a terminator counts: not inside a string,
/// a quoted identifier or a comment.
fn code_mask(src: &str, tokens: &[Token]) -> Vec<bool> {
    let mut mask = vec![false; src.len()];
    for token in tokens {
        let code = !matches!(
            token.kind,
            TokenKind::String
                | TokenKind::QuotedIdent
                | TokenKind::Comment
                | TokenKind::Unterminated
        );
        if code {
            mask[token.span.clone()].fill(true);
        }
    }
    mask
}

/// The tokens in `region`, found by binary search, their spans clipped to
/// it: a custom MySQL delimiter can end a statement inside a token
/// (`end$$`).
fn within(tokens: &[Token], region: Range<usize>) -> impl Iterator<Item = Token> + '_ {
    let first = tokens.partition_point(|t| t.span.end <= region.start);
    tokens[first..]
        .iter()
        .take_while(move |t| t.span.start < region.end)
        .map(move |t| Token {
            kind: t.kind,
            span: t.span.start.max(region.start)..t.span.end.min(region.end),
        })
}

/// The upper-cased words that start `region`, skipping trivia.
fn leading_words(tokens: &[Token], src: &str, region: Range<usize>, n: usize) -> Vec<String> {
    within(tokens, region)
        .filter(|t| !is_trivia(t.kind))
        .take(n)
        .map(|t| src[t.span.clone()].to_ascii_uppercase())
        .collect()
}

/// Whether an Oracle statement starting with these words is a PL/SQL
/// block, in which `;` does not end the statement.
fn plsql_block(words: &[String]) -> bool {
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    let body = |kind: &str| {
        matches!(
            kind,
            "PROCEDURE" | "FUNCTION" | "PACKAGE" | "TRIGGER" | "TYPE"
        )
    };
    match words.as_slice() {
        ["BEGIN", ..] | ["DECLARE", ..] => true,
        ["CREATE", "OR", "REPLACE", kind, ..] | ["CREATE", kind, ..] => body(kind),
        _ => false,
    }
}

/// The statement in `region`, trimmed to its first and last tokens that
/// are not trivia; `None` when there are none.
fn statement(tokens: &[Token], src: &str, region: Range<usize>) -> Option<Statement> {
    let inside = within(tokens, region).filter(|t| !is_trivia(t.kind));
    let mut first = None;
    let mut last = None;
    for t in inside {
        first.get_or_insert(t.span.start);
        // An unterminated string or comment runs to the end, blank lines
        // and all; the statement does not.
        let text = &src[t.span.clone()];
        last = Some(if t.kind == TokenKind::Unterminated {
            t.span.start + text.trim_end().len()
        } else {
            t.span.end
        });
    }
    let span = first?..last?;
    Some(Statement {
        text: src[span.clone()].to_string(),
        span,
    })
}

/// The statements of `src`, in order. `;` outside strings and comments
/// ends one. In Oracle a line holding only `/` ends one too, and inside a
/// PL/SQL block (`BEGIN`, `DECLARE`, `CREATE … PROCEDURE` and the like)
/// only that line does. In MySQL a `DELIMITER x` line changes the
/// terminator and is not part of any statement. Empty statements and ones
/// of only comments are left out.
pub fn split(dialect: Dialect, src: &str) -> Vec<Statement> {
    let tokens = lex(dialect, src);
    let code = code_mask(src, &tokens);
    let mut statements = Vec::new();
    let mut delimiter = ";".to_string();
    let mut begin = 0;
    let mut at = 0;
    let line_start = |i: usize| i == 0 || src.as_bytes()[i - 1] == b'\n';
    while at < src.len() {
        if line_start(at) {
            let end = src[at..].find('\n').map_or(src.len(), |n| at + n);
            let line = &src[at..end];
            let lead = line.len() - line.trim_start().len();
            let next = (end + 1).min(src.len());
            let is_code = code.get(at + lead).copied().unwrap_or(false);
            let trimmed = line.trim_start();
            if dialect == Dialect::MySql
                && is_code
                && trimmed
                    .get(..10)
                    .is_some_and(|w| w.eq_ignore_ascii_case("DELIMITER "))
            {
                statements.extend(statement(&tokens, src, begin..at));
                delimiter = trimmed[10..].trim().to_string();
                if delimiter.is_empty() {
                    delimiter = ";".into();
                }
                begin = next;
                at = next;
                continue;
            }
            if dialect == Dialect::Oracle && is_code && line.trim() == "/" {
                statements.extend(statement(&tokens, src, begin..at));
                begin = next;
                at = next;
                continue;
            }
        }
        if code[at] && src[at..].starts_with(delimiter.as_str()) {
            let words = leading_words(&tokens, src, begin..at, 4);
            if !(dialect == Dialect::Oracle && plsql_block(&words)) {
                statements.extend(statement(&tokens, src, begin..at));
                at += delimiter.len();
                begin = at;
                continue;
            }
        }
        at += src[at..].chars().next().map_or(1, char::len_utf8);
    }
    statements.extend(statement(&tokens, src, begin..src.len()));
    statements
}

/// The statement to run for a cursor at byte `offset`: the one it is in,
/// or the one whose `;` it is right after, else the next one below it,
/// else the last one.
pub fn statement_at(statements: &[Statement], offset: usize) -> Option<usize> {
    statements
        .iter()
        .position(|s| s.span.start <= offset && offset <= s.span.end)
        .or_else(|| statements.iter().position(|s| s.span.end + 1 == offset))
        .or_else(|| statements.iter().position(|s| s.span.start > offset))
        .or_else(|| statements.len().checked_sub(1))
}

/// Whether `statement` changes the schema: `CREATE`, `ALTER`, `DROP`,
/// `TRUNCATE`, `RENAME`, `COMMENT`, `GRANT` or `REVOKE` as its first word.
pub fn is_ddl(dialect: Dialect, statement: &str) -> bool {
    lex(dialect, statement)
        .iter()
        .find(|t| !is_trivia(t.kind))
        .is_some_and(|t| {
            t.kind == TokenKind::Keyword
                && matches!(
                    statement[t.span.clone()].to_ascii_uppercase().as_str(),
                    "CREATE"
                        | "ALTER"
                        | "DROP"
                        | "TRUNCATE"
                        | "RENAME"
                        | "COMMENT"
                        | "GRANT"
                        | "REVOKE"
                )
        })
}

/// Whether `statement` only reads: `SELECT`, `WITH` without `INSERT`,
/// `UPDATE`, `DELETE` or `MERGE` in it, `SHOW`, `EXPLAIN`, `DESCRIBE` /
/// `DESC`, `VALUES`, or transaction control (`BEGIN`, `COMMIT`, `ROLLBACK`
/// and the like), which writes nothing by itself. Anything else, including
/// a statement qry cannot read, counts as a write.
pub fn is_read_only(dialect: Dialect, statement: &str) -> bool {
    let words: Vec<String> = lex(dialect, statement)
        .iter()
        .filter(|t| !is_trivia(t.kind))
        .filter(|t| matches!(t.kind, TokenKind::Keyword | TokenKind::Ident))
        .map(|t| statement[t.span.clone()].to_ascii_uppercase())
        .collect();
    let Some(first) = words.first() else {
        return true;
    };
    match first.as_str() {
        "SELECT" | "SHOW" | "EXPLAIN" | "DESCRIBE" | "DESC" | "VALUES" | "BEGIN" | "START"
        | "COMMIT" | "ROLLBACK" | "END" | "ABORT" | "SAVEPOINT" | "RELEASE" => true,
        "WITH" => !words
            .iter()
            .any(|w| matches!(w.as_str(), "INSERT" | "UPDATE" | "DELETE" | "MERGE")),
        _ => false,
    }
}
