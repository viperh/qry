# T304 — Record inspector and value popup

**Phase:** 3 — Features · **Depends on:** T303 · **Blocks:** —

## Description

### Current state

qry has no way to see a full cell value. `results.rs` truncates every column at 40 cells, and a JSON document, a long text, a multi-line value or a binary blob cannot be read or copied whole. There is also no row-as-record view, which matters for wide tables (30+ columns) where only a few columns fit on screen.

### Target design

There are two pieces, both modelled on tachy's inspector (`tachy/crates/tachy/src/components/inspector.rs` and its value popup):

1. **The inspector**, a right-hand panel exactly 40 columns wide.
2. **The value popup**, a teal dialog for one cell.

#### 1. Inspector panel

```
 … results grid …                         │ COLUMN
                                          │ email · text · col 3/9
                                          │ nulls 4 / 42 · distinct 38
                                          │
                                          │ TOP VALUES · 42 fetched rows
                                          │ alice@example.com   ▉▉▉▉▏   3
                                          │ bob@example.com     ▉▉▋     2
                                          │
                                          │ RECORD · row 3 / 42
                                          │ id          3
                                          │ name        carol
                                          │ email       carol@example.com
                                          │ created_at  2026-10-03 11:30:00
                                          │ notes       first line↵second …
```

**When it shows:**

- It is shown when the terminal is **at least 140 columns** wide and the inspector is toggled on with `i` (Results context). The toggle persists for the session.
- Below 140 columns, `i` opens the inspector as a full-height overlay on the right half instead, so it stays usable at 80×24.
- It sits behind a 1-column `│` divider (`render_vertical_divider` from T202).

**Sections.** Each has an uppercase dim label and a blank line between sections:

- **COLUMN:**
  - `name · type · col N/M` for the cursor column.
  - Null count and distinct count, computed over the fetched rows and capped at 100 000 rows. Counting runs incrementally on ticks, with a teal spinner until done.
- **TOP VALUES:**
  - The five most frequent values of the column among fetched rows, with eighth-block bars (`▏▎▍▌▋▊▉█`) in `bar_fill` on `bar_track`, and counts.
  - It is hidden for columns where every value is distinct.
- **RECORD:** every column of the selected row, as `field  value`.
  - Field names are `record_field` (dim) and values `record_value` (muted). NULL is shown as dim italic.
  - Values are truncated to the panel width with `…`, and control characters are shown as in T303.
  - It scrolls when focused.

**Focus.** `Tab`/`F6` cycles focus into the inspector (T102 Inspector context). When focused:

- Section labels turn from `inspector_label` to `inspector_label_focused` (amber bold), as in tachy.
- `↑↓` moves a selection within RECORD, `Enter` opens the value popup for that field, and `y` copies that value.
- `Esc`/`Tab` returns focus to the grid.

**Describe mode** (from T301 `d`). The COLUMN and TOP VALUES sections are replaced by **TABLE · schema.name**, a list of columns with type, PK/FK and nullability from the `SchemaCache`. `Esc` returns to the normal sections.

#### 2. Value popup

```
            ┌ users.notes · text · row 3 ───────────────────────────┐
            │ first line                                            │
            │ second line with a long sentence that wraps across    │
            │ the dialog width without cutting words in half        │
            │                                                       │
            └─ y copy  w wrap  j/k scroll  f format  Esc close ── 1–4 of 4 ┘
```

**Frame and size:**

- `Enter` on a grid cell (or on a RECORD field) opens it.
- It is drawn with `dialog_frame(accent = teal)` from T203, over the backdrop.
- Size is 80% of the screen, at most 120 × 40, centred.

**Rendering by kind:**

- **Text:** word-wrapped (toggle `w` to no-wrap with horizontal scroll `h`/`l`).
- **JSON:** detected when `ValueKind::Json` or when the text parses as an object or array. It is pretty-printed with 2-space indent, using the T302 colours (keys fg, strings green, numbers purple, `true`/`false`/`null` teal). `f` toggles raw / pretty.
- **Binary** (`ValueKind::Binary`, or invalid UTF-8 hex coming from a backend): a hex dump, 16 bytes per line with offset and ASCII column (`00000010  48 65 6c 6c …  |Hell…|`).
- **NULL:** a centred dim italic `NULL`.

**Keys:**

- `y` copies the **full untruncated** value (OSC 52 / arboard, T303 helper). A toast confirms.
- `j/k`, PgUp/PgDn and `g/G` scroll.
- The bottom border shows the keys plus ` a–b of n ` when scrollable, the same pattern as the T204 help overlay.
- `Esc`/`q` close.

### qry-core split

`crates/qry-core/src/inspect.rs` holds pure logic:

```rust
pub struct ColumnProfile { pub nulls: u64, pub distinct: u64, pub top: Vec<(String, u64)>, pub sampled: u64, pub complete: bool }
pub struct Profiler { /* incremental state */ }
impl Profiler {
    pub fn new(col: usize, cap: usize) -> Self;
    pub fn step(&mut self, result: &QueryResult, budget: usize) -> Option<&ColumnProfile>; // Some when complete
}
pub enum Rendered { Text(Vec<String>), Json(Vec<Vec<(JsonTok, String)>>), Hex(Vec<String>), Null }
pub fn render_value(value: Option<&str>, kind: ValueKind, width: u16, wrap: bool, pretty: bool) -> Rendered;
pub fn hex_dump(bytes: &[u8]) -> Vec<String>;
pub fn wrap_words(text: &str, width: usize) -> Vec<String>; // unicode-width aware
```

Profiling is keyed by `(result id, column)`. Moving the cursor to another column cancels the old profiler and starts a new one.

### Edge cases

- A 10 MB value: the popup wraps lazily (only visible lines are computed), and copy copies everything.
- A value that is invalid JSON but starts with `{` is shown as text, with no error.
- Huge distinct counts are approximated after the cap and labelled `sampled 100 000 rows`.
- The inspector open while a new result arrives resets to the new result's first row.
- Terminal resizes across 140 columns switch between docked and overlay modes without losing focus state.

### Out of scope

Editing values, image previews, histograms for numeric columns (a follow-up may add min/max/mean).

## Acceptance criteria

- [x] `i` toggles a 40-column docked inspector at ≥140 columns and an overlay below that.
- [x] COLUMN shows name, type, position, nulls and distinct, computed incrementally with a spinner, without blocking input on 1M rows.
- [x] TOP VALUES shows up to five values with eighth-block bars, and is hidden when every value is distinct.
- [x] RECORD lists every field of the selected row and follows the grid cursor.
- [x] Focused section labels are amber bold; unfocused ones are dim.
- [x] RECORD navigation, `Enter` to the popup and `y` to copy work when the inspector is focused.
- [x] Describe mode shows table structure from the schema cache. `Esc` leaves it.
- [x] The value popup renders text (wrap/no-wrap), pretty JSON (toggle `f`), hex for binary, and NULL. `y` copies the full value.
- [x] The popup's bottom border shows its keys and a scroll position, as in the T204 help overlay.
- [x] All styles come from `Theme` helpers.

## Tests

**Unit tests** — `crates/qry-core/src/inspect_tests.rs`:

- Profiling:
  - `profile_counts_nulls_and_distinct`
  - `profile_top_values_sorted_by_count_then_value`
  - `profile_step_respects_budget`
  - `profile_cap_marks_sampled`
- Rendering:
  - `render_json_pretty_two_space`
  - `render_invalid_json_falls_back_to_text`
  - `hex_dump_offsets_and_ascii_column`
  - `wrap_words_respects_unicode_width`
  - `wrap_words_breaks_overlong_word`
  - `render_null_is_null_variant`

**Proptests:**

- `prop_wrap_words_never_exceeds_width_and_preserves_text`: joining the output lines without separators equals the input with whitespace collapsed.
- `prop_hex_dump_line_count`

**Fuzz:** `fuzz/fuzz_targets/render_value.rs`, with arbitrary bytes for every `ValueKind`. Its twin is in `inspect_tests.rs`.

**Unit tests** — `crates/qry/src/components/inspector_tests.rs`:

- Layout: `docked_at_140_overlay_below`.
- Focus and navigation:
  - `focus_switches_label_style`
  - `record_follows_grid_cursor`
  - `enter_on_record_opens_popup`
- Describe mode: `describe_mode_from_tree_and_esc_returns`.
- Popup keys: `popup_y_copies_full_value_not_truncated`, `popup_f_toggles_pretty_json`, `popup_w_toggles_wrap`.
- `new_result_resets_record_row`

**Snapshots** — plain and styled:

- `inspector_docked_160x40`
- `inspector_overlay_80x24`
- `inspector_focused_160x40`
- `inspector_describe_160x40`
- `inspector_profiling_spinner_160x20`
- `popup_text_wrapped_120x40`
- `popup_json_pretty_120x40`
- `popup_hex_120x30`
- `popup_null_80x24`
- `popup_scrolled_80x24`

**AppHarness flow:** run a query returning a JSON column, then `h.keys("enter f y esc")`. Assert a clipboard effect with the exact full value, a toast `copied value`, and a popup that is closed.

## Implementation notes

- **qry-core** (`inspect.rs`). `ColumnProfile`, `Profiler` (`new`, `step`,
  plus `column`, `profile` and `steps` for the view), `Rendered`,
  `JsonTok`, `render_value`, `hex_dump`, `wrap_words` as specified, and
  `is_json` / `binary_bytes`. NULLs never count as top values. JSON is
  parsed by a small hand-written parser that keeps key order and number
  text as written (no new dependency); nesting past 256 levels is shown as
  text. Binary text from the backends (`x'…'` from SQLite and Oracle,
  `\x…` from Postgres, `0x…`) is decoded before the dump; anything else is
  dumped as its bytes. `wrap_words` collapses whitespace runs, and a
  character wider than the line still gets a line of its own.
- **State.** `Focus::Inspector`, `Overlay::Value(ValuePopup)`, and
  `ResultsState::{profile, record_scroll}` (reset with each new result, so
  a new result starts on its first row). The value popup uses the `Help`
  context for scrolling and Esc, like the Messages dialog, and its own
  letters (`y w f h l g q`) are unbound shortcuts.
- **Reducer** (`app/reducer/inspector.rs`). `i` toggles the panel (hiding
  it while focused gives the grid the focus back). Tab goes Results →
  Inspector while it is shown, and Tab, shift-Tab or Esc go back to the
  grid. The RECORD selection *is* the grid's cursor column, so the panel
  always follows the grid: `↑↓` in the inspector move the grid's column
  and `←→` its row. Profiling runs `PROFILE_BUDGET` = 20 000 rows after
  every event (ticks included) while the panel is shown, up to
  `PROFILE_CAP` = 100 000 rows, so a 1M-row result takes five events and
  never blocks input; moving to another column starts a new profiler. The
  popup is laid out in the reducer when it opens, on `w`/`f` and on resize
  (its size follows from `State::size`), not on every frame.
- **Views.** `views/inspector.rs` (docked in the T202 slot from 140
  columns, or over the right half of the body below that, behind a `│`
  divider in either case) and `views/value_popup.rs` (`dialog_frame` with
  the teal accent, 80% of the screen capped at 120×40, keys and
  ` a–b of n ` in the bottom border as the help overlay draws them). JSON
  uses the SQL helpers (`sql_identifier` keys, `sql_string`, `sql_number`,
  `sql_keyword` literals, `sql_punct`). The bars use `gauge(theme.teal)`
  (teal fill on `track`) rather than new `bar_fill`/`bar_track` helpers.
  Inspector hints: `j field  Enter value  y copy  Esc grid`.
- **Keys.** Inspector context gained `Enter` Inspect, `y` CopyCell, `i`
  ToggleInspector and `Esc` FocusResults (`print_config_default` and the
  help snapshots were re-accepted); the help's Inspector group has a note
  listing the popup's letters. Copying a NULL copies nothing and says so.
- **Deviations.** Tests live in `views/inspector_tests.rs`,
  `views/value_popup_tests.rs` and `app/reducer/inspector_tests.rs` (there
  is no `components/`). The spinner snapshot is `160x24`: 160×20 is below
  the 80×24 minimum and draws only the too-small message. JSON lines are
  never wrapped (pan with `h`/`l`). A huge value is laid out whole once
  per open/toggle/resize rather than lazily line by line.
- **Describe mode** (added once T301 landed). `Action::Describe`, bound to
  `d` in the Tree (on a table or view, or a row under one) and the Results
  (when the active tab's statement read one table: `tab.table`, with no
  JOIN, second FROM item or UNION, checked by
  `reducer::inspector::reads_one_table`, resolved against the loaded
  objects case-insensitively, else the given or default catalog with the
  name folded as the server folds it). It sets
  `SessionState::describe: Option<Describe { catalog, object, back,
  scroll }>`, shows the inspector and focuses it. The columns and indexes
  come from the session's `SchemaCache`; when they are not there,
  `tree::after`'s `ensure_loaded` asks for them through `LoadSchema` like
  an open node (so DDL or a reconnect reloads them), after any catalogs or
  objects load above them, whose answer would drop them; a failed load is
  asked for again by pressing `d` again. The panel then shows
  `TABLE · catalog.name` (`VIEW` for views) with one line per column:
  name, type, `PK` amber, `FK → t.col` purple, `NULL` dim, `= default`
  dim (the tree's `tree_*` Theme helpers), then `INDEXES · n` with
  `name  (cols)  UNIQUE`; a teal spinner (the tree's frame) and
  `loading…` while it loads, `! error` and `d tries again` if it failed.
  Lines are cut at the panel with `…`. Focused, `↑↓`/`g g`/`G` scroll it
  and the hints read `j scroll  Esc back`; `Esc` leaves describe mode and
  returns the focus to where `d` was pressed; hiding the panel with `i`
  ends it too. Deviation: RECORD is not drawn in describe mode, so the
  structure has the panel's height (the described table and the grid's
  result are often unrelated). Tests: `describe_mode_from_tree_and_esc_returns`
  and eight more in `app/reducer/inspector_tests.rs` (cache hit, no table,
  retry, cleared cache, scrolling, from the results, the one-table check);
  `views/inspector_tests.rs` has `inspector_describe_160x40` (the spec's
  snapshot), `inspector_describe_overlay_80x24` and
  `inspector_describe_loading_80x24`, plain and styled, plus a view and a
  failure and line cutting. Re-accepted: help, palette and
  `print_config_default` snapshots, and `docs/keybindings.md`.
- **Not done.** The `fuzz/fuzz_targets/render_value.rs` target: there is no `fuzz/` crate
  until T005; its twin, `fuzz_render_value`, is in `inspect_tests.rs`. A
  mouse click on the inspector does not focus it. No `arboard` fallback
  (copying is OSC 52, as in T303).
