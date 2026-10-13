# T302 — SQL editor: highlighting and statement execution

**Phase:** 3 — Features · **Depends on:** T201, T202 · **Blocks:** T306, T301 (DDL detection), T303 (error position)

## Description

### Current state

`crates/qry/src/components/editor.rs` wraps `ratatui_textarea::TextArea`:

- It draws `Block::bordered().title("Query")` with a cyan border when focused and dark grey otherwise. The cursor is reversed and line numbers are dark grey.
- Keys map through `EditorCommand` (`keymap.rs:146`), covering movement, selection, word ops, undo/redo, cut/copy/paste.
- `F8` (`RunQuery`) sends `Action::Execute(self.query())`, where `query()` is the **whole buffer**.
- Each backend then runs the whole text:
  - Postgres uses `simple_query`, which runs every statement but **merges all row messages into one `QueryResult`**. Two `SELECT`s with different columns produce a corrupt grid.
  - SQLite and MySQL behave differently for multiple statements.
  - Oracle refuses multiple statements.
- There is no highlighting, no notion of the current statement, and a single buffer shared by everything.

### Decision: keep `ratatui-textarea`

Keep `ratatui-textarea` for editing: cursor, selection, undo/redo, yank, and the key handling already tested in `editor.rs`. Render it ourselves.

- `TextArea` exposes `lines()`, `cursor()` and `selection_range()`.
- A custom widget `SqlView` draws the visible lines from those, using tokens from the qry-core lexer, instead of calling `frame.render_widget(&textarea)`.
- This avoids writing a text buffer, and gives full control over styles, the gutter and error underlines.
- If `TextArea`'s scroll offset is not exposed in 0.9, we track the viewport ourselves: we keep the cursor in view the same way `scroll_rows` in `results.rs` does.
- Revisit only if multi-cursor or very large files (> 1 MB) become goals. Both are out of scope.

### Target design

```
 QUERY · local-app                          ln 4, col 12 · 3 statements      ← title row (surface), amber when focused
  1 │ -- active users
  2 │ SELECT id, name
  3 │   FROM users
▌ 4 │  WHERE created_at > now() - interval '7 days';   ← amber ▌ marks the statement under the cursor
  5 │
  6 │ SELECT count(*) FROM orders WHERE total > 100.5;
  7 │ /* nested /* ok */ comment */
```

**Token colours**, from T201 query helpers reused for SQL:

| Token | Style |
|---|---|
| keyword | `query_keyword` (teal) |
| string / quoted literal | `query_string` (green) |
| number | `query_number` (purple) |
| comment | `dim` + italic |
| quoted identifier | `fg` |
| identifier | `fg` |
| operator / punctuation | `query_operator` (teal) |
| unterminated string or comment, or a server error position | `query_error` (coral underline) |

**Gutter:**

- Line numbers in `fg_dim`, with `│` in `border`.
- The current statement's lines get an amber `▌` in column 0.
- Lines of a statement that failed in its last run get a coral `▌`.

**Bracket matching:** when the cursor is next to `(` or `)`, both brackets are drawn amber bold. Unmatched ones are coral.

**Auto-indent:** `Enter` copies the leading whitespace of the current line.

**Tab handling:**

- `Tab` in the editor indents (4 spaces), unless completion is open.
- `shift-Tab` dedents.
- Focus cycling moves to `ctrl-Tab`/`F6` in the Editor context (T102 defines the binding).

**Execution commands** (Editor context):

| Key | Command | Behaviour |
|---|---|---|
| `ctrl-enter`, `F5` | Run statement | runs the statement under the cursor |
| `ctrl-shift-enter`, `F9` | Run selection | runs the selected text, split into statements |
| `F8` | Run all | runs every statement in the buffer, in order |
| `ctrl-l` | Format? | out of scope, reserved |

Each statement becomes its own T305 job and its own result set. Results appear as tabs in the results pane (T303). Run-all stops at the first failure unless config `continue_on_error = true`.

**Per-session buffers.** Each session (T103) owns its editor buffer, undo history and cursor. Switching sessions swaps buffers. The scratch buffer contents are kept in memory only. Persisting buffers is out of scope; T306 history covers recall.

**Completion** (optional, can be split into T302b):

- `Tab` after an identifier prefix opens a small popup.
- It lists names from the session's `SchemaCache` (T301) plus keywords for the dialect.
- `Enter`/`Tab` accepts and `Esc` dismisses. Selection is drawn purple-tinted like the palette.

### qry-core split

New module `crates/qry-core/src/sql/` (no terminal deps):

```rust
pub enum Dialect { Postgres, MySql, Sqlite, Oracle }

pub enum TokenKind {
    Keyword, Ident, QuotedIdent, String, Number, Comment,
    Operator, Punct, Whitespace, Unterminated, Delimiter,
}
pub struct Token { pub kind: TokenKind, pub span: std::ops::Range<usize> } // byte offsets

pub fn lex(dialect: Dialect, src: &str) -> Vec<Token>;  // total: never fails, never panics

pub struct Statement { pub span: std::ops::Range<usize>, pub text: String }
pub fn split(dialect: Dialect, src: &str) -> Vec<Statement>;
pub fn statement_at(stmts: &[Statement], offset: usize) -> Option<usize>;
pub fn is_ddl(dialect: Dialect, stmt: &str) -> bool;
```

**Lexer rules:**

- `--` line comments; `/* */` block comments, which nest for Postgres only. MySQL `#` comments.
- Strings:
  - `'...'` with `''` escaping.
  - Postgres `E'..'` with backslash escapes.
  - Postgres dollar-quoting `$tag$ ... $tag$`.
  - Oracle `q'[...]'`, `q'{...}'`, `q'<...>'`, `q'(...)'` and `q'!...!'`.
  - MySQL `"..."` is a string unless `ANSI_QUOTES`; treat it as a string for MySQL/MariaDB.
- Quoted identifiers: `"..."` (Postgres, SQLite, Oracle), `` `...` `` (MySQL, MariaDB, SQLite), `[...]` (SQLite).
- Numbers: integers, decimals, exponents and `0x` hex.
- Keywords: a per-dialect static sorted list, matched case-insensitively.

**Splitter rules:**

- `;` outside strings and comments ends a statement.
- Oracle: a line containing only `/` ends a PL/SQL block. Inside `BEGIN … END;`, `DECLARE` or `CREATE [OR REPLACE] PROCEDURE|FUNCTION|PACKAGE|TRIGGER`, a `;` does **not** split.
- MySQL: `DELIMITER $$` lines change the terminator and are not sent to the server.
- Empty or whitespace/comment-only statements are dropped. Spans point into the original text, so positions map back.

**Backend contract:**

- `Database::query` keeps taking one statement. The worker no longer sends multi-statement text.
- Postgres switches from `simple_query` to the extended protocol for single statements where possible. This gives column types to T303. It falls back to `simple_query` for statements the extended protocol rejects, such as `COPY` or multiple commands.

**Error positions.** `QueryError { message, position: Option<usize> }` carries the position where the backend reports one: Postgres `position`, SQLite offset, Oracle `ORA-` error column. The editor maps it to the statement span and underlines from that position to the end of the token.

### Edge cases

- Unterminated string or comment at EOF: lexed as `Unterminated` to the end and highlighted coral. The splitter treats the rest as one statement.
- Running with the cursor on a blank line between statements runs the next statement below. At EOF it runs the previous one.
- A selection that cuts a statement in half runs exactly the selected text.
- CRLF pasted text is normalised to LF on paste.
- Tabs are rendered as 4 spaces.
- Wide unicode (CJK, emoji) in strings must not misalign the gutter. Use `unicode-width`.
- Lexing a 1 MB buffer must not freeze input. Highlighting is incremental per line, or the visible range is lexed from the nearest statement start (cached statement starts).

### Out of scope

SQL formatting, multi-cursor, vim mode, LSP, files on disk (open/save `.sql`; a follow-up task).

## Acceptance criteria

- [ ] `qry_core::sql::{lex, split, statement_at, is_ddl}` exist, are documented, and have no terminal dependencies.
- [ ] The lexer is total: it never panics and its spans tile the whole input with no gaps or overlaps (property-tested and fuzzed).
- [ ] The splitter handles semicolons inside strings, comments, dollar-quoted bodies, Oracle q-quotes and PL/SQL blocks; the Oracle `/` terminator; and MySQL `DELIMITER`.
- [ ] Highlighting uses only `Theme` helpers: keyword teal, string green, number purple, comment dim italic, error coral underline.
- [ ] An amber `▌` marks the statement under the cursor and moves as the cursor moves.
- [ ] Run statement, run selection and run all each produce one job and one result set per statement. Two `SELECT`s on Postgres yield two correct result tabs, not a merged grid.
- [ ] A server error with a position underlines the right token in coral, and the gutter shows a coral `▌` on the failed statement.
- [ ] Each session has its own buffer, cursor and undo history. Switching sessions keeps them.
- [ ] Bracket matching and auto-indent work as described.
- [ ] Every existing `editor.rs` test still passes, or is ported to a sibling `editor_tests.rs` with the same assertions.
- [ ] Typing in a 10 000-line buffer stays under the T006 budget (bench `editor_render_10k_lines`).

## Tests

**Unit tests** — `crates/qry-core/src/sql/lex_tests.rs`:

- Dialect quoting: `keywords_case_insensitive`, `identifier_vs_keyword_boundary` (`selected` is an identifier), `quoted_identifiers_per_dialect`.
- Strings: `single_quote_doubling`, `postgres_e_string_backslash`, `dollar_quoted_with_tag_and_nested_dollar`, `oracle_q_quote_all_delimiters`, `mysql_double_quote_is_string`.
- Comments: `line_and_block_comments`, `postgres_nested_block_comment`, `mysql_hash_comment`.
- Literals and errors: `numbers_hex_exponent_decimal`, `unterminated_string_runs_to_eof`.
- Coverage: `spans_tile_input_exactly`.

**Unit tests** — `crates/qry-core/src/sql/split_tests.rs`:

- Basic: `splits_on_semicolons`, `ignores_semicolon_in_string_and_comment`, `drops_empty_and_comment_only`, `trailing_statement_without_semicolon`, `spans_map_to_original_text`.
- Dialect blocks: `dollar_quoted_function_body_is_one_statement`, `oracle_plsql_block_until_slash`, `oracle_create_procedure_not_split_inside`, `mysql_delimiter_changes_terminator_and_is_stripped`.
- Cursor lookup: `statement_at_cursor_inside`, `statement_at_blank_line_picks_next`, `statement_at_eof_picks_previous`.
- DDL: `is_ddl_true_for_create_alter_drop_truncate`, `is_ddl_false_for_select_with_drop_in_string`.

**Proptests:**

- `prop_lex_never_panics_and_tiles(src in ".{0,2000}")`
- `prop_split_concat_covers_all_non_trivia(src)`: the split statements, rejoined with their terminators, cover every non-whitespace, non-comment byte.
- `prop_split_is_stable_under_trailing_whitespace`

**Fuzz targets**, with proptest twins per T005:

- `fuzz/fuzz_targets/sql_lex.rs`: all four dialects on the same input.
- `fuzz/fuzz_targets/sql_split.rs`

Seeds go in `fuzz/seed-corpus.sh` from `crates/qry-core/tests/fixtures/sql/*.sql`, which includes PL/SQL, dollar-quoted and DELIMITER samples.

**Unit tests** — `crates/qry/src/components/editor_tests.rs`:

- The ported existing tests.
- `ctrl_enter_runs_statement_under_cursor_only`
- `f8_runs_all_statements_as_separate_jobs`
- `run_selection_runs_selected_text`
- `run_all_stops_at_first_failure_by_default`
- `continue_on_error_runs_the_rest`
- `buffers_are_per_session`
- `undo_history_is_per_session`
- `enter_auto_indents`
- `tab_indents_shift_tab_dedents`
- `crlf_paste_normalised`
- `error_position_maps_into_statement`

**Snapshots** — plain and styled:

- `editor_highlight_postgres_80x12`
- `editor_highlight_oracle_plsql_80x16`
- `editor_current_statement_marker_80x12`
- `editor_error_underline_80x8`
- `editor_bracket_match_60x6`
- `editor_unfocused_80x8`
- `editor_wide_unicode_60x6`

**AppHarness flow:** type `select 1; select 2;`, then `h.keys("f8")`. The results tabs show ` 1 · 1 row ` and ` 2 · 1 row `. Snapshot `run_all_two_tabs_120x40`.

**DB integration tests** (T004 `db_or_skip!`): `two_selects_produce_two_results` for every backend, and `error_position_reported` for Postgres, SQLite and Oracle.
