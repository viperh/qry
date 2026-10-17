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

- [x] All benchmark ids in the table exist and run with `cargo bench --workspace --locked`. The `sql_*` ids may be added by T302, with that recorded in T302's checklist.
- [x] Every bench's fixtures are generated from a fixed seed. No fixture over 100 KiB is committed.
- [x] `scripts/bench-gates.toml` has a gate for every row marked as gated above, with both local and CI limits, and the PR records the measured numbers.
- [x] `scripts/bench-gate.py self-test` exits 0, and the script exits 1 for a breached gate, for a regression over the threshold, and for a gate whose bench is missing.
- [x] `crates/qry/tests/startup.rs` exists, is `#[ignore]`d, passes in release mode locally, and reports the median time.
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

## Implementation notes

- **Harness.** criterion 0.8.2 was not in `Cargo.lock` but could be fetched, so the benches use it as specified: `criterion = { version = "0.8.2", default-features = false, features = ["cargo_bench_support"] }` in `[workspace.dependencies]` under `# --- dev ---` (no plotters or rayon: `bench.yml` passes `--noplot`, and the gates read only `estimates.json`/`benchmark.json`). `portable-pty = "0.9.0"` was added the same way for the start-up test. Every new crate in the lock is MIT, Apache-2.0, BSD-2-Clause or a choice of those; `cargo deny` itself is not installed here, so its job is first checked on GitHub.
- **Targets.** `crates/qry/benches/{results,frame,editor,import}.rs` and `crates/qry-core/benches/{sql,query}.rs`, each `harness = false`. `qry` sets `autobenches = false`, because `benches/app.rs` and `benches/common.rs` are shared modules, not benches. Both libraries (and the `qry` binary) set `bench = false`, so `cargo bench` builds only the criterion benches instead of the unit tests in bench mode too.
- **Everything goes through public entry points.** The views and the reducer are being changed by other tasks, so no bench reaches into `views::*` or `app::*` internals:
  - Draws use `AppHarness::render_buffer`, which runs `qry::views::draw` into a `TestBackend` (and clones its buffer; small next to the draw). So `results_render_*` measure the **whole screen** at that size with the results pane focused, not the results pane alone. `benches/app.rs` puts the app in shape with keys and events: `DbAnswer::QueryDone` brings the result, Tab/F6 move the focus (checked through `AppHarness::contexts`, panicking if it never gets there), and `down` is pressed `rows / 2` times so the selection is on the middle row whatever the page size.
  - `column_widths/*` benches `qry_core::grid::column_widths` (the sampled sizing that T303 introduced, cap 40). The app's reducer still sizes columns with its own full scan (`app/reducer/results.rs`); the extra, ungated `results_arrive/100k_rows` measures that: handling the 100k-row `QueryDone` takes **176 ms** on the UI thread. That is T303's to fix by switching to the sampled sizing.
  - `import_*` needed a public parser: `qry::import::from_bytes(bytes, format, path)` is new, and `from_file` now reads the file and calls it (the zip and UTF-8 checks moved there unchanged). The benches detect the format (`Format::Auto`), as the import form does by default, and check once that the generated file imports whole.
- **Benches named by other tasks.**
  - `editor_render_10k_lines` (T302): a 120x40 frame with a 10 000-line buffer, plus the ungated `editor_keystroke_10k_lines` (type `x`, draw, backspace, draw).
  - `results_render_1m` (T303): 120x40 of a 1M x 20 result. Setup holds about 1 GB of strings and takes a few seconds.
  - `stream_first_1000_of_1m_sqlite` (T305): **streaming does not exist yet**, so this is `Driver::query("SELECT * FROM big LIMIT 1000")` on a 1M-row in-memory SQLite table. When T305 lands the row stream, the bench should read the first 1000 rows of an unlimited `SELECT` from the stream, under the same id.
  - `sql_lex/*` and `sql_split/1MiB` are added here, not by T302, since T302's lexer and splitter have landed.
- **Fixtures** come from `benches/common.rs` (splitmix64, `SEED = 0x0005_eed0_f9e7`): `wide_result(rows, cols)` (typed integer/text/numeric/timestamp/boolean columns, about 5% NULLs, some wide characters), `mixed_sql(bytes)` (whole statements, between `bytes` and `bytes + 1 KiB`), `sql_lines(n)`, and `dbeaver_file(n)`, `sqldeveloper_file(n)`, `qry_file(n)`. It uses only `qry_core` and `std`, so `qry-core`'s benches include it through `#[path = "../../qry/benches/common.rs"]`. The SQLite tables are filled by SQLite itself from deterministic expressions of the row number. Nothing new is committed apart from source.
- **`benches_common_fixtures_are_deterministic`** is in `crates/qry/tests/bench_fixtures.rs`, which includes `benches/common.rs`, rather than a `#[cfg(test)]` inside the bench: `cargo test` does not build bench targets, so a test there would never run. The same file has `generated_connection_files_import_whole`.
- **Measured numbers** (criterion medians, this sandbox: 4 cores, shared with seven other agents' builds, so noisy). Disk was too short for the LTO release profile, so this run used `CARGO_PROFILE_BENCH_LTO=false CARGO_PROFILE_BENCH_CODEGEN_UNITS=16`; CI's numbers (full `[profile.release]`: LTO, one codegen unit, `opt-level = "s"`) will differ somewhat.

  | Bench | Median | Gate (local / CI) |
  |---|---|---|
  | `results_render_300x100/100k_rows` | 4.30 ms | 6 / 12 ms, `TODO(T303): tighten to 2 / 4` |
  | `results_render_80x24/100k_rows` | 0.25 ms | — |
  | `results_render_1m` | 0.60 ms | 2 / 4 ms |
  | `full_frame_160x48/idle` | 0.76 ms | — |
  | `full_frame_160x48/100k_result` | 1.06 ms | 4 / 8 ms |
  | `column_widths/100k_rows` | 0.58 ms | 1 / 2 ms |
  | `column_widths/1k_rows` | 0.56 ms | — |
  | `results_arrive/100k_rows` | 176 ms | — |
  | `editor_render_10k_lines` | 59.4 ms (0.83 ms after T302's follow-up) | 2 / 4 ms (was 80 / 160 with `TODO(T302)`) |
  | `editor_keystroke_10k_lines` | 149 ms (11.6 ms after) | — |
  | `sql_lex/10KiB` | 0.31 ms (0.12 ms after) | 0.25 / 0.5 ms (was 0.5 / 1) |
  | `sql_lex/1MiB` | 32.2 ms (31 MB/s; 77 MB/s after) | 50 / 25 MB/s (was 24 / 12 with `TODO(T302)`) |
  | `sql_split/1MiB` | 50.2 ms (20.6 ms after) | — |
  | `import_dbeaver/1k` | 4.57 ms | 50 / 100 ms |
  | `import_sqldeveloper/1k` | 4.94 ms | — |
  | `import_qry/1k` | 3.26 ms | — |
  | `sqlite_select/100k_rows` | 115 ms | — |
  | `stream_first_1000_of_1m_sqlite` | 1.27 ms | 50 / 100 ms (T305's figures) |
  | start-up to first frame (500 connections, 20 runs) | 8.5 ms (min 5.6, max 18.4) | 150 ms |

  `python3 scripts/bench-gate.py gate --baseline local` passes on these.
  - **Gates that miss their targets today**, committed at the measured value plus headroom with a TODO, as the task says: `results_render_300x100` (4.3 ms against a 2 ms target), `editor_render_10k_lines` (59 ms: the editor lexes and splits the whole buffer on every draw, so typing in a 10k-line buffer costs about 75 ms a keystroke, over a 60 fps frame) and `sql_lex/1MiB` (31 MB/s against 50).
  - **T302's follow-up closed two of them.** The editor now keeps its tokens and statements in state (`qry_core::sql::Outline`, updated by the reducer around each edit), so a frame lexes nothing, and the lexer and splitter are faster; see T302's notes. Measured on one machine before and after (LTO off): `editor_render_10k_lines` 51.1 → 0.83 ms, `editor_keystroke_10k_lines` 110 → 11.6 ms, `sql_lex/10KiB` 0.23 → 0.12 ms, `sql_lex/1MiB` 43 → 77 MB/s, `sql_split/1MiB` 39.8 → 20.6 ms. The gates went to 2 / 4 ms, 0.25 / 0.5 ms and 50 / 25 MB/s, past the TODOs' 8 / 16 ms for the editor since a frame no longer grows with the buffer; the TODOs are gone. Only `results_render_300x100` (T303) still misses its target.
  - **Gates beyond the table**: `results_render_1m`, `editor_render_10k_lines` and `stream_first_1000_of_1m_sqlite`, which T303, T302 and T305 asked for.
  - `column_widths/100k_rows` is gated at 1 / 2 ms, not the proposed 30 / 60: the sampled sizing costs the same for 1k and 100k rows, and a looser gate would not notice a return to the full scan.
- **`scripts/bench-gate.py`** (stdlib only, Python 3.11+ for `tomllib`): `gate`, `compare` and `self-test` as specified, plus `--criterion DIR` (default `target/criterion`, or `$CARGO_TARGET_DIR/criterion`). An ungrouped id such as `results_render_1m` has one directory level fewer. Throughput gates divide the bytes in `benchmark.json` by the median, in MB/s (10^6 bytes; criterion prints MiB/s). `self-test` runs eleven cases: gate pass, gate breach (naming the bench), throughput breach and pass, missing bench (`no estimates for <bench>`), an ungrouped bench, compare within and over the threshold, compare with no common benches, and that every gate in the real `bench-gates.toml` has both limits.
  - **The task's compare case is off:** it lists "old 10 ms, new 12 ms, threshold 15 → exit 0", but +20% must fail a 15% threshold. The self-test checks 10 → 12 ms against thresholds of 25 (exit 0) and 10 (exit 1), and 10 → 11 ms against 15 (exit 0).
- **`crates/qry/tests/startup.rs`**: 500 Postgres records with `prompt` secrets in a temporary `QRY_DATA` (nothing may connect at start-up), one untimed warm-up and 20 timed runs in a 120x40 `portable-pty`. The first frame is the moment both `qry` and `NORMAL` (the top bar's name and mode pill) appear after the alternate-screen switch, so a `qry:` warning on stderr cannot count. A cursor-position query (`ESC[6n`) would be answered, though qry sends none. It prints median, min and max, and asserts the median against `QRY_STARTUP_GATE_MS` (default 150). Locally it passes in release mode, built with LTO off for lack of disk space.
- **`bench.yml`**: nightly at `17 3 * * *` and on dispatch, 60-minute timeout. It restores `target/criterion` from the newest `criterion-main-*` cache (`restore-keys`), runs `cargo bench --workspace --locked --bench '*' -- --save-baseline ci --noplot`, enforces the gates, compares against `main` when a `main/estimates.json` exists (otherwise it prints "No main baseline yet; this run becomes it."), and runs the start-up test with `QRY_STARTUP_GATE_MS: "150"`. On `master` only it copies every `ci` baseline to `main` and saves the cache as `criterion-main-<run id>`. `target/criterion` is uploaded as an artifact on every run.
- **`ci.yml`**: `bench-build` no longer needs the `hashFiles` condition on the self-test step, so it is gone; the job's comment says what it does now.
- **Unticked boxes need GitHub:** `bench.yml` running nightly and on dispatch, `bench-build` green on a PR, and the planted-sleep run link. Nothing was pushed. Local stand-ins: the bench build (`cargo bench --workspace --no-run`) and `bench-gate.py self-test` pass, and the gate script passes against a real criterion tree (baseline `local`). The box about the PR recording the numbers is ticked because the table above holds them, ready for the PR.
- **Checks run locally:** `cargo fmt --all`, `cargo clippy --all-targets --all-features --workspace --locked -- -D warnings`, `QRY_KEYRING=off CI=true cargo test --workspace --all-features --locked` (403 `qry` unit tests, 90 in `qry-core`, plus the integration tests; the start-up test is ignored), the CI docs command, `scripts/check-layering.py`, every bench once, and the start-up test in release mode.

Schedule removed at the owner's request: `bench.yml` no longer runs on a cron;
it runs only by hand (`workflow_dispatch`, Actions → Run workflow). The
boxes about it running nightly no longer apply as written.

