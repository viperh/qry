# T201 — Theme system ported from tachy

**Phase:** 2 — Visual system · **Depends on:** T001 · **Blocks:** T202, T203, T204, T205, T206, T302, T303, T304

## Description

### Current state

Colours are hard-coded in every component:
- `components/editor.rs` `restyle`: `Style::new().cyan()` for the focused border, `dark_gray()` otherwise, `reversed()` for the selection.
- `components/results.rs`: a green focused border and a `black().on_green()` cursor cell; its test `the_selected_cell_is_highlighted` asserts `Color::Green`.
- `components/form.rs`: cyan/dark_gray field borders.
- `components/help.rs`: bold titles.
- `statuspanel.rs`: its own colours.

`config.rs` still carries the template's `Styles(HashMap<Mode, HashMap<String, Style>>)` and `parse_style`/`parse_color`, and `.config/config.json` has an empty `"styles": {"Home": {}}`. None of it is used. There is no `NO_COLOR` support and no 256-colour fallback.

### Target design

Port tachy's theme module (`~/Projects/tachy/crates/tachy/src/theme.rs`) to `crates/qry/src/theme.rs`. Keep its structure and add helpers for qry's surfaces.

**Palette** (`Theme::DARK`, exact values from tachy; 256-colour indices from tachy's own tests):

| field | hex | xterm-256 | role in qry |
|---|---|---|---|
| bg | #0E1116 | 233 | app background, even grid rows, text on accent pills and cursor |
| bg_alt_row | #10141A | 233 | odd striped grid rows (stripes vanish in 256 mode) |
| surface | #131820 | 234 | top bar, grid header, sidebar, pane title rows |
| surface_raised | #161C25 | 234 | dialog, palette and help bodies, buttons |
| status_bg | #1A212B | 235 | status line |
| selection_bg | #1B2533 | 235 | selected grid row, selected tree row, editor selection |
| dialog_title_bg | #1E2632 | 235 | dialog title background |
| track | #1E2530 | 235 | gauge and progress empty part |
| border | #232A34 | 236 | dividers, column `│` |
| border_inner | #2C3542 | 237 | inner borders (input fields) |
| border_strong | #3A4452 | 238 | divider after frozen columns, focused pane divider |
| fg | #D3D9E0 | 253 | main text |
| fg_muted | #A9B4C2 | 249 | secondary text, inspector values |
| fg_dim | #8A94A3 | 246 | labels, types, hints, gutter, NULL, comments, backdrop |
| amber | #F2B04A | 215 | "you are here / act": cursor cell, focus, keys, active tab, EDIT pill |
| amber_bright | #FFD08A | 222 | reserved |
| amber_tint | #3A2F17 | 235 | search-match background |
| teal | #4FC1C9 | 74 | data and info: NORMAL pill, primary button, help accent, spinner, SQL keywords and operators, open-session dot |
| purple | #C7A6FF | 183 | command and jobs: COMMAND pill, palette border, jobs count, SQL numbers |
| purple_tint | #2A2340 | 236 | selected palette item |
| green | #9FD38A | 150 | success `✓`, SQL strings |
| coral | #F28B6B | 209 | errors and warnings (one colour for both), failed session `✗`, SQL error underline |

Keep tachy's doc comment on accent meanings at the top of the file. Every new helper must pick its accent by that rule.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme { /* the 22 colour fields above */, pub monochrome: bool }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSupport { TrueColor, Ansi256, NoColor }

impl ColorSupport {
    /// NO_COLOR non-empty → NoColor; COLORTERM=truecolor|24bit → TrueColor; else Ansi256.
    pub fn detect() -> Self;
    pub fn from_env(no_color: Option<&str>, colorterm: Option<&str>) -> Self; // testable core
}

impl Theme {
    pub const NAMES: &[&str] = &["dark"];
    pub const DARK: Theme = /* exact values */;
    pub fn by_name(name: &str) -> Option<Theme>;
    /// Rgb to nearest xterm-256 (indices 16–255, redmean distance); nudge selection_bg/status_bg
    /// by ±1 when they collapse onto bg; NoColor → every colour Reset and monochrome = true.
    pub fn adapt(self, support: ColorSupport) -> Theme;
    fn pick(&self, colored: Style, mono: Modifier) -> Style;
}
```

**Helpers.** Drawing code calls only these, each with a monochrome fallback through `pick`:
- **Base:** `base`, `surface`, `muted`, `dim`, `border`, `border_strong`, `spinner`.
- **Top bar (T202):** `app_name` (amber bold), `tab`, `tab_active` (fg on bg, amber underline), `tab_running` (teal), `mode_pill(Pill)` (NORMAL teal, EDIT amber, COMMAND purple; fg is bg, bold).
- **Panes:**
  - `pane_title` (fg_dim on surface) and `pane_title_focused` (amber bold on surface). Panes are separated by background and `│` dividers, not boxes; focus is shown by the title.
  - `divider` and `divider_focused`.
- **Tree (T301):**
  - `tree_connection`, `tree_connection_selected`, `tree_object` (fg_muted), `tree_kind` (fg_dim: `table`, `view`).
  - `session_dot(SessionStatus)`: `●` teal, `◌` dim, `✗` coral.
  - `driver_badge(Driver)`: a short dim label `pg`, `my`, `ma`, `ora`, `lite`. No brand colours.
- **Grid (T303):**
  - `header`, `header_type`, `header_active`, `sort_indicator`.
  - `gutter`, `gutter_selected`.
  - `row(i)`, `row_selected`, `cursor_cell` (bg on amber bold / REVERSED|BOLD).
  - `null_value` (fg_dim italic), `match_highlight`, `column_divider`, `frozen_divider`, `overflow_hint`.
- **Inspector (T304):** `inspector_label`, `inspector_label_focused`, `record_field`, `record_value`, `value_changed`.
- **Status line:** `status_line`, `status_session` (teal bold), `status_dim`, `status_warning` (coral), `status_jobs` (purple bold), `gauge(fill)`.
- **Hints:** `hint` (dim) and `hint_key` (amber bold).
- **SQL highlighting (T302):**
  - `sql_keyword` and `sql_operator` (teal), `sql_string` (green), `sql_number` (purple).
  - `sql_comment` (fg_dim italic), `sql_identifier` and `sql_punct` (fg), `sql_quoted_ident` (fg_muted).
  - `sql_error` (coral, coral underline, UNDERLINED).
  - `editor_cursor_line`, `editor_selection`, `line_number`, `line_number_current`.
- **Overlays (T203–T206):**
  - `dialog`, `dialog_border(accent)`, `dialog_title(accent)`.
  - `field`, `field_focused` (the `› ` marker in amber), `field_placeholder`.
  - `button`, `button_primary` (bg on teal bold).
  - `palette_border`, `palette_item_selected`, `palette_match`.
  - `backdrop`: applied to every cell behind a modal without clearing it.
  - `toast(level)`.
- **Jobs (T305):** `job(kind)` and `job_state(state) -> (Style, &'static str)` with glyphs `✓ ✗ – … ▸`.

**Selection and wiring:**
- Config key `"theme": "dark"` at the top level of `.config/config.json`, plus a `--theme <name>` CLI flag (`cli.rs`, validated against `Theme::NAMES`; the CLI wins).
- An unknown name is a startup error listing the valid names.
- Resolve once at startup: `Theme::by_name(name).adapt(ColorSupport::detect())`. Store the result in `State` (T101); views get `&Theme`.
- `qry --version` prints the detected colour support.

**Removals:**
- `config.rs` `Styles`, `parse_style`, `parse_color` and their tests.
- The `"styles"` config section.
- An existing user config that still has `"styles"` loads, with a one-line warning that it is ignored.

**Guard test.** Copy tachy `tests/no_raw_colors.rs`. It scans `crates/qry/src/**/*.rs` and fails on `Color::Rgb`, `Color::Indexed` or any named `Color::` variant, and on the ratatui `Stylize` colour shorthands (`.cyan()`, `.green()`, `.on_green()`, `.dark_gray()`, `.red()`, …), outside `theme.rs`.

### Out of scope

- Light theme and user-defined palettes. `NAMES` stays `["dark"]`, but the struct and `by_name` leave room for both.
- Layout changes (T202).

## Acceptance criteria

- [x] `crates/qry/src/theme.rs` defines `Theme::DARK` with exactly the 22 hex values above.
- [x] `ColorSupport::from_env` implements the NO_COLOR > COLORTERM > Ansi256 precedence; `NO_COLOR=""` (empty) does not disable colour.
- [x] `adapt(Ansi256)` produces only `Color::Indexed(16..=255)` and keeps selection_bg/status_bg distinct from bg.
- [x] `adapt(NoColor)` produces only `Color::Reset` and every helper returns a modifier-only style.
- [x] Every existing component (editor, results, tree, forms, help, status) draws through helpers; `tests/no_raw_colors.rs` passes and fails when a raw colour is added (proved by a fixture).
- [x] `"theme"` config key and `--theme` flag work; an unknown name errors with the list of names.
- [x] `Styles`, `parse_style` and the `styles` config section are gone; an old config with `styles` still loads, with a warning.
- [x] `qry --version` shows the colour support.
- [x] `results.rs` test `the_selected_cell_is_highlighted` asserts `theme.cursor_cell()` instead of `Color::Green`.

## Tests

`crates/qry/src/theme_tests.rs`:
- `dark_palette_exact_values`: a table test for all 22 fields against the hex values above.
- `color_support_precedence` (table): `(Some("1"), Some("truecolor"))` gives NoColor; `(Some(""), Some("truecolor"))` gives TrueColor; `(None, Some("24bit"))` gives TrueColor; `(None, None)` gives Ansi256; `(None, Some("xterm"))` gives Ansi256.
- `ansi256_indices_match_tachy`: `adapt(Ansi256)` maps bg→233, surface→234, status_bg→235, border→236, fg→253, fg_dim→246, amber→215, teal→74, purple→183, green→150, coral→209.
- `ansi256_only_indexed_16_to_255`: every field is `Indexed(n)` with `n >= 16`.
- `ansi256_keeps_selection_distinct_from_bg`.
- `nocolor_is_reset_and_monochrome`: every field is `Reset`, and `cursor_cell()` has REVERSED|BOLD and no fg or bg.
- `every_helper_has_mono_fallback`: call every helper on the NoColor theme and assert fg and bg are `None` or `Reset` (a list of helper closures).
- `mode_pill_accents`: NORMAL is teal, EDIT amber, COMMAND purple, with fg = bg.
- `job_state_glyphs`.

`crates/qry/tests/no_raw_colors.rs`:
- `no_raw_colors_outside_theme`: the source scan described above.
- `scanner_catches_raw_color_fixture`: runs the scanner on `tests/fixtures/raw_color.rs.txt` and asserts it reports line numbers.

`crates/qry/src/cli_tests.rs` and `config_tests.rs`:
- `unknown_theme_is_an_error_listing_names`.
- `cli_theme_overrides_config`.
- `legacy_styles_section_warns_and_loads`.

Snapshots, styled (sverb `styled()` format: text plus `--- styles ---` runs) in `crates/qry/src/views/snapshots/`:
- `theme_swatch_truecolor_80x24`, `theme_swatch_ansi256_80x24` and `theme_swatch_nocolor_80x24`: a debug view (test-only) that renders one line per helper with a sample. Changes to the palette show up in review.

## Implementation notes

- **Where the theme lives before T101.** There is no `State` yet, so `App` holds the resolved `theme: Theme` and passes `&Theme` down at draw time: `Component::draw(&mut self, frame, area, theme)`, `Form::render(frame, area, theme)`, `Help::draw(frame, area, theme)` and `TextInput::render(frame, area, focused, theme)`. T101 moves the field into `State` and keeps the `&Theme` parameter on the view functions. The editor restyles its `TextArea` on every draw (it used to on `set_focus`), because the theme only arrives with `draw`.
- **Resolution.** `theme::resolve(cli, config, support) -> eyre::Result<Theme>` picks `--theme`, then the config's `"theme"`, then `dark`, and adapts it. `App::new(tick_rate, frame_rate, theme_flag)` calls it with `ColorSupport::detect()`, so an unknown config name fails before the terminal is touched (`unknown theme `neon`; the themes are: dark`). `--theme` uses clap's `PossibleValuesParser::new(Theme::NAMES)`, so a bad flag is a clap error (exit 2) that lists the names. The flag wins, so a bad config value is never looked at when the flag is given.
- **`qry --version`** ends with `Colour support: 24-bit (truecolor)`, `256 colours` or `none (NO_COLOR)` (`ColorSupport`'s `Display`).
- **Legacy `styles`.** `Config` keeps a private `styles: Option<serde::de::IgnoredAny>` only to notice the section. `Config::warnings()` returns one line, and `App::new` logs it with `tracing::warn!` and prints it to stderr as `qry: …` before the TUI starts. T205 can turn these into toasts. The embedded `.config/config.json` now has `"theme": "dark"` and no `"styles"`.
- **T005 must drop `fuzz_style_parse`**: `parse_style` is gone. The T005 text still mentions it.
- **`ThemeEnv`** moved to `theme.rs`, and `qry::testing` re-exports it. It gained `from_process()`, `truecolor()`, `no_color()`, `support()` and `adapt(theme)`. `Default` is the empty environment, which gives 256 colours. **`AppHarness` draws in truecolor by default** (`ThemeEnv::truecolor()`), so styled snapshots show the palette's hex values. `with_theme_env` now really changes the theme, and `AppHarness::theme()` returns the one it draws with. `ColorSupport::from_env(no_color, colorterm)` takes the two values, as this task specifies, rather than tachy's closure.
- **Placeholder enums in `theme.rs`**, for the later tasks to move next to their owners and keep matching on:
  - `Pill { Normal, Edit, Command }` (T202).
  - `SessionStatus { Connecting, Open { label }, Failed(String), Closed }` (T103's shape). `session_dot(&SessionStatus)` takes a reference because of the payloads.
  - `ToastLevel { Info, Success, Warning, Error }`. `Success` is already there for T205: it is fg on bg, like `Info`.
  - `JobKind { Query, FetchMore, Export, Schema }` and `JobState { Queued, Running { rows }, Done { rows }, Failed, Cancelled }` (T305's shapes).
- **`job_state(kind, &state)`** takes the kind too, as tachy's does: `▸` while running uses the kind's colour (T305). `job(kind)`: Query, FetchMore and Schema are teal (reading data), and Export is amber (as in tachy).
- **Helpers beyond the T201 list**, each with a mono fallback and included in `every_helper_has_mono_fallback` and the swatch:
  - `focused_border` (from tachy): amber, for the boxed panes until T202 removes the boxes.
  - `value_ok` and `value_error` (from tachy; `job_state` and `session_dot` use them).
  - `status_ok`: green on `status_bg`, for success messages in today's status panel.
  - `editor_cursor`: bg on amber / REVERSED, for the text cursor.
  - `dialog_heading(accent)`: bold accent on `surface_raised`, for help group titles.
  - `field_border` (`border_inner`) and `field_border_focused` (amber), for today's boxed form fields.
- **Monochrome choices not given by tachy:** `tab_running` is ITALIC. `border_strong` and `divider_focused` are BOLD. `pane_title` is DIM and `pane_title_focused` BOLD. `tree_connection_selected` is REVERSED|BOLD. `null_value`, `sql_comment` and `field_placeholder` are DIM|ITALIC. `sql_keyword` is BOLD. `editor_selection` is REVERSED. `line_number_current` is BOLD.
- **How the existing screens map onto the helpers.** The layout is unchanged.
  - The pane boxes stay: `border()` normally and `focused_border()` with focus. Titles use `pane_title` / `pane_title_focused`.
  - `Home::draw` first fills the screen with `base()`, so text has the app's fg and bg.
  - Results: the header uses `header`, with `header_active` on the cursor's column. Rows use `row(i)` stripes, and the selected row `row_selected`. The cursor cell is `cursor_cell` with focus and `row_selected` without. `NULL` is `null_value`. The scrollbars use `border` / `border_strong`.
  - Tree: connections use `tree_connection`, and the selected one `tree_connection_selected`. Tables use `tree_object`, and a selected table `row_selected`. Notes and "No connections yet" use `tree_kind` / `dim`. The connection the worker is on still gets `.bold()`, a modifier and not a colour; T301 replaces it with `session_dot`.
  - Forms and help: the body is `dialog`, and the border and title use `dialog_border(teal)` / `dialog_title(teal)`. Labels use `field` / `field_focused`, and the field boxes `field_border(_focused)`. The footer uses `hint`, `spinner` (pending) or `value_error`. Help keys are `hint_key`. T203 and T204 restyle these.
  - Status panel: `status_warning` or `status_ok`, on `status_bg`.
- **Snapshots.** Only `results_grid_80x24` changed (re-accepted after review: amber focused border, surface header with an amber active column, striped rows, a `selection_bg` row and an amber bold cursor cell). The other `home_tests` snapshots are text-only and did not change. The colour assertions in `home_tests::tree_is_focused_and_highlighted_at_startup` and in `conntree_tests` (renamed `the_selection_takes_the_selected_style_and_the_rest_the_plain_one`) now compare against theme helpers. `testing_tests::styled_lists_one_run_per_style_change` uses theme styles instead of `Color::Red` / `Color::Blue`.
- **`no_raw_colors`** exempts only `src/theme.rs` and `src/theme_tests.rs` (tachy also exempted `config.rs`, whose parser is now gone). Test files under `src/` are scanned, so tests must also take colours from the theme (`Theme::DARK.coral`, `style.fg.unwrap_or_default()` for `Reset`). Besides `Color::…`, it flags `.<colour>()` and `.on_<colour>()` for ratatui's 16 `Stylize` colour names. It does not flag modifiers such as `.bold()`. The fixture is the shared `tests/fixtures/raw_color.rs.txt` at the repository root, read through `qry::testing::fixture`. A planted `.green()` in `statuspanel.rs` made the test fail with `statuspanel.rs:57: .green()`, and it was then reverted.
- **`src/views.rs`** exists now, holding only `#[cfg(test)] mod swatch;`. T101 fills it. The swatch (`views/swatch.rs`) draws every helper on a sample of its name, in 4 columns of 20 at 80x24 (86 samples; `every_sample_fits_at_80x24` guards that). Helpers that take an argument share one line, such as `NORMAL EDIT COMMAND`, `session ● ◌ ✗` or `jobs ▸ ✓ ✗ – …`. A new helper should be added to the swatch and to `theme_tests::helpers()`.
- **Test count:** 234 in all: 192 `qry` unit tests, 2 in `tests/harness.rs`, 3 in `tests/no_raw_colors.rs`, 6 in `tests/workspace_metadata.rs` and 31 in `qry-core`. The 7 `parse_style`/`parse_color` tests were removed.
- **Not run here:** nightly rustfmt (there is no rustup on this machine). Stable `cargo fmt` cannot apply `imports_granularity` / `group_imports`, so the new `use` blocks were grouped by hand. CI on GitHub was not run.
