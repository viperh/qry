# Testing

## The everyday suite

```sh
QRY_KEYRING=off cargo test --workspace --all-features --locked
```

`QRY_KEYRING=off` keeps the tests away from your real keychain. Unit tests
live next to their code in sibling `*_tests.rs` files; integration tests are
under `crates/*/tests/`.

## Snapshots

Screens are tested with [insta](https://insta.rs) snapshots, named with
their size (`help_120x40`), plain and `_styled` (every cell's colours).
After a change that moves pixels:

```sh
cargo insta test --review   # or: INSTA_UPDATE=new cargo test, then cargo insta review
```

Never commit a `*.snap.new`.

## The app harness

`qry::testing::AppHarness` runs the real reducer against a `FakeDatabase`
on a virtual clock: `keys("tab f5")`, `type_str("select 1")`,
`advance(1500)` (ticks, timers and slow replies fall due without waiting),
`render(120, 40)`. No test sleeps.

## Databases

The integration suites in `crates/qry-core/tests/db_*.rs` run against real
servers when their URLs are set, and skip otherwise (SQLite always runs):

```sh
docker compose -f deploy/docker-compose.test.yml up -d
eval "$(scripts/db-test-env.sh)"
cargo test -p qry-core --tests
```

The variables are `QRY_TEST_POSTGRES_URL`, `QRY_TEST_MYSQL_URL`,
`QRY_TEST_MARIADB_URL` and `QRY_TEST_ORACLE_URL`. Under `CI=true` a missing
URL fails instead of skipping, unless `QRY_DB_TESTS=skip` is set.

## Property tests and fuzzing

Every parser of untrusted input has a [proptest](https://proptest-rs.github.io/proptest/)
twin that runs in the normal suite, and a cargo-fuzz target:

```sh
cargo +nightly fuzz list --fuzz-dir fuzz
fuzz/seed-corpus.sh
cargo +nightly fuzz run sql_lex --fuzz-dir fuzz -- -max_total_time=60
```

A crash found by a fuzzer is fixed with its input saved under
`tests/fixtures/fuzz-regressions/<target>/`, which the twins replay.

## Benchmarks

```sh
cargo bench -p qry --bench results -- --save-baseline local
cargo bench -p qry-core --bench sql -- --save-baseline local
python3 scripts/bench-gate.py gate --baseline local
```

`scripts/bench-gates.toml` holds each gated bench's limit, for a local
machine and for CI; the nightly `bench.yml` workflow also fails on a regression of more than
15% against `master`.

## Generated docs

[keybindings.md](keybindings.md), the table in [config.md](config.md),
[config.schema.json](config.schema.json) and the README's screenshot are
made by the code. A test fails when they drift; regenerate them with:

```sh
QRY_BLESS=1 cargo test -p qry docs
```
