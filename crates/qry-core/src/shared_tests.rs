#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Unit tests for `shared.rs`: the pieces every backend's stream is built
//! from.

use super::*;

fn row(n: u32) -> Vec<Option<String>> {
    vec![Some(n.to_string())]
}

#[test]
fn batches_hand_back_each_full_batch_and_then_the_rest() {
    let mut batches = Batches::new(2);
    assert_eq!(batches.push(row(1)), None);
    assert_eq!(batches.push(row(2)), Some(vec![row(1), row(2)]));
    assert_eq!(batches.sent(), 2);
    assert_eq!(batches.push(row(3)), None);
    assert_eq!(batches.rest(), Some(vec![row(3)]));
    assert_eq!(batches.sent(), 3);
    assert_eq!(batches.rest(), None, "nothing is handed back twice");
}

#[test]
fn a_fetch_size_of_zero_batches_a_row_at_a_time() {
    let mut batches = Batches::new(0);
    assert_eq!(batches.push(row(1)), Some(vec![row(1)]));
    assert_eq!(batches.rest(), None);
}

#[test]
fn an_empty_result_has_no_rest() {
    let mut batches = Batches::new(100);
    assert_eq!(batches.rest(), None);
    assert_eq!(batches.sent(), 0);
}

#[tokio::test]
async fn sending_to_a_reader_that_has_gone_is_an_error() {
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    drop(rx);
    let err = send(&tx, StreamItem::Rows(vec![row(1)])).await.unwrap_err();
    assert_eq!(err.to_string(), "the rows' reader stopped");
    let err = std::thread::spawn(move || blocking_send(&tx, StreamItem::Columns(Vec::new())))
        .join()
        .unwrap()
        .unwrap_err();
    assert_eq!(err.to_string(), "the rows' reader stopped");
}

#[tokio::test]
async fn send_result_sends_the_columns_then_batches_of_fetch_size() {
    let result = QueryResult {
        columns: vec![ColumnMeta::untyped("n")],
        rows: (1..=5).map(row).collect(),
        affected: None,
        elapsed: std::time::Duration::ZERO,
        truncated: false,
    };
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    assert_eq!(send_result(result, 2, &tx).await.unwrap(), 5);
    drop(tx);
    let mut items = Vec::new();
    while let Some(item) = rx.recv().await {
        items.push(item);
    }
    assert_eq!(
        items,
        [
            StreamItem::Columns(vec![ColumnMeta::untyped("n")]),
            StreamItem::Rows(vec![row(1), row(2)]),
            StreamItem::Rows(vec![row(3), row(4)]),
            StreamItem::Rows(vec![row(5)]),
        ]
    );
}
