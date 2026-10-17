//! Fixtures the benchmarks share, generated from a fixed seed so every run
//! measures the same input and nothing large is committed.
//!
//! Only `qry_core` and `std` are used, so the `qry-core` benches include
//! this file too (`#[path = "../../qry/benches/common.rs"]`). The
//! determinism test is `crates/qry/tests/bench_fixtures.rs`, which includes
//! it the same way: bench targets are not built by `cargo test`.

// Each bench uses a few of these; the rest would warn.
#![allow(dead_code, unreachable_pub)]

use std::fmt::Write as _;

use qry_core::{ColumnMeta, QueryResult};

/// The seed every fixture starts from.
pub const SEED: u64 = 0x0005_eed0_f9e7;

/// splitmix64: tiny, fast, and the same on every platform.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A number in `0..n`.
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n.max(1)
    }

    /// One of `items`.
    pub fn pick<'a, T: ?Sized>(&mut self, items: &[&'a T]) -> &'a T {
        items[self.below(items.len() as u64) as usize]
    }
}

const WORDS: &[&str] = &[
    "alpha",
    "bravo",
    "charlie",
    "delta",
    "echo",
    "foxtrot",
    "golf",
    "hotel",
    "india",
    "juliet",
    "kilo",
    "lima",
    "mike",
    "november",
    "oscar",
    "papa",
    "quebec",
    "romeo",
    "sierra",
    "tango",
    "überweisung",
    "naïve",
    "東京",
    "données",
];

/// The column types of [`wide_result`], repeated across the width.
const COLUMN_TYPES: &[&str] = &["integer", "text", "numeric", "timestamp", "boolean", "text"];

/// A `rows` x `cols` result such as a wide `SELECT` gives: integer ids,
/// words of varying length (some wide characters), decimals, timestamps,
/// booleans and the odd NULL, with typed columns.
pub fn wide_result(rows: usize, cols: usize) -> QueryResult {
    let mut rng = Rng::new(SEED);
    let columns = (0..cols)
        .map(|c| {
            let type_name = COLUMN_TYPES[c % COLUMN_TYPES.len()];
            ColumnMeta::new(format!("{type_name}_{c}"), type_name)
        })
        .collect();
    let rows = (0..rows)
        .map(|r| {
            (0..cols)
                .map(|c| {
                    if c > 0 && rng.below(20) == 0 {
                        return None;
                    }
                    Some(match COLUMN_TYPES[c % COLUMN_TYPES.len()] {
                        "integer" if c == 0 => r.to_string(),
                        "integer" => rng.below(1_000_000).to_string(),
                        "numeric" => {
                            format!("{}.{:02}", rng.below(100_000), rng.below(100))
                        }
                        "timestamp" => format!(
                            "2026-{:02}-{:02} {:02}:{:02}:{:02}",
                            1 + rng.below(12),
                            1 + rng.below(28),
                            rng.below(24),
                            rng.below(60),
                            rng.below(60)
                        ),
                        "boolean" => (rng.below(2) == 0).to_string(),
                        _ => {
                            let n = 1 + rng.below(4);
                            (0..n)
                                .map(|_| rng.pick(WORDS))
                                .collect::<Vec<_>>()
                                .join(" ")
                        }
                    })
                })
                .collect()
        })
        .collect();
    QueryResult {
        columns,
        rows,
        affected: None,
        elapsed: std::time::Duration::ZERO,
        truncated: false,
    }
}

/// One generated statement, at most a few hundred bytes, ending in `;\n`:
/// selects with joins, inserts with quoted strings, updates, DDL, line and
/// block comments.
fn statement(rng: &mut Rng, out: &mut String) {
    let table = rng.pick(WORDS);
    let column = rng.pick(WORDS);
    let _ = match rng.below(6) {
        0 => writeln!(
            out,
            "SELECT t.id, t.\"{column}\", count(*) AS n\n  FROM {table} t\n  JOIN other o ON o.id = t.id\n WHERE t.amount > {} AND t.name LIKE '%{}%'\n GROUP BY 1, 2 ORDER BY n DESC LIMIT 100;",
            rng.below(10_000),
            rng.pick(WORDS)
        ),
        1 => writeln!(
            out,
            "INSERT INTO {table} (id, {column}, note) VALUES ({}, '{}', 'it''s {}');",
            rng.below(1_000_000),
            rng.pick(WORDS),
            rng.pick(WORDS)
        ),
        2 => writeln!(
            out,
            "-- {} {}\nUPDATE {table} SET {column} = {column} + {} WHERE id = {};",
            rng.pick(WORDS),
            rng.pick(WORDS),
            rng.below(100),
            rng.below(1_000_000)
        ),
        3 => writeln!(
            out,
            "/* {} */\nCREATE TABLE IF NOT EXISTS {table}_{} (\n    id integer PRIMARY KEY,\n    {column} varchar(255) NOT NULL,\n    created timestamp DEFAULT now()\n);",
            rng.pick(WORDS),
            rng.below(1000)
        ),
        4 => writeln!(
            out,
            "DELETE FROM {table} WHERE {column} IN (SELECT {column} FROM archive WHERE year < {});",
            1990 + rng.below(40)
        ),
        _ => writeln!(
            out,
            "WITH recent AS (SELECT * FROM {table} WHERE created > '2026-01-{:02}')\nSELECT {column}, sum(amount) FROM recent GROUP BY {column};",
            1 + rng.below(28)
        ),
    };
}

/// Mixed SQL of at least `bytes` bytes and less than `bytes + 1 KiB`: whole
/// statements, stopping at the first that reaches the size.
pub fn mixed_sql(bytes: usize) -> String {
    let mut rng = Rng::new(SEED);
    let mut out = String::with_capacity(bytes + 1024);
    while out.len() < bytes {
        statement(&mut rng, &mut out);
    }
    out
}

/// Mixed SQL of exactly `lines` lines, for the editor.
pub fn sql_lines(lines: usize) -> String {
    let mut rng = Rng::new(SEED);
    let mut out = String::new();
    let mut count = 0;
    while count < lines {
        let start = out.len();
        statement(&mut rng, &mut out);
        count += out[start..].lines().count();
    }
    out.lines().take(lines).collect::<Vec<_>>().join("\n")
}

/// A DBeaver `data-sources.json` with `n` connections across the drivers
/// the importer reads.
pub fn dbeaver_file(n: usize) -> String {
    let mut rng = Rng::new(SEED);
    let mut out = String::from("{\n  \"folders\": {},\n  \"connections\": {\n");
    for i in 0..n {
        let name = format!("{} {i}", rng.pick(WORDS));
        let (provider, driver, port) = match rng.below(4) {
            0 => ("postgresql", "postgres-jdbc", 5432),
            1 => ("mysql", "mysql8", 3306),
            2 => ("mysql", "mariaDB", 3306),
            _ => ("oracle", "oracle_thin", 1521),
        };
        let sep = if i + 1 == n { "" } else { "," };
        let _ = writeln!(
            out,
            "    \"{driver}-{i:08x}\": {{\n      \"provider\": \"{provider}\", \"driver\": \"{driver}\",\n      \"name\": \"{name}\", \"save-password\": true, \"read-only\": false,\n      \"configuration\": {{\n        \"host\": \"db{i}.example.com\", \"port\": \"{port}\", \"database\": \"{}\",\n        \"user\": \"{}\", \"configurationType\": \"MANUAL\", \"auth-model\": \"native\"\n      }}\n    }}{sep}",
            rng.pick(WORDS),
            rng.pick(WORDS),
        );
    }
    out.push_str("  }\n}\n");
    out
}

/// A SQL Developer connection export with `n` connections.
pub fn sqldeveloper_file(n: usize) -> String {
    let mut rng = Rng::new(SEED);
    let mut out = String::from("{\"connections\": [\n");
    for i in 0..n {
        let sep = if i + 1 == n { "" } else { "," };
        let _ = writeln!(
            out,
            "  {{\"name\": \"conn{i}\", \"type\": \"jdbc\", \"info\": {{\"OracleConnectionType\": \"BASIC\", \"RaptorConnectionType\": \"Oracle\", \"serviceName\": \"{}\", \"hostname\": \"db{i}.local\", \"port\": \"1521\", \"user\": \"{}\", \"ConnName\": \"conn{i}\"}}}}{sep}",
            rng.pick(WORDS).to_uppercase(),
            rng.pick(WORDS),
        );
    }
    out.push_str("]}\n");
    out
}

/// A qry connection backup (`connections.json`) with `n` connections.
pub fn qry_file(n: usize) -> String {
    let mut rng = Rng::new(SEED);
    let mut out = String::from("{\n  \"version\": 1,\n  \"connections\": [\n");
    for i in 0..n {
        let sep = if i + 1 == n { "" } else { "," };
        let _ = writeln!(
            out,
            "    {{\"id\": \"{:016x}\", \"name\": \"{} {i}\", \"driver\": \"postgres\", \"host\": \"db{i}.local\", \"port\": 5432, \"user\": \"{}\", \"database\": \"{}\", \"secret\": {{\"mode\": \"prompt\"}}}}{sep}",
            rng.next_u64(),
            rng.pick(WORDS),
            rng.pick(WORDS),
            rng.pick(WORDS),
        );
    }
    out.push_str("  ]\n}\n");
    out
}
