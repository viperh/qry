# T006 — Benchmarks and performance gates

**Phase:** 0 — Foundation · **Depends on:** T003 · **Blocks:** none (T302, T303 and T305 must keep their gates green)

## Description

qry has no benchmarks, and no performance budget is written down anywhere. The hot paths are all on the UI thread:

- **Results rendering and column sizing.** `crates/qry/src/components/results.rs` calls `column_widths(result)` (line ~181), which walks **every** cell of the result to size the columns, and draws the visible window of the grid on every frame. The frame rate defaults to 60 fps in `cli.rs`. A 100k-row `SELECT` must not stall input.
- **SQL lexing for highlighting** (T302). It runs on every keystroke over the whole editor buffer.
- **Import parsing.** A DBeaver workspace can hold hundreds of connections.
- **Start-up.** `App::run` loads connections with `spawn_blocking` and connects the in-memory scratch SQLite before the first frame.

This task adds criterion benchmarks, hard gates with CI-relaxed limits, and a nightly regression comparison. It mirrors sverb:

- `crates/*/benches/*.rs` with `harness = false`
- `scripts/bench-gates.toml` and `scripts/bench-gate.py {gate|compare|self-test}`
- `bench.yml`, nightly, with a cached `main` baseline and a >15% regression check
- a release-mode `#[ignore]`d start-up test, gated by `SVERB_STARTUP_GATE_MS`

Benchmarks are **not** run on pull requests, because shared runners are too noisy. PRs only build them (`bench-build` in `ci.yml`).

### Benchmarks

| Crate | File | Benchmark ids | What it measures |
|---|---|---|---|
| `qry` | `benches/results.rs` | `results_render_300x100/100k_rows`, `results_render_80x24/100k_rows` | One full draw of the results pane into a `TestBackend` of that size, with the selection in the middle of a 100k × 12 result. Measures the per-frame cost only, not sizing |
| `qry` | `benches/results.rs` | `column_widths/100k_rows`, `column_widths/1k_rows` | `column_widths` over the result (after T303 this is the incremental or sampled sizing it introduces) |
| `qry` | `benches/frame.rs` | `full_frame_160x48/idle`, `full_frame_160x48/100k_result` | `Home::draw` (or `App::render` after T101) for the whole screen, with the theme on, so T201/T202 chrome is included |
| `qry-core` | `benches/sql.rs` | `sql_lex/1MiB`, `sql_lex/10KiB`, `sql_split/1MiB` | T302's lexer and splitter on a generated mixed-SQL file (until T302 lands, these benches are behind `#[cfg(feature = "sql")]`, or simply added by T302 against this task's harness) |
| `qry` | `benches/import.rs` | `import_dbeaver/1k`, `import_sqldeveloper/1k`, `import_qry/1k` | `import::from_bytes` (T005) on generated 1k-connection files |
| `qry-core` | `benches/query.rs` | `sqlite_select/100k_rows` | `Driver::query` against an in-memory SQLite with 100k seeded rows, through to `QueryResult`. Covers the type conversion cost |

Fixtures are generated in each bench's setup, from a fixed seed, so nothing large is committed. A shared `benches/common.rs` provides `wide_result(rows, cols)`, `mixed_sql(bytes)` and `dbeaver_file(n)`.

Manifest entries:

```toml
[[bench]]
name = "results"
harness = false
```

`criterion = "0.8"` goes in `[workspace.dependencies]` under `# --- dev ---`.

### Gates: `scripts/bench-gates.toml`

```toml
# Hard limits. `max_ms` is the target on a developer machine; `ci_max_ms` is
# the limit enforced on shared GitHub runners (about 2x). A gate fails when
# the criterion median exceeds the CI limit.

[[gate]]
bench = "results_render_300x100/100k_rows"
max_ms = 2
ci_max_ms = 4          # one frame must fit well inside 16 ms at 60 fps

[[gate]]
bench = "full_frame_160x48/100k_result"
max_ms = 4
ci_max_ms = 8

[[gate]]
bench = "column_widths/100k_rows"
max_ms = 30
ci_max_ms = 60         # sizing happens once per result, not per frame

[[gate]]
bench = "sql_lex/10KiB"
max_ms = 0.5
ci_max_ms = 1          # a keystroke in a typical editor buffer

[[gate]]
bench = "sql_lex/1MiB"
min_mb_s = 50
ci_min_mb_s = 25

[[gate]]
bench = "import_dbeaver/1k"
max_ms = 50
ci_max_ms = 100
```

These numbers are starting proposals. Measure on the reference machine first, then commit numbers that pass with about 30% headroom, and record the measurements in the PR. If a gate cannot be met today (very likely `column_widths/100k_rows` and `full_frame` with the current full-scan sizing), commit it with the measured value plus headroom and a `# TODO(T303): tighten to N` comment. Do not drop it.

### `scripts/bench-gate.py`

Stdlib only.

- **`gate --baseline <name>`** reads `target/criterion/<group>/<bench>/<name>/estimates.json` for every `[[gate]]`, compares the median against `ci_max_ms` or `ci_min_mb_s`, and prints a table. It exits 1 on any breach. A gate whose bench has no estimates is also an error, so a renamed bench cannot silently disable its gate.
- **`compare --old main --new ci --threshold 15`** compares every bench present in both baselines, and fails if any median regressed by more than the threshold percent.
- **`self-test`** runs both modes against synthetic `estimates.json` trees in a temporary directory: one passing, one breaching, one regressed, one missing. It asserts the exit codes.

### Start-up gate

`crates/qry/tests/startup.rs`, marked `#[ignore = "release-mode timing; run by bench.yml"]`:

1. Builds a temporary `QRY_CONFIG`/`QRY_DATA` with 500 stored connections.
2. Runs `env!("CARGO_BIN_EXE_qry")` in a PTY (`portable-pty`, dev-dependency) 20 times.
3. Measures the time to the first rendered frame. Detect it by the top bar's app name appearing in the PTY output (`qry` in the T202 top bar; before T202, the `Query` title).
4. Asserts the median is under `QRY_STARTUP_GATE_MS` (default 150).

The app must not try to reach the network during start-up. Only the in-memory scratch connection opens.

### `bench.yml`

Copy sverb's `bench.yml`:

- Nightly cron `"17 3 * * *"` plus `workflow_dispatch`, `timeout-minutes: 60`.
- Restore `target/criterion` from `criterion-main-*`.
- Run `cargo bench --workspace --locked --bench '*' -- --save-baseline ci --noplot`.
- Run `python3 scripts/bench-gate.py gate --baseline ci`.
- Run `compare --old main --new ci --threshold 15` when a main baseline exists (otherwise print "No main baseline yet; this run becomes it.").
- Run the start-up test in release mode: `cargo test --release --locked -p qry --test startup -- --ignored --nocapture`, with `QRY_STARTUP_GATE_MS: "150"`.
- On `master` only, promote `ci` to `main` and save the cache.

The `bench-build` job in `ci.yml` (T003) runs `cargo bench --workspace --no-run --locked` and `python3 scripts/bench-gate.py self-test` on every PR.

### Out of scope

- Fixing the performance problems the benches reveal. T303 owns results-grid sizing and rendering, and T302 owns lexing. This task only makes them measurable and gated.

## Acceptance criteria

- [ ] All benchmark ids in the table exist and run with `cargo bench --workspace --locked`. The `sql_*` ids may be added by T302, with that recorded in T302's checklist.
- [ ] Every bench's fixtures are generated from a fixed seed. No fixture over 100 KiB is committed.
- [ ] `scripts/bench-gates.toml` has a gate for every row marked as gated above, with both local and CI limits, and the PR records the measured numbers.
- [ ] `scripts/bench-gate.py self-test` exits 0, and the script exits 1 for a breached gate, for a regression over the threshold, and for a gate whose bench is missing.
- [ ] `crates/qry/tests/startup.rs` exists, is `#[ignore]`d, passes in release mode locally, and reports the median time.
- [ ] `bench.yml` runs nightly and on dispatch, enforces the gates, compares against the cached `main` baseline, and promotes the baseline only on `master`.
- [ ] `bench-build` in `ci.yml` is green and builds every bench on PRs.
- [ ] A synthetic regression (a `std::thread::sleep(Duration::from_millis(5))` planted in the results draw, on a throwaway branch with the workflow dispatched) makes `bench.yml` fail on the gate. Record the run link.

## Tests

`scripts/bench-gate.py self-test` cases:

- **gate pass**: median 1.0 ms against `ci_max_ms = 4`, exit 0.
- **gate breach**: median 5.0 ms against `ci_max_ms = 4`, exit 1, and the output names the bench.
- **throughput gate**: 20 MB/s against `ci_min_mb_s = 25`, exit 1.
- **missing bench**: a gate with no estimates file, exit 1, with the message `no estimates for <bench>`.
- **compare regression**: old 10 ms, new 12 ms, threshold 15 → exit 0; new 12 ms, threshold 10 → exit 1.
- **compare with no common benches**: exit 0, printing a notice.

Rust tests:

- **`benches_common_fixtures_are_deterministic`** (`crates/qry/benches/common.rs`, run as a unit test through `#[cfg(test)]`): `wide_result(1000, 12)` called twice gives equal results, and `mixed_sql(10 * 1024)` is between 10 and 11 KiB.
- **`startup_reaches_the_first_frame_under_the_gate`** (`crates/qry/tests/startup.rs`, ignored by default).

CI:

- `bench-build` green on every PR.
- `bench.yml` green on its first scheduled run.
