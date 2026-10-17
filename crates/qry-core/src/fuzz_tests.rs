#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The proptest twins of `qry-core`'s fuzz targets, over random bytes and
//! over structured inputs, plus the replay of inputs the fuzzers once
//! crashed.

use std::path::{Path, PathBuf};

use proptest::prelude::*;

use super::*;

/// Random bytes, up to 4 KiB.
fn bytes() -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(any::<u8>(), 0..4096)
}

/// SQL-shaped text: statements of words, numbers, strings, quoted
/// identifiers, comments, dollar quotes, Oracle `q'[…]'` quotes, PL/SQL
/// blocks, `/` lines and MySQL `DELIMITER` lines, joined by `;`, with
/// some of the quotes left open.
fn arb_sql() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        proptest::sample::select(vec![
            "select",
            "from",
            "where",
            "insert",
            "into",
            "values",
            "create",
            "or",
            "replace",
            "procedure",
            "function",
            "package",
            "trigger",
            "begin",
            "declare",
            "end",
            "as",
            "language",
            "plpgsql",
            "drop",
            "table",
            "x",
            "t1",
            "*",
            "=",
            "(",
            ")",
            ",",
            ".",
            "::",
            "||",
            "-",
            "+",
            "/",
            "\n/\n",
            "\nDELIMITER $$\n",
            "\nDELIMITER ;\n",
            "\ndelimiter //\n",
            "$$",
            "$fn$",
            "$",
            "'",
            "\"",
            "`",
            "[",
            "]",
            "--",
            "/*",
            "*/",
            "#",
            "q'[",
            "]'",
            "q'{",
            "N'",
            "E'\\'",
            "x'",
            "\\",
            "\r\n",
            "\t",
            "é",
            "🦀",
            "\u{0}",
        ])
        .prop_map(str::to_string),
        "[a-z_][a-z0-9_]{0,8}",
        "[0-9]{1,6}(\\.[0-9]{0,3})?(e[+-]?[0-9]{1,2})?",
        "'[^']{0,10}'",
        "\"[^\"]{0,10}\"",
        "--[^\n]{0,10}\n",
        "/\\*[^*]{0,10}\\*/",
        "\\$\\$[^$]{0,12}\\$\\$",
        "\\PC{0,4}",
    ];
    let statement = proptest::collection::vec(piece, 0..12).prop_map(|p| p.join(" "));
    proptest::collection::vec(statement, 0..6).prop_map(|s| s.join(";"))
}

/// A result's cells as `fuzz_grid` and `fuzz_copy_render` read them:
/// numbers, NaN and infinities, numbers with leading zeros, text, NULLs.
fn arb_result_bytes() -> impl Strategy<Value = Vec<u8>> {
    let cell = prop_oneof![
        proptest::sample::select(vec![
            "NaN", "nan", "-nan", "inf", "-inf", "infinity", "1e309", "-0", "0", "00", "007", "1",
            "10", "9", "a2", "a10", "a02", "A2", " 3 ", "", "NULL", "|", "\n", "\r\n", "\t", "\\",
            "'", "\"", "`", ",", "İ", "ß", "a01b", "a1b",
        ])
        .prop_map(|s| s.as_bytes().to_vec()),
        Just(vec![0xFE]),
        "[a-zA-Z0-9 ]{0,6}".prop_map(String::into_bytes),
        any::<f64>().prop_map(|f| f.to_string().into_bytes()),
        proptest::collection::vec(
            any::<u8>().prop_filter("not a separator", |&b| b != 0xFF),
            0..6
        ),
    ];
    (
        proptest::collection::vec(any::<u8>(), 7),
        proptest::collection::vec(cell, 0..60),
    )
        .prop_map(|(head, cells)| {
            let ncols = usize::from(head[0] % 4) + 1;
            let mut out = head[..ncols + 3].to_vec();
            out.extend(cells.join(&0xFF));
            out
        })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    #[test]
    fn sql_lex_never_panics_and_tiles(data in bytes(), sql in arb_sql()) {
        fuzz_sql_lex(&data);
        fuzz_sql_lex(sql.as_bytes());
    }

    #[test]
    fn sql_split_never_panics_and_covers_the_code(data in bytes(), sql in arb_sql()) {
        fuzz_sql_split(&data);
        fuzz_sql_split(sql.as_bytes());
    }

    #[test]
    fn grid_never_panics_and_sorts_into_a_permutation(data in bytes(), shaped in arb_result_bytes()) {
        fuzz_grid(&data);
        fuzz_grid(&shaped);
    }

    #[test]
    fn copy_render_never_panics(data in bytes(), shaped in arb_result_bytes()) {
        fuzz_copy_render(&data);
        fuzz_copy_render(&shaped);
    }
}

#[test]
fn result_from_bytes_reads_names_cells_and_nulls() {
    // Two columns (1 % 4 + 1), Integer and Text, two choice bytes, then the
    // names and two rows; the last row is cut short.
    let mut data = vec![1, 0, 3, 7, 9];
    data.extend(b"id\xFFname\xFF1\xFF\xFE\xFF2");
    let (result, choices) = result_from_bytes(&data);
    assert_eq!(choices, [7, 9]);
    assert_eq!(result.columns[0].name, "id");
    assert_eq!(result.columns[0].kind, ValueKind::Integer);
    assert_eq!(result.columns[1].kind, ValueKind::Text);
    assert_eq!(
        result.rows,
        vec![vec![Some("1".into()), None], vec![Some("2".into())]]
    );
}

/// `tests/fixtures/` at the repository root.
fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

/// The SQL fixtures the seed corpus starts from lex and split cleanly.
#[test]
fn sql_fixtures_pass_the_fuzz_bodies() {
    let mut seen = 0;
    for file in std::fs::read_dir(fixtures().join("sql")).unwrap() {
        let file = file.unwrap().path();
        let data = std::fs::read(&file).unwrap();
        fuzz_sql_lex(&data);
        fuzz_sql_split(&data);
        seen += 1;
    }
    assert!(
        seen >= 3,
        "expected the PL/SQL, dollar-quoted and DELIMITER samples"
    );
}

/// Which `fuzz_*` body replays the inputs under each target's directory.
fn entry_point(target: &str) -> Option<fn(&[u8])> {
    Some(match target {
        "sql_lex" => fuzz_sql_lex,
        "sql_split" => fuzz_sql_split,
        "grid" => fuzz_grid,
        "copy_render" => fuzz_copy_render,
        _ => return None,
    })
}

/// Every input a fuzzer once crashed on, under
/// `tests/fixtures/fuzz-regressions/<target>/`, runs through its target's
/// body again. The `qry` crate's targets are replayed by its own copy of
/// this test.
#[test]
fn known_crash_inputs_stay_fixed() {
    let root = fixtures().join("fuzz-regressions");
    let mut replayed = 0;
    for dir in std::fs::read_dir(&root).unwrap() {
        let dir = dir.unwrap().path();
        let Some(run) = dir
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(entry_point)
        else {
            continue;
        };
        for file in std::fs::read_dir(&dir).unwrap() {
            run(&std::fs::read(file.unwrap().path()).unwrap());
            replayed += 1;
        }
    }
    assert!(
        replayed > 0,
        "no regression inputs under {}",
        root.display()
    );
}
