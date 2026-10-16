//! Identifiers as SQL text: quoted only when the dialect needs it.

use super::{Dialect, TokenKind, lex};

/// `name` as an identifier in `dialect`, quoted only when it has to be:
///
/// - Postgres folds unquoted names to lower case, so only a lower-case
///   plain name (`users`, `order_items`) goes bare; `Users` is `"Users"`.
/// - Oracle folds to upper case, so only an upper-case plain name goes
///   bare; `users` is `"users"`.
/// - MySQL and MariaDB keep the case and quote with backticks.
/// - SQLite ignores case, so any plain name goes bare.
///
/// A plain name starts with a letter or `_` and holds only ASCII letters,
/// digits and `_`; a keyword (`order`, `user`) is quoted too. A quote
/// character inside the name is doubled.
pub fn quote_ident(dialect: Dialect, name: &str) -> String {
    if is_plain(dialect, name) {
        return name.to_string();
    }
    match dialect {
        Dialect::MySql => format!("`{}`", name.replace('`', "``")),
        Dialect::Postgres | Dialect::Sqlite | Dialect::Oracle => {
            format!("\"{}\"", name.replace('"', "\"\""))
        }
    }
}

/// `catalog.object`, each part quoted as it needs, or `object` alone when
/// `catalog` is `None`.
pub fn qualified_name(dialect: Dialect, catalog: Option<&str>, object: &str) -> String {
    match catalog {
        Some(catalog) => format!(
            "{}.{}",
            quote_ident(dialect, catalog),
            quote_ident(dialect, object)
        ),
        None => quote_ident(dialect, object),
    }
}

fn is_plain(dialect: Dialect, name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let case_ok = |c: char| match dialect {
        Dialect::Postgres => !c.is_ascii_uppercase(),
        Dialect::Oracle => !c.is_ascii_lowercase(),
        Dialect::MySql | Dialect::Sqlite => true,
    };
    let shape = (first.is_ascii_alphabetic() || first == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && name.chars().all(case_ok);
    shape
        && !matches!(
            lex(dialect, name).first().map(|t| t.kind),
            Some(TokenKind::Keyword)
        )
}
