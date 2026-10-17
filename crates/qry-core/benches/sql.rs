//! T302's lexer and splitter on generated mixed SQL: `sql_lex/10KiB` is a
//! keystroke in a typical editor buffer, `sql_lex/1MiB` and
//! `sql_split/1MiB` a pasted dump.

use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use qry_core::sql::{Dialect, lex, split};

#[path = "../../qry/benches/common.rs"]
mod common;

fn sql(c: &mut Criterion) {
    let small = common::mixed_sql(10 * 1024);
    let large = common::mixed_sql(1024 * 1024);

    let mut group = c.benchmark_group("sql_lex");
    for (id, text) in [("10KiB", &small), ("1MiB", &large)] {
        group.throughput(Throughput::Bytes(text.len() as u64));
        group.bench_function(id, |b| {
            b.iter(|| black_box(lex(Dialect::Postgres, black_box(text))));
        });
    }
    group.finish();

    let mut group = c.benchmark_group("sql_split");
    group.throughput(Throughput::Bytes(large.len() as u64));
    group.bench_function("1MiB", |b| {
        b.iter(|| black_box(split(Dialect::Postgres, black_box(&large))));
    });
    group.finish();
}

criterion_group!(benches, sql);
criterion_main!(benches);
