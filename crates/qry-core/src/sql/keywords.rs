//! The words highlighted as keywords, upper-case and sorted for a binary
//! search.

use super::Dialect;

/// Keywords every dialect shares.
const COMMON: &[&str] = &[
    "ADD",
    "ALL",
    "ALTER",
    "AND",
    "ANY",
    "AS",
    "ASC",
    "BEGIN",
    "BETWEEN",
    "BY",
    "CASCADE",
    "CASE",
    "CAST",
    "CHECK",
    "COLUMN",
    "COMMENT",
    "COMMIT",
    "CONSTRAINT",
    "CREATE",
    "CROSS",
    "CURRENT_DATE",
    "CURRENT_TIME",
    "CURRENT_TIMESTAMP",
    "DATABASE",
    "DECLARE",
    "DEFAULT",
    "DELETE",
    "DESC",
    "DISTINCT",
    "DROP",
    "ELSE",
    "END",
    "ESCAPE",
    "EXCEPT",
    "EXISTS",
    "EXPLAIN",
    "FALSE",
    "FETCH",
    "FOR",
    "FOREIGN",
    "FROM",
    "FULL",
    "FUNCTION",
    "GRANT",
    "GROUP",
    "HAVING",
    "IF",
    "IN",
    "INDEX",
    "INNER",
    "INSERT",
    "INTERSECT",
    "INTO",
    "IS",
    "JOIN",
    "KEY",
    "LEFT",
    "LIKE",
    "LIMIT",
    "NATURAL",
    "NOT",
    "NULL",
    "OF",
    "OFFSET",
    "ON",
    "OR",
    "ORDER",
    "OUTER",
    "OVER",
    "PARTITION",
    "PRIMARY",
    "PROCEDURE",
    "REFERENCES",
    "RENAME",
    "REPLACE",
    "RETURNING",
    "REVOKE",
    "RIGHT",
    "ROLLBACK",
    "ROW",
    "ROWS",
    "SAVEPOINT",
    "SCHEMA",
    "SELECT",
    "SET",
    "TABLE",
    "THEN",
    "TO",
    "TRANSACTION",
    "TRIGGER",
    "TRUE",
    "TRUNCATE",
    "UNION",
    "UNIQUE",
    "UPDATE",
    "USING",
    "VALUES",
    "VIEW",
    "WHEN",
    "WHERE",
    "WITH",
];

const POSTGRES: &[&str] = &[
    "ILIKE",
    "INTERVAL",
    "LANGUAGE",
    "LATERAL",
    "MATERIALIZED",
    "RETURNS",
    "SIMILAR",
];
const MYSQL: &[&str] = &[
    "AUTO_INCREMENT",
    "DELIMITER",
    "ENGINE",
    "REGEXP",
    "SHOW",
    "USE",
];
const SQLITE: &[&str] = &["AUTOINCREMENT", "GLOB", "PRAGMA", "VACUUM", "WITHOUT"];
const ORACLE: &[&str] = &[
    "CONNECT",
    "DUAL",
    "EXCEPTION",
    "LOOP",
    "MERGE",
    "MINUS",
    "NOCOPY",
    "PACKAGE",
    "PRIOR",
    "ROWNUM",
    "START",
    "SYSDATE",
];

fn sorted_contains(list: &[&str], word: &str) -> bool {
    list.binary_search(&word).is_ok()
}

/// Whether `word` is a keyword in `dialect`, ignoring case.
pub(super) fn is_keyword(dialect: Dialect, word: &str) -> bool {
    if word.len() > 24 || !word.is_ascii() {
        return false;
    }
    let upper = word.to_ascii_uppercase();
    let extra = match dialect {
        Dialect::Postgres => POSTGRES,
        Dialect::MySql => MYSQL,
        Dialect::Sqlite => SQLITE,
        Dialect::Oracle => ORACLE,
    };
    sorted_contains(COMMON, &upper) || sorted_contains(extra, &upper)
}

#[cfg(test)]
pub(super) fn lists() -> [&'static [&'static str]; 5] {
    [COMMON, POSTGRES, MYSQL, SQLITE, ORACLE]
}
