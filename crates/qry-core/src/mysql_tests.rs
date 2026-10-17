//! Unit tests for `mysql.rs`: how columns and values are shown, which
//! MariaDB shares. The servers themselves are in `tests/db_mysql.rs`.

use mysql_async::{Column, Value, consts::ColumnType};

use super::{Shown, cell, column_meta};
use crate::ValueKind;

fn column(column_type: ColumnType, charset: u16, length: u32) -> Column {
    Column::new(column_type)
        .with_name(b"c")
        .with_character_set(charset)
        .with_column_length(length)
}

#[test]
fn blob_and_text_are_told_apart_by_the_binary_collation() {
    let (blob, shown) = column_meta(&column(ColumnType::MYSQL_TYPE_BLOB, 63, 65_535));
    assert_eq!((blob.kind, shown), (ValueKind::Binary, Shown::Binary));
    assert_eq!(blob.type_name, "MYSQL_TYPE_BLOB");

    let (text, shown) = column_meta(&column(ColumnType::MYSQL_TYPE_BLOB, 255, 262_140));
    assert_eq!((text.kind, shown), (ValueKind::Text, Shown::Text));

    let (varbinary, shown) = column_meta(&column(ColumnType::MYSQL_TYPE_VAR_STRING, 63, 16));
    assert_eq!((varbinary.kind, shown), (ValueKind::Binary, Shown::Binary));
}

#[test]
fn tinyint_1_is_a_boolean_and_wider_tinyints_are_numbers() {
    let (flag, shown) = column_meta(&column(ColumnType::MYSQL_TYPE_TINY, 63, 1));
    assert_eq!((flag.kind, shown), (ValueKind::Bool, Shown::Bool));

    let (tiny, shown) = column_meta(&column(ColumnType::MYSQL_TYPE_TINY, 63, 4));
    assert_eq!((tiny.kind, shown), (ValueKind::Integer, Shown::Text));
}

#[test]
fn numbers_keep_their_kind_although_their_collation_is_binary() {
    let (decimal, shown) = column_meta(&column(ColumnType::MYSQL_TYPE_NEWDECIMAL, 63, 14));
    assert_eq!((decimal.kind, shown), (ValueKind::Decimal, Shown::Text));
}

#[test]
fn cells_are_written_as_the_column_says() {
    let bytes = |b: &[u8]| Value::Bytes(b.to_vec());
    assert_eq!(
        cell(bytes(&[0x00, 0xff, 0x10]), Shown::Binary).as_deref(),
        Some("\\x00ff10")
    );
    assert_eq!(cell(bytes(b"1"), Shown::Bool).as_deref(), Some("true"));
    assert_eq!(cell(bytes(b"0"), Shown::Bool).as_deref(), Some("false"));
    // A tinyint(1) can hold other numbers; those stay numbers.
    assert_eq!(cell(bytes(b"7"), Shown::Bool).as_deref(), Some("7"));
    assert_eq!(cell(Value::Int(1), Shown::Bool).as_deref(), Some("true"));
    assert_eq!(
        cell(bytes("Zoë".as_bytes()), Shown::Text).as_deref(),
        Some("Zoë")
    );
    assert_eq!(
        cell(bytes(b"-12.50"), Shown::Text).as_deref(),
        Some("-12.50")
    );
    for shown in [Shown::Text, Shown::Binary, Shown::Bool] {
        assert_eq!(cell(Value::NULL, shown), None);
    }
}
