//! Reading other tools' connection files: `qry::import::from_bytes` on
//! generated 1000-connection files, from parsing the JSON to the converted
//! records, with the format detected as the import form does by default.

use std::{hint::black_box, path::Path};

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use qry::import::{Format, from_bytes};

#[path = "common.rs"]
mod common;

const N: usize = 1000;

fn import(c: &mut Criterion) {
    for (group, format, text) in [
        ("import_dbeaver", Format::DBeaver, common::dbeaver_file(N)),
        (
            "import_sqldeveloper",
            Format::SqlDeveloper,
            common::sqldeveloper_file(N),
        ),
        ("import_qry", Format::Qry, common::qry_file(N)),
    ] {
        // A generator that drifted from the importer would bench an error.
        match from_bytes(text.as_bytes(), Format::Auto, Path::new("bench.json")) {
            Ok(imported) if imported.format == format && imported.connections.len() == N => {}
            Ok(imported) => panic!(
                "{group}: read as {:?} with {} connections; skipped {:?}",
                imported.format,
                imported.connections.len(),
                imported.skipped
            ),
            Err(e) => panic!("{group}: the generated file does not import: {e:#}"),
        }

        let mut g = c.benchmark_group(group);
        g.throughput(Throughput::Bytes(text.len() as u64));
        g.bench_function("1k", |b| {
            b.iter(|| {
                black_box(from_bytes(
                    black_box(text.as_bytes()),
                    Format::Auto,
                    Path::new("bench.json"),
                ))
            });
        });
        g.finish();
    }
}

criterion_group!(benches, import);
criterion_main!(benches);
