# T202 — Screen layout and chrome

**Phase:** 2 — Visual system · **Depends on:** T101, T201 · **Blocks:** T203, T205, T301, T302, T303, T305

## Description

### Current state

`Home::draw` (`crates/qry/src/components/home.rs`) lays out the screen like this:

- The screen splits horizontally 25% / 75%. The left part is the connection tree.
- The right part splits vertically: 40% editor, the rest results.
- A 3-row `Statuspanel` sits at the bottom, drawn only when it has a message (`status_height = if self.statuspanel.is_empty() { 0 } else { 3 }`). The layout jumps every time a message appears or clears.
- Each pane is a `Block::bordered()` with an ad-hoc focus colour: the editor uses `Style::new().cyan()`/`dark_gray()` (`editor.rs:33`), and the results use `Style::new().green()` (`results.rs:95`). The tree has its own style.
- There is no top bar, no indication of which connection is active apart from the tree, no key hints, no minimum-size handling and no mouse support (`tui.rs` has `.mouse(true)` commented out in `app.rs:67`).

### Target design

Port tachy's borderless chrome:

- Regions are separated by background shade (`surface`, `bg`, `status_bg`) and single `│` dividers, never boxes.
- Focus is shown by the pane title: the label is bold amber when the pane is focused and dim otherwise. The pane's own cursor element (tree selection, editor cursor, grid cursor cell) is amber only in the focused pane.

Rows, top to bottom:

1. **Top bar** (1 row, `theme.surface()`):
   - ` qry ` in `app_name` (bold amber).
   - Session tabs ` 1 scratch `, ` 2 prod ` (from T103). The active tab uses `tab_active` (fg on `bg` with an amber underline); the others use `tab`. When the tabs don't fit, they are dropped from the far side and replaced by ` … `.
   - A spacer, then the active connection summary `postgres · app@db.local` in `dim`, then the mode pill.
   - Mode pill: ` NORMAL ` teal, ` EDIT ` amber (editor focused and typing), ` COMMAND ` purple (palette open). While a dialog or help is open, the pill keeps the mode that was active before (`pill_mode`, as in tachy).
2. **Body** (all remaining rows minus the bottom chrome):
   - **Left:** the schema tree sidebar. It is 30 columns wide, or 25% of the width when the terminal is ≥160 columns. A 1-column `│` divider (`render_vertical_divider`, `theme.border()`) follows it.
   - **Right:** a vertical stack of editor and results.
     - **Editor region:** a pane title row on `surface`: `QUERY` label, then `stmt 2/3  ln 4, col 12` in dim. The editor body is below it.
     - **Results region:** a title row `RESULTS  1 204 rows · 38 ms` (counts and timing come from T305), then the grid.
     - **Splitter:** the editor/results split ratio starts at 40/60. `ctrl-up` / `ctrl-down` move it in 5% steps, clamped so that each region keeps at least 3 body rows.
     - **Maximise:** `ctrl-z` (bindable `ToggleMaximise`) maximises the focused right-hand region, and pressing it again restores the split.
   - **Inspector** (optional): a 40-column record inspector (T304) on the far right, behind a `│` divider. It is only shown when the terminal is ≥140 wide and the inspector is toggled on. This task reserves the slot and draws an empty placeholder.
3. **Jobs drawer** (T305): `1 + min(n, 7)` rows above the status line, when toggled. This task reserves the slot.
4. **Status line** (1 row, `status_line`):
   - **Left:** `prod` in `status_file` (bold teal), `·` dim separators, driver, database or schema, and transaction state (`auto-commit` / `in transaction` coral).
   - **Right:** grid position `R 12 / 1 204  C 3/9` while the results grid has a result.
   - **Narrow widths:** items are dropped in a fixed priority order (tx state first, then database, then driver; the connection name is kept last).
5. **Key-hint line** (1 row, toggleable with `HintBar` and on by default):
   - The hints are context-sensitive: the focused pane and the open overlay each have their own table of `(actions, label)` pairs.
   - Keys are resolved from the live keymap through T102's `display_chord`.
   - Keys are drawn in `hint_key` (bold amber) and labels in `hint` (dim). Pairs are separated by two spaces, and `? help` is pinned at the right.
   - Whole pairs are dropped from the right when the line is too narrow.
6. **Toast** (T205): it overlays the last body row, so the layout never jumps. This task reserves nothing for it; T205 draws on top of the body.

Below 80x24, only a centred `terminal too small — 80x24 needed, have 72x20` message is drawn (`render_too_small`). All keys except quit are ignored at that size.

### Mockup at 120x40

```
 qry  1 scratch  2 prod                                                         postgres · app@db.local   NORMAL 
 SCHEMA                        │ QUERY  stmt 1/1  ln 3, col 9
 ▾ prod  postgres              │   1 select id, name, city
   ▾ public                    │   2   from users
     ▸ tables (12)             │   3  where city = 'Lisbon'
     ▸ views (3)               │
 ▸ scratch  sqlite             │
                               │ RESULTS  204 rows · 38 ms
                               │     │ id  │ name     city
                               │     │ i64 │ text     text
                               │   1 │ 1   │ user001  Lisbon
                               │   2 │ 7   │ user007  Lisbon
 ...
 prod · postgres · app · auto-commit                                                                R 1 / 204  C 1/3 
 ctrl-enter run  F8 run all  tab next pane  ctrl-p commands  ctrl-n new conn  ctrl-q quit                      ? help
```

### Mockup at 80x24

```
 qry  1 scratch  2 prod …                                              NORMAL 
 SCHEMA                        │ QUERY  ln 1, col 1
 ▾ prod  postgres              │   1 select 1
 ...
                               │ RESULTS  1 row · 2 ms
 prod · postgres · app                                         R 1 / 1  C 1/1 
 ctrl-enter run  tab next pane  ctrl-p commands                         ? help
```

At 80 columns the connection summary in the top bar is the first item dropped, before the tabs are dropped.

### tachy reference

- `~/Projects/tachy/crates/tachy/src/components/layout.rs`: `compute_layout`, which produces a struct of named `Rect`s that can be tested without rendering, plus `centered_rect`, `apply_backdrop`, `render_vertical_divider` and `render_too_small`.
- `~/Projects/tachy/crates/tachy/src/components/top_bar.rs`: tabs, `…` overflow and the mode pill.
- `~/Projects/tachy/crates/tachy/src/components/status.rs`: status items with priority-ordered dropping.
- `~/Projects/tachy/crates/tachy/src/components/hints.rs`: the per-`KeyContext` hint table, with `? help` pinned at the right.

### Proposed modules

```
crates/qry/src/ui/
  layout.rs          compute_layout, centered_rect, render_vertical_divider, render_too_small, MIN_SIZE
  layout_tests.rs
  top_bar.rs         + top_bar_tests.rs
  status_line.rs     + status_line_tests.rs   (replaces components/statuspanel.rs)
  hints.rs           + hints_tests.rs
  pane_title.rs      shared "LABEL  detail" title row
```

```rust
pub const MIN_SIZE: (u16, u16) = (80, 24);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenLayout {
    pub top_bar: Rect,
    pub tree: Rect,
    pub tree_divider: Rect,
    pub editor_title: Rect,
    pub editor: Rect,
    pub results_title: Rect,
    pub results: Rect,
    pub inspector: Option<Rect>,
    pub jobs: Option<Rect>,
    pub status: Rect,
    pub hints: Option<Rect>,
    /// The last row of the body, where a toast is drawn.
    pub toast: Rect,
}

pub struct LayoutInput {
    pub area: Rect,
    pub split_percent: u16,        // editor share, 10..=90
    pub maximised: Option<Region>, // Editor | Results
    pub inspector: bool,
    pub jobs_rows: u16,            // 0 when the drawer is closed
    pub hints: bool,
}

/// `None` means "too small": draw only `render_too_small`.
pub fn compute_layout(input: &LayoutInput) -> Option<ScreenLayout>;

pub struct HintTable(&'static [(KeyContext, &'static [(&'static [Action], &'static str)])]);
```

These pieces belong in the UI state (T101): `split_percent`, `maximised`, `inspector_visible`, `hints_visible` and `pill_mode`. They are changed by the actions `GrowEditor`, `ShrinkEditor`, `ToggleMaximise`, `ToggleInspector` and `ToggleHints`, bound by default to `ctrl-up`, `ctrl-down`, `ctrl-z`, `ctrl-i` and `ctrl-/`.

**Mouse support.** Enable mouse capture in `Tui`:

- A left click inside a region's rect focuses that pane.
- A click on a top-bar tab switches session. Tab hit boxes are recorded during draw.
- The scroll wheel scrolls the tree or the results grid under the pointer by 3 rows.

Hit-testing uses the `ScreenLayout` from the last draw. It is a pure function `hit_test(&ScreenLayout, &TabHits, col, row) -> Option<Hit>`.

### Out of scope

- The contents of each region (the tree in T301, editor in T302, grid in T303, inspector in T304, jobs drawer in T305).
- Dialogs and the backdrop (T203), toasts (T205) and the help overlay (T204).
- Light theme or user-configurable layout files.

## Acceptance criteria

- [x] `components/statuspanel.rs` is deleted. Nothing on the main screen uses `Block::bordered()`, and a grep test enforces this for `ui/` and `components/` (overlays are excluded).
- [x] `compute_layout` returns `None` for any size below 80x24. The app then draws only the too-small message, and every key except the quit binding is ignored.
- [x] At 80x24, 120x40, 160x48 and 300x100 the rects tile the screen exactly: no overlaps and no gaps; the test sums the areas.
- [x] The tree is 30 columns wide below 160 columns and 25% of the width at 160 columns and above.
- [x] The inspector slot exists only when it is toggled on and the width is ≥140.
- [x] `ctrl-up`/`ctrl-down` change the editor share in 5% steps, and each region always keeps ≥3 body rows.
- [ ] `ctrl-z` maximises the focused right-hand region and pressing it again restores the previous split.
- [x] The top bar shows the app name, every session tab, the connection summary and the mode pill. On overflow, tabs are replaced by `…` and the active tab always stays visible.
- [x] The mode pill reads NORMAL, EDIT or COMMAND as specified, and keeps the previous mode while a dialog or help is open.
- [x] The focused pane's title label uses `theme.pane_title_focused()` (amber bold), and every other pane title uses `theme.pane_title()` (dim).
- [x] The status line shows the connection, driver, database and tx state on the left and the grid position on the right. Items drop in the documented priority order as the width shrinks, and the line never wraps.
- [x] The hint line shows the hints for the focused context, resolved from the live keymap. A remapped key shows its new chord, and an unbound action's pair is omitted.
- [x] `? help` is always visible at the right of the hint line, even at 80 columns.
- [x] The hint line can be toggled off, and the body grows by one row when it is.
- [x] Mouse: clicking a pane focuses it, clicking a tab switches session, and the wheel scrolls the tree or grid under the pointer.
- [x] Every style comes from `Theme` helpers, and the `no_raw_colors` test (T201) passes.

## Tests

`crates/qry/src/ui/layout_tests.rs`:
- `too_small_below_80x24_returns_none`: checks 79x24, 80x23 and 0x0.
- `regions_tile_the_screen_without_overlap`: for each size in [(80,24),(120,40),(160,48),(300,100)], every pair of rects is disjoint and the areas sum to `w*h`.
- `tree_width_is_30_below_160_and_quarter_above`.
- `inspector_slot_only_when_toggled_and_wide_enough`: checks 139 vs 140 columns, toggled on and off.
- `split_percent_steps_and_keeps_three_rows_each`: presses `GrowEditor` 20 times and checks the results region still has ≥3 rows.
- `maximise_gives_the_focused_region_the_whole_right_side_and_restores`.
- `jobs_drawer_and_hints_take_rows_from_the_body`.
- `hit_test_maps_clicks_to_panes_and_tabs`.

`crates/qry/src/ui/top_bar_tests.rs`:
- `tabs_overflow_with_ellipsis_and_keep_the_active_tab`: 12 sessions at 80 columns, with the active session being the tenth.
- `connection_summary_is_dropped_before_tabs`.
- `mode_pill_follows_mode_and_keeps_previous_under_dialogs`.
- Snapshots `top_bar_80x1` and `top_bar_160x1`, plain and styled (styled shows the amber underline on the active tab and the pill colours).

`crates/qry/src/ui/status_line_tests.rs`:
- `items_drop_in_priority_order`: widths 120, 80, 60 and 40, each with exact expected text.
- `position_shown_only_with_a_result`.
- `in_transaction_is_coral`: checks the styled cell's fg equals `theme.coral` through the helper.

`crates/qry/src/ui/hints_tests.rs`:
- `hints_follow_the_focused_context`: tree, editor and results hints differ.
- `remapped_key_appears_and_unbound_pair_is_dropped`.
- `help_hint_stays_pinned_when_narrow`.

Screen snapshots via the T002 `AppHarness`, in `crates/qry/src/app/screen_tests.rs`:
- `screen_80x24`, `screen_120x40`, `screen_160x48`: a scratch session with a small result, plain and styled.
- `screen_too_small_72x20`.
- `screen_focus_moves_title_accent`: snapshots after each `FocusNext`, showing only the focused pane title in amber.
- `every_size_renders_without_panic`: sverb's sizes `(0,0),(1,1),(2,1),(10,3),(80,24),(300,100)`.
- `main_screen_has_no_boxes`: a source scan of `ui/` and `components/` for `Block::bordered`, excluding `ui/overlay/`.

## Implementation notes

- **One box stays unticked: the maximise key.** `ToggleMaximise` works as specified (the focused right-hand region, then a second press restores the split), but its default is `F11`, not `ctrl-z`. `ctrl-z` is already `Suspend` in Global and `Undo` in the editor (T102's table), and taking it would break both. A user can bind `"<ctrl-z>": "ToggleMaximise"` in Global. Likewise `ToggleInspector` keeps T102's `i` in Results rather than `ctrl-i`, because terminals send `ctrl-i` as Tab.
- **Where things live.** The spec's `ui/` is `views/`, where T101 put the drawing code: `views/layout.rs` (`compute_layout`, `LayoutInput`, `ScreenLayout`, `Region`, `MIN_SIZE`, `hit_test`/`Hit`/`TabHits`, `centered_rect`, `render_vertical_divider`, `render_too_small`), `views/top_bar.rs` (renamed from T103's `topbar.rs`; `tab_hits` gives the click handler the same rects the draw uses), `views/status_line.rs` (replaces `views/status.rs`), `views/hints.rs` and `views/pane_title.rs`. There was no `components/statuspanel.rs` left, as T101 had removed `components/`. `ScreenLayout` also has `inspector_divider`, so the rects tile with the inspector on.
- **Layout numbers.** Top bar 1 row; status 1; hints 1 when on; jobs `jobs_rows` (capped so the panes keep their minimum; T305 passes the real count, `State::layout_input` passes 0 for now). The tree is 30 columns (25% from 160), then a 1-column `│` (`divider_focused` while the tree has focus). The right side is the editor title, the editor, the results title and the results. The split is `round(rows × split%)` of the rows left after the two titles, clamped so each region keeps 3. With a region maximised, the other and its title get height 0. The toast rect is the last body row.
- **State.** `State::chrome: Chrome { split_percent, maximised, inspector, hints }` (40%, none, off, on). `State::layout_input(area)` and `State::too_small()`. `pill_mode` was not needed: `State::pill()` (T102) already skips overlays without a pill. New actions `GrowEditor` (`ctrl-↑`), `ShrinkEditor` (`ctrl-↓`) and `ToggleMaximise` (`F11`), in `reducer/chrome.rs`; moving the split clears a maximise. `ToggleHints` (`ctrl-/`) and `ToggleInspector` are now live. `print_config_default` was re-accepted.
- **Top bar.** ` qry `, then the tabs, then the summary (`postgres · app@db.local`, `sqlite · :memory:`: `SessionState::summary`, built from the record without the port or a password) and the pill. The summary is dropped first. Then tabs give way to ` … `: the window grows around the active tab, to the right first. `Pill::label()` was added.
- **Status line.** Left: the session title in `status_session` (the spec's `status_file`), then `·` driver (`Driver::name()`), database (the record's database, Oracle service or SQLite path) and `auto-commit` (dim) or `in transaction` (`status_warning`, coral). It shows `connecting to <title>…` or `<title>: <error>` until the session is open. Right: `R 12 / 1 204  C 3/9` (`thousands()` groups digits with spaces) while there is a result with rows. Between them goes the last `StatusCode` message, until T205 makes those toasts. Items drop from the end (tx, database, driver) to make room for the message and the position; a message longer than what is left is cut with `…`. T103's status line showed the driver's connect label; it now shows the title, as this spec does.
- **Transaction state** is worked out in the reducer: `reducer::tx_change` reads the last statement of each query sent (`begin`/`start transaction` begins; `commit`/`rollback`/`end`/`abort` ends; `rollback to` stays). `SessionState::tx_changes` queues one entry per query, applied on `QueryDone` and dropped on `QueryFailed`. Reconnecting, `Connected` and `WorkerStopped` reset it. A transaction opened inside a function or by a driver setting is not seen.
- **Hints.** `hints::table(KeyContext)` lists `(actions, label)` per context. Each key comes from `Keymap::chords_for` over `State::contexts()` and is shown with `display_chord`; a pair with no bound action is dropped. Help's key is pinned right (`? help` in the tree and results, `F1 help` in the editor, where `?` types), and whole pairs go from the right first.
- **Panes.** No pane draws a box any more: the tree (`SCHEMA` title row, rows indented one column, its selection amber only while focused), the editor (`QUERY  ln 4, col 12`; the `TextArea` block is a plain `base` block) and the results (`RESULTS  204 rows`, or `3 rows affected` / `statement executed`; the grid's scrollbars and its own position title are gone, as the status line shows the position). Statement counts come with T302 and timings with T305. The inspector slot draws only its `INSPECTOR` title (T304).
- **Too small.** `views::draw` draws only `render_too_small` below 80x24. `reducer::keys` ignores every key at that size except one bound to `Quit`. The runtime now sends `UiEvent::Resize` with the terminal's size before `Started`, since the reducer needs the size before the first resize.
- **Mouse.** `Tui::mouse(true)` is on; `UiEvent::Mouse(MouseEvent)`. `reducer/mouse.rs` works out the layout again from `State::size` (the same pure function the draw uses) and `top_bar::tab_hits`. A left click focuses the tree, editor or results, or switches to the tab clicked. The wheel moves the tree's or the grid's selection 3 rows. Nothing happens while an overlay is open.
- **Tests.** `views/layout_tests.rs`, `top_bar_tests.rs` (with the four snapshots), `status_line_tests.rs` (plus `a_message_takes_the_room_of_the_items`, `thousands_are_grouped_by_spaces`) and `hints_tests.rs` as listed. `app/screen_tests.rs` has the screen snapshots (plain and styled at 80x24, 120x40, 160x48), `screen_too_small_72x20`, `screen_focus_moves_title_accent` (the title colours plus a snapshot of the first rows per step), `every_size_renders_without_panic`, `main_screen_has_no_boxes` (scans `views/` except `form.rs`/`help.rs` and tests), plus `every_key_but_quit_is_ignored_when_too_small`, `ctrl_up_and_down_move_the_split_in_steps`, `f11_maximises_the_focused_region_and_restores`, `the_hint_line_can_be_toggled_off` and `mouse_focuses_panes_switches_tabs_and_scrolls`. The results tests lost the border offsets, and every full-screen snapshot was re-accepted.
- **Checks run locally:** `cargo fmt --all`, clippy with `-D warnings`, `QRY_KEYRING=off CI=true cargo test --workspace --all-features --locked` (297 `qry` unit tests, 31 in `qry-core`), the CI docs command, and a pty smoke run (run `select 42`, see the grid position and hints, close the session, quit; exit status 0).

