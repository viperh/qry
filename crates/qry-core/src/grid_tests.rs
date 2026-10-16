//! Unit tests for `grid.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use proptest::prelude::*;

use super::*;
use crate::testing::{result, typed_result};

fn rows_of(result: &QueryResult, order: &[u32], col: usize) -> Vec<Option<String>> {
    order
        .iter()
        .map(|&i| result.rows[i as usize][col].clone())
        .collect()
}

fn s(v: &str) -> Option<String> {
    Some(v.to_string())
}

#[test]
fn widths_capped_and_sampled_from_first_1000_rows() {
    let mut rows: Vec<Vec<Option<String>>> = (0..1500).map(|_| vec![s("ab")]).collect();
    rows[1200][0] = s(&"x".repeat(30));
    let mut r = result(&["c"], &[]);
    r.rows = rows;
    assert_eq!(column_widths(&r, 40), [2], "row 1200 is past the sample");
    r.rows[3][0] = s(&"y".repeat(100));
    assert_eq!(column_widths(&r, 40), [40], "capped");
    let header = typed_result(&[("id", "timestamptz")], &[&[Some("1")]]);
    assert_eq!(column_widths(&header, 40), [11], "the type row counts");
}

#[test]
fn widths_count_display_width_not_bytes() {
    let r = result(&["c"], &[&[Some("日本")], &[Some("é")]]);
    assert_eq!(column_widths(&r, 40), [4]);
}

#[test]
fn visible_columns_with_frozen_first() {
    let widths = [5, 5, 5, 5];
    assert_eq!(visible_columns(&widths, 2, false, 11), [(2, 5), (3, 5)]);
    assert_eq!(visible_columns(&widths, 2, true, 11), [(0, 5), (2, 5)]);
    assert_eq!(visible_columns(&widths, 0, true, 11), [(0, 5), (1, 5)]);
    assert_eq!(
        visible_columns(&widths, 1, false, 7),
        [(1, 5), (2, 1)],
        "the last is cut"
    );
    assert!(visible_columns(&[], 0, true, 10).is_empty());
}

#[test]
fn sort_numeric_not_lexical() {
    let r = typed_result(
        &[("n", "int4")],
        &[&[Some("10")], &[Some("9")], &[Some("100")]],
    );
    let order = sort_permutation(&r, &[(0, Order::Asc)]);
    assert_eq!(rows_of(&r, &order, 0), [s("9"), s("10"), s("100")]);
}

#[test]
fn sort_nulls_last_asc_first_desc() {
    let r = result(&["c"], &[&[None], &[Some("b")], &[Some("a")]]);
    let asc = sort_permutation(&r, &[(0, Order::Asc)]);
    assert_eq!(rows_of(&r, &asc, 0), [s("a"), s("b"), None]);
    let desc = sort_permutation(&r, &[(0, Order::Desc)]);
    assert_eq!(rows_of(&r, &desc, 0), [None, s("b"), s("a")]);
}

#[test]
fn sort_is_stable() {
    let r = result(
        &["k", "v"],
        &[
            &[Some("x"), Some("1")],
            &[Some("x"), Some("2")],
            &[Some("a"), Some("3")],
        ],
    );
    let order = sort_permutation(&r, &[(0, Order::Asc)]);
    assert_eq!(order, [2, 0, 1]);
}

#[test]
fn sort_multi_key_priority() {
    let r = result(
        &["a", "b"],
        &[
            &[Some("1"), Some("z")],
            &[Some("2"), Some("a")],
            &[Some("1"), Some("a")],
        ],
    );
    let order = sort_permutation(&r, &[(0, Order::Asc), (1, Order::Desc)]);
    assert_eq!(order, [0, 2, 1]);
}

#[test]
fn sort_natural_text() {
    let r = result(&["c"], &[&[Some("a10")], &[Some("a2")], &[Some("A1")]]);
    let order = sort_permutation(&r, &[(0, Order::Asc)]);
    assert_eq!(rows_of(&r, &order, 0), [s("A1"), s("a2"), s("a10")]);
}

#[test]
fn search_case_insensitive_substring() {
    let r = result(
        &["a", "b"],
        &[&[Some("Foo"), Some("x")], &[Some("y"), Some("food")]],
    );
    let mut search = Search::new("FOO");
    assert!(search.step(&r, 100));
    assert_eq!(search.matches, [(0, 0), (1, 1)]);
}

#[test]
fn search_step_respects_budget_and_resumes() {
    let mut r = result(&["c"], &[]);
    r.rows = (0..25).map(|i| vec![s(&format!("row{i}"))]).collect();
    let mut search = Search::new("row1");
    assert!(!search.step(&r, 10));
    assert_eq!(search.matches, [(1, 0)]);
    assert!(!search.step(&r, 10));
    assert!(search.step(&r, 10));
    assert_eq!(search.matches.len(), 11, "row1 and row10..19");
}

#[test]
fn search_wraps_on_next() {
    let r = result(&["c"], &[&[s("x").as_deref()], &[Some("y")], &[Some("x")]]);
    let mut search = Search::new("x");
    search.step(&r, 10);
    assert_eq!(search.next_from((0, 0), true), Some((2, 0)));
    assert_eq!(search.next_from((2, 0), true), Some((0, 0)), "wraps");
    assert_eq!(search.next_from((0, 0), false), Some((2, 0)), "wraps back");
    assert_eq!(Search::new("zz").next_from((0, 0), true), None);
}

#[test]
fn display_text_maps_control_chars() {
    assert_eq!(display_text(None), "NULL");
    assert_eq!(display_text(Some("NULL")), "NULL", "the text NULL is text");
    assert_eq!(display_text(Some("a\nb\tc\0d\x07")), "a↵b→c␀d·");
    assert!(matches!(display_text(Some("plain")), Cow::Borrowed(_)));
    assert!(is_control_glyph('↵') && !is_control_glyph('a'));
}

proptest! {
    #[test]
    fn prop_sort_permutation_is_a_permutation(cells in prop::collection::vec(prop::option::of("[a-c0-9]{0,3}"), 0..40)) {
        let mut r = result(&["c"], &[]);
        r.rows = cells.into_iter().map(|c| vec![c]).collect();
        let mut order = sort_permutation(&r, &[(0, Order::Asc)]);
        order.sort_unstable();
        prop_assert_eq!(order, (0..r.rows.len() as u32).collect::<Vec<_>>());
    }

    #[test]
    fn prop_sort_then_reverse_matches_desc_for_non_null(cells in prop::collection::vec("[0-9]{1,4}", 0..40)) {
        let mut r = typed_result(&[("n", "int8")], &[]);
        r.rows = cells.into_iter().map(|c| vec![Some(c)]).collect();
        let value = |i: &u32| r.rows[*i as usize][0].clone().unwrap().parse::<u64>().unwrap();
        let asc: Vec<u64> = sort_permutation(&r, &[(0, Order::Asc)]).iter().map(value).collect();
        let mut desc: Vec<u64> = sort_permutation(&r, &[(0, Order::Desc)]).iter().map(value).collect();
        desc.reverse();
        prop_assert_eq!(asc, desc);
    }
}
