# T303 — Results grid redesign

**Phase:** 3 — Features · **Depends on:** T201, T202 · **Blocks:** T304, T307

## Description

### Current state

`crates/qry/src/components/results.rs` renders one `Arc<QueryResult>` with ratatui's `Table`:

- It sits inside `Block::bordered().title(" Results ")`. The border is green when focused.
- The selected cell is `black().on_green()`. The rest of the row is `reversed`. There is a scrollbar.
- Column widths are each column's widest value, capped at `MAX_COLUMN_WIDTH = 40`.
- Helpers `scroll_rows`, `scroll_columns` and `visible_columns` handle the viewport.
- Keys come from `ResultsCommand`: arrows, `i/j/k/l`, PgUp/PgDn, Home/End = first/last column, `g`/`G` = first/last row.
- A statement with no columns prints "executed".

`QueryResult` (`qry-core/src/shared.rs:86`) is `{ columns: Vec<String>, rows: Vec<Vec<Option<String>>>, affected: Option<u64> }`. It has **no column types**, so numbers cannot be right-aligned. Only one result is kept, and a new query replaces it.

### Target design (tachy table look)

```
  1 users · 42   2 orders · 7   3 UPDATE · 3 rows                       ← result tabs (active: fg on bg, amber underline)
     │ id   │ name      email                 created_at            total │ → 4 more cols
     │ int8 │ text      text                  timestamptz         numeric │
   1 │    1 │ alice     alice@example.com     2026-10-01 09:12:44   12.50 │
   2 │    2 │ bob       NULL                  2026-10-02 10:00:01  100.00 │   ← odd rows bg_alt_row
   3 │    3 │ █carol█   carol@example.com     2026-10-03 11:30:00    7.25 │   ← cursor cell: bg on amber
  ...
 42 rows · 38 ms · sorted by total ▼                                   R 3 / 42  C 2/9
```

**Header.** Two rows on `surface`:

- Row 1 is the column name (`header`, bold).
- Row 2 is the type (`header_type`, dim).
- The active column's name uses `header_active` (amber).
- The sort indicator `▲`/`▼` (amber bold) goes after the name, with a priority digit for a multi-column sort.

**Gutter:**

- Row numbers right-aligned in `gutter` (dim).
- The selected row's number in `gutter_selected` (amber).
- `│` in `column_divider`.
- When the first column is frozen (`F` toggles), a `frozen_divider` (`border_strong`) follows it.

**Body:**

- Striped rows via `row(i)`. The selected row is `row_selected`. The cursor cell is `cursor_cell`.
- `NULL` is drawn as the literal `NULL` in dim italic, and is distinguishable from the string `'NULL'`, which is drawn in normal fg.
- Numeric types (int, decimal, float, money) are right-aligned.
- Control characters (`\n`, `\t`, `\0`, and others below 0x20) are shown as dim `↵`, `→` and `␀`. Long values are truncated with `…`.
- Columns that do not fit produce an overflow hint `→ N more cols` (`overflow_hint`).

**Result tabs.** Every statement of a run (T302) adds a result set. The tab label is ` N name · rows `:

- `name` is the first table in the FROM clause when cheap to detect, otherwise `result`.
- DML shows ` N UPDATE · 3 rows `.
- Errors show a coral ` N ✗ error `.

Tabs are switched with `[`/`]` or `alt-1…9`. At most 20 tabs are kept, the oldest dropped first. The state of each tab (selection, offsets, sort, search) is kept per tab.

**Non-row results:**

- **DML/DDL:** a centred line `✓ 3 rows affected · 12 ms`, with the check mark green.
- **Error:** a coral message. When the error has a position, the offending statement line is shown with `^~~~` under the position, in the tachy `query_bar` style.

**Interaction** (Results context, T102):

| Key | Action |
|---|---|
| `↑↓←→`, `hjkl`, PgUp/PgDn, `ctrl-d/u`, `g`/`G`, `0`/`$` (or Home/End) | move |
| `:` | goto row: a small teal dialog that accepts `N`, or `N%` for a percentage |
| `s` / `S` | sort the current column asc → desc → off / add it as a secondary sort key |
| `/` | search in the result set: the query line opens at the bottom of the pane |
| `n` / `N` | next / previous match |
| `y` | copy cell |
| `Y` | copy row (TSV) |
| `ctrl-y` | copy menu: cell, row, column, whole result × CSV, TSV, JSON, Markdown, SQL INSERT |
| `<` / `>` | narrow / widen the current column (min 3, max 200) |
| `=` | reset widths |
| `F` | freeze or unfreeze the first column |
| `w` | toggle wrap of the current cell into the inspector (T304) |
| `Enter` | value popup (T304) |

**Sort.** Sorting is **client-side**, over the rows currently fetched. The status says `sorted (fetched rows only)` when the result is truncated (T305).

- Comparison is type-aware: numbers numerically, dates lexically (ISO), text with natural ordering.
- NULLs sort last ascending and first descending.
- The sort is stable.
- It does not mutate the `Arc<QueryResult>`. It works through a permutation index `Vec<u32>`.

**Search.** A case-insensitive substring match over the visible text of every cell.

- Matches are drawn with `match_highlight` (amber on amber_tint).
- The status shows `match 3 / 17`.
- The search scans incrementally, in chunks of 10 000 rows per tick, so a 1M-row result does not block input.

**Clipboard.**

- Copying uses OSC 52 by default, which works over SSH.
- `arboard` is the fallback behind the `clipboard` cargo feature (default on). Config `clipboard = "osc52" | "system" | "both"` chooses between them.
- Success shows a toast `copied 1 row (TSV)` (T205).

### qry-core split

`crates/qry-core/src/shared.rs`:

```rust
pub struct ColumnMeta { pub name: String, pub type_name: String, pub kind: ValueKind }
pub enum ValueKind { Integer, Decimal, Float, Text, Bool, Date, Time, Timestamp, Json, Binary, Other }

pub struct QueryResult {
    pub columns: Vec<ColumnMeta>,          // was Vec<String>
    pub rows: Vec<Vec<Option<String>>>,
    pub affected: Option<u64>,
    pub elapsed: std::time::Duration,      // filled by the worker (T305)
    pub truncated: bool,                   // more rows exist (T305)
}
```

Each backend maps its native type to `ValueKind`:

- Postgres via the extended protocol `Column::type_()`.
- MySQL via `Column::column_type()`.
- SQLite via `decltype`, falling back to the value's runtime type in the first row.
- Oracle via `ColumnInfo::oracle_type()`.

`crates/qry-core/src/grid.rs` holds pure grid logic, tested without a TTY:

- `column_widths(&QueryResult, cap)` and `visible_columns(...)`, moved from `results.rs`.
- `sort_permutation(&QueryResult, &[(col, Order)]) -> Vec<u32>`.
- `Search::new(needle).step(&result, budget) -> Progress`.
- `display_text(cell) -> Cow<str>`, which handles the control-char and NULL mapping.

`crates/qry-core/src/copy.rs`:

- `render(&QueryResult, Selection, CopyFormat) -> String` for CSV, TSV, JSON, Markdown and SQL INSERT.
- INSERT uses `quote_ident` from T301 and dialect literal quoting.
- T307 reuses these writers.

The qry component keeps only view state (tabs, offsets, sort spec, search state) and drawing.

### Edge cases

- Zero rows with columns: header plus a dim `(no rows)`.
- Zero columns: the DML view.
- Duplicate column names (`SELECT 1, 1`) and empty column names: shown as `?column?` the way Postgres does.
- Very wide cells (100 KB JSON) are truncated at render time, never copied into width calculations beyond the cap. Width is computed from the first 1 000 rows and the header only.
- A terminal narrower than gutter plus one column shows the gutter and a partial column, and does not panic.
- A result arriving while the user is scrolling another tab must not move their selection.

### Out of scope

Editing cells, server-side sort and filter (re-querying with `ORDER BY`), pivot or charts.

## Acceptance criteria

- [x] `QueryResult.columns` carries `ColumnMeta` with a `ValueKind` for all five backends. Existing callers (exporter, help, tests) are updated.
- [x] The grid has no box, a two-row header (name and type), a gutter, `│` dividers, striped rows, a selected-row band and an amber cursor cell, all from `Theme` helpers.
- [x] Numeric columns are right-aligned. `NULL` is dim italic and visually distinct from the text `'NULL'`.
- [x] Control characters render as dim glyphs, and long values are truncated with `…`.
- [x] Multiple result sets show as tabs. Each tab keeps its own selection, sort and search. At most 20 tabs are kept.
- [x] DML shows `✓ N rows affected · T ms`. Errors show a coral message and, with a position, a `^~~~` marker.
- [x] Sort cycles asc, desc, off; `S` adds secondary keys; NULLs are ordered as specified; the sort is stable; the shown indicators are correct.
- [x] Search highlights every match, `n`/`N` cycle through them, and searching 1M rows never blocks input for more than one tick.
- [ ] Copy works in every format listed. INSERT output re-runs on the same backend and reproduces the rows (integration test).
- [x] `:` goto, column resize `<`/`>`/`=`, and freeze `F` work.
- [x] The existing `results.rs` behaviour tests (edges, scrolling keeps the selection visible, header kept) pass in the ported `results_tests.rs`.
- [ ] Rendering 120×40 of a 1M × 20 result stays under the T006 gate (`results_render_1m`).

## Tests

**Unit tests** — `crates/qry-core/src/grid_tests.rs`:

- Widths: `widths_capped_and_sampled_from_first_1000_rows`, `widths_count_display_width_not_bytes`.
- Visibility: `visible_columns_with_frozen_first`.
- Sorting:
  - `sort_numeric_not_lexical` (`10 > 9`)
  - `sort_nulls_last_asc_first_desc`
  - `sort_is_stable`
  - `sort_multi_key_priority`
  - `sort_natural_text` (`a2 < a10`)
- Search:
  - `search_case_insensitive_substring`
  - `search_step_respects_budget_and_resumes`
  - `search_wraps_on_next`
- Display: `display_text_maps_control_chars`.

**Unit tests** — `crates/qry-core/src/copy_tests.rs`:

- `csv_quotes_separator_and_newlines`
- `tsv_escapes_tabs`
- `json_null_is_null_not_string`
- `markdown_escapes_pipes`
- `insert_quotes_identifiers_and_literals_per_dialect`
- `insert_null_and_numbers_unquoted`

**Proptests:**

- `prop_sort_permutation_is_a_permutation`
- `prop_sort_then_reverse_matches_desc_for_non_null`
- `prop_csv_round_trips_through_csv_reader` (dev-dep `csv`)

**Unit tests** — `crates/qry/src/components/results_tests.rs`:

- The ported existing tests.
- Tabs: `tabs_keep_independent_state`, `tab_cap_drops_oldest`.
- Sorting and search keys: `s_cycles_sort_order`, `slash_search_then_n_moves_selection`.
- Goto: `goto_row_dialog_accepts_number_and_percent`.
- Column width: `resize_column_bounds`.
- Clipboard: `copy_cell_emits_clipboard_effect_and_toast`.
- `late_result_does_not_move_selection_of_other_tab`

**Snapshots** — plain and styled:

- `grid_basic_80x24`
- `grid_wide_overflow_hint_80x24`
- `grid_frozen_first_column_100x24`
- `grid_nulls_and_control_chars_80x12`
- `grid_numeric_right_aligned_80x12`
- `grid_sorted_multi_key_80x12`
- `grid_search_matches_80x12`
- `grid_dml_affected_80x8`
- `grid_error_with_position_80x10`
- `grid_tabs_three_results_120x24`
- `grid_empty_rows_80x8`
- `grid_tiny_terminal_10x3` (must not panic)

**AppHarness flow:** run `select * from t` (fake session, 200 rows), then `h.keys("G : 5 0 enter s s / f o o enter n")`, and assert the selection and status text.

**DB integration tests** (T004):

- `column_kinds_mapped` for each backend: a fixture table with int, decimal, text, bool, date, timestamp, json and blob.
- `copy_insert_round_trip` for each backend.

## Implementation notes

- **qry-core.** `ColumnMeta`/`ValueKind` live in `shared.rs`; Postgres prepares
  the statement for its column types, MySQL uses `column_type()`, Oracle
  `data_type()`, SQLite the runtime type of the first non-NULL value (the
  driver does not expose `decltype` through our wrapper). `grid.rs` holds
  `display_text`, `column_widths` (first 1 000 rows plus name and type,
  capped), `visible_columns` (with a frozen first column), `sort_permutation`
  (type-aware, natural text order, NULLs last asc / first desc, stable, a
  `Vec<u32>` permutation) and the incremental `Search`. `copy.rs` renders a
  `Selection` as CSV, TSV, JSON, Markdown or SQL INSERT, quoting identifiers
  per dialect.
- **View** (`views/results.rs`). Drawn cell by cell rather than with ratatui's
  `Table`, so the gutter, dividers, right alignment, glyph runs and `…` cuts
  are exact. The type row only appears when some column has a type. The
  overflow hint takes its room from the columns, and a 1–2 cell sliver of a
  last column is dropped unless it is selected. Empty column names show as
  `?column?`. The search line takes the bottom line of the pane (toasts draw
  over it while they show) and says `match i / n` on a match.
- **Keys.** `:` stays the command palette (T206), so goto row is `ctrl-g`.
  `0`/`$`, `ctrl-u`/`ctrl-d`, `s`/`S`, `n`/`N`, `ctrl-y`, `<`/`>`/`=`, `F`
  are bound in the Results context. `w` and `Enter` belong to T304.
- **Tabs.** A new result is shown only if the last tab was; someone looking
  back at an earlier tab stays there, and the cap (20) keeps their tab.
- **Search** scans `SEARCH_BUDGET` = 10 000 rows at once and as much per tick
  after, so a 1M-row search spreads over 100 ticks.
- **Elapsed.** The worker fills `QueryResult::elapsed`; the title reads
  `N rows · T ms · sorted by X ▼`.
- **Not done here.** The `arboard` fallback and the `clipboard` config key
  (copying is OSC 52 only); the `copy_insert_round_trip` and
  `column_kinds_mapped` DB integration tests (T004); the
  `results_render_1m` gate (T006). The
  `prop_csv_round_trips_through_csv_reader` proptest reads the CSV back with
  a small reader in the test instead of the `csv` dev-dependency.

