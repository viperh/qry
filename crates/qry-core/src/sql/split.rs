//! The splitter: SQL text into the statements a server runs one at a time.

use std::{ops::Range, sync::Arc};

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

/// Whether a token is code, where a terminator counts: not a string, a
/// quoted identifier or a comment.
fn is_code(kind: TokenKind) -> bool {
    !matches!(
        kind,
        TokenKind::String | TokenKind::QuotedIdent | TokenKind::Comment | TokenKind::Unterminated
    )
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
fn statement(tokens: &[Token], src: &str, region: Range<usize>) -> Option<Range<usize>> {
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
    Some(first?..last?)
}

/// Where a statement may start: the start of the text, or just past a
/// terminator, a `DELIMITER` line or an Oracle `/` line; and the
/// terminator in force from there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Boundary {
    pub(super) at: usize,
    pub(super) delimiter: Arc<str>,
}

impl Boundary {
    pub(super) fn start() -> Self {
        Self {
            at: 0,
            delimiter: ";".into(),
        }
    }
}

/// Splits `src`, whose tokens are `tokens`, from `from` on, as [`split`]
/// does: each statement's span goes into `spans`, and each boundary,
/// `from` first, into `boundaries`. It stops at the first later boundary
/// that `stop` takes, leaving it out and returning it; it returns `None`
/// at the end of the text.
///
/// Nothing before `from` is looked at but the byte just before it, so a
/// run from a boundary of an earlier split goes on as that split did.
pub(super) fn split_from(
    dialect: Dialect,
    src: &str,
    tokens: &[Token],
    from: Boundary,
    spans: &mut Vec<Range<usize>>,
    boundaries: &mut Vec<Boundary>,
    mut stop: impl FnMut(&Boundary) -> bool,
) -> Option<Boundary> {
    let Boundary {
        at: mut begin,
        mut delimiter,
    } = from.clone();
    boundaries.push(from);
    let mut at = begin;
    let mut token = tokens.partition_point(|t| t.span.end <= at);
    let line_rules = matches!(dialect, Dialect::MySql | Dialect::Oracle);
    while at < src.len() {
        while tokens[token].span.end <= at {
            token += 1;
        }
        let t = &tokens[token];
        // Nothing ends a statement inside a string or a comment, and a
        // line that starts inside one is not a `DELIMITER` or `/` line.
        if !is_code(t.kind) {
            at = t.span.end;
            continue;
        }
        let mut next = None;
        if line_rules && (at == 0 || src.as_bytes()[at - 1] == b'\n') {
            let end = src[at..].find('\n').map_or(src.len(), |n| at + n);
            let line = &src[at..end];
            let trimmed = line.trim_start();
            let first = at + line.len() - trimmed.len();
            let starts_code = tokens[token..]
                .iter()
                .find(|t| t.span.end > first)
                .is_some_and(|t| is_code(t.kind));
            if dialect == Dialect::MySql
                && starts_code
                && trimmed
                    .get(..10)
                    .is_some_and(|w| w.eq_ignore_ascii_case("DELIMITER "))
            {
                spans.extend(statement(tokens, src, begin..at));
                let new = trimmed[10..].trim();
                delimiter = if new.is_empty() { ";" } else { new }.into();
                next = Some((end + 1).min(src.len()));
            } else if dialect == Dialect::Oracle && starts_code && line.trim() == "/" {
                spans.extend(statement(tokens, src, begin..at));
                next = Some((end + 1).min(src.len()));
            }
        }
        if next.is_none()
            && src[at..].starts_with(&*delimiter)
            && !(dialect == Dialect::Oracle
                && plsql_block(&leading_words(tokens, src, begin..at, 4)))
        {
            spans.extend(statement(tokens, src, begin..at));
            next = Some(at + delimiter.len());
        }
        if let Some(next) = next {
            let boundary = Boundary {
                at: next,
                delimiter: delimiter.clone(),
            };
            if stop(&boundary) {
                return Some(boundary);
            }
            boundaries.push(boundary);
            begin = next;
            at = next;
        } else if &*delimiter == ";" {
            // `;` is always a token of its own, and of the code tokens only
            // whitespace holds line breaks: the next place that matters is
            // the next token or line.
            at = match t.kind {
                TokenKind::Whitespace if line_rules => src[at..t.span.end]
                    .find('\n')
                    .map_or(t.span.end, |n| at + n + 1),
                _ => t.span.end,
            };
        } else {
            at += src[at..].chars().next().map_or(1, char::len_utf8);
        }
    }
    spans.extend(statement(tokens, src, begin..src.len()));
    None
}

/// The statements of `src`, in order. `;` outside strings and comments
/// ends one. In Oracle a line holding only `/` ends one too, and inside a
/// PL/SQL block (`BEGIN`, `DECLARE`, `CREATE … PROCEDURE` and the like)
/// only that line does. In MySQL a `DELIMITER x` line changes the
/// terminator and is not part of any statement. Empty statements and ones
/// of only comments are left out.
pub fn split(dialect: Dialect, src: &str) -> Vec<Statement> {
    let tokens = lex(dialect, src);
    let mut spans = Vec::new();
    split_from(
        dialect,
        src,
        &tokens,
        Boundary::start(),
        &mut spans,
        &mut Vec::new(),
        |_| false,
    );
    spans
        .into_iter()
        .map(|span| Statement {
            text: src[span.clone()].to_string(),
            span,
        })
        .collect()
}

/// The statement to run for a cursor at byte `offset`: the one it is in,
/// or the one whose `;` it is right after, else the next one below it,
/// else the last one.
pub fn statement_at(statements: &[Statement], offset: usize) -> Option<usize> {
    span_at(statements.iter().map(|s| &s.span), offset)
}

/// [`statement_at`] over the statements' spans.
pub(super) fn span_at<'a>(
    spans: impl Iterator<Item = &'a Range<usize>> + Clone,
    offset: usize,
) -> Option<usize> {
    spans
        .clone()
        .position(|s| s.start <= offset && offset <= s.end)
        .or_else(|| spans.clone().position(|s| s.end + 1 == offset))
        .or_else(|| spans.clone().position(|s| s.start > offset))
        .or_else(|| spans.count().checked_sub(1))
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
