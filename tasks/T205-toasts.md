# T205 — Toasts and notifications

**Phase:** 2 — Visual system · **Depends on:** T202 · **Blocks:** T305, T307, T308

## Description

### Current state

User feedback goes through `Action::Status(StatusCode::{None, Success(String), Error(String)})` into `Statuspanel` (`crates/qry/src/components/statuspanel.rs`). That panel is a 3-row bordered box which only appears when it holds a message, so the whole layout shifts down and up again.

- There is only one slot: a new message silently replaces the previous one.
- A message never expires.
- `Action::Error(String)` is only written to the log (`app.rs:179`, `tracing::error!`). Failures such as "cannot save connections", "cannot remove the stored password" and draw errors are invisible to the user unless they read `qry.log`.
- Some failure paths (`App::import`, `App::export_connections`) send both a status message and a modal answer, to work around the modal footer being one line.

### Target design

A tachy-style toast line plus a message history.

**Levels and styles** (via `theme.toast(level)`):

| Level | Style | Prefix |
|---|---|---|
| `Info` | fg on bg | — |
| `Success` | green `✓` then fg | `✓ ` |
| `Warning` | coral | `! ` |
| `Error` | coral bold | `✗ ` |

**Placement**
- The toast overlays the **last row of the body**: the rect `ScreenLayout::toast` from T202, which is the bottom row of the results region, or of the jobs drawer when that is open.
- The layout never moves.
- It is drawn after any overlay, so it stays visible above dialogs and the backdrop (T203 z-order).

**Format**
- ` text… (+N) `: one space of padding on each side.
- The text is truncated with `…` to fit.
- `(+N)` is in dim at the right when N more toasts are queued.

**Queue and timing**
- Toasts form a FIFO queue, and one is visible at a time.
- Each has a lifetime that starts when it becomes visible: Info and Success 4 s, Warning 6 s, Error 8 s.
- Lifetimes use the reducer's virtual clock (T101 `Effect::ScheduleTimer`), so they are testable with `AppHarness::advance`.
- `Esc` with no overlay open dismisses the visible toast early. When something else also consumes `Esc` in that context, the other handler wins.
- Identical consecutive messages are coalesced: the visible toast's timer restarts and a `×3` counter is appended instead of queueing duplicates.
- The queue is capped at 20 entries. The oldest non-error entries are dropped first.

**Sources**
- Every `Action::Status(StatusCode::Success | Error)` becomes a toast, and `StatusCode` is replaced by `Notice { level, text }`.
- `Action::Error(String)` becomes an Error toast as well as a log line.
- `Action::Info` becomes an Info toast.
- Worker results that used to set the status panel ("Connected to …", "Exported 3 connections to …", query errors) now emit `Effect::Notify(Notice)`.
- Query errors are also shown inline in the results title (T303 and T305). The toast is the transient copy.

**Message history**
- Every notice (not just the visible one) is appended to a bounded ring buffer of 500 entries, holding the timestamp (wall-clock time of day, stored as data rather than read in the reducer), level and full untruncated text.
- `ctrl-l` (action `ShowMessages`, also listed in the palette) opens a **Messages** dialog. It is a T203 teal dialog, 90% of the width by 70% of the height.
  - It lists the entries newest first as `12:04:31  ✗ cannot save connections: permission denied (os error 13)`.
  - Text is word-wrapped, and the level glyph is styled as above.
  - `j/k/PgUp/PgDn/Home/G` scroll, `y` copies the selected message, `c` clears the history, and `Esc` closes.
- When an error toast is truncated, the toast ends with ` ctrl-l` in `hint_key` so the user knows where to read the full text.

### Mockup at 120x40 (error toast with 2 queued, over the results grid)

```
                               │   7 │ 31  │ user031  Porto
                               │   8 │ 44  │ user044  Lisbon
                               │ ✗ cannot save connections: permission denied (os error 13)       ctrl-l  (+2)
 prod · postgres · app · auto-commit                                                                R 1 / 204  C 1/3 
 ctrl-enter run  F8 run all  tab next pane  ctrl-p commands                                                     ? help
```

### Mockup at 80x24 (Messages dialog)

```
     ┌ Messages ─────────────────────────────────────────────────────────┐
     │ 12:04:31  ✗ cannot save connections: permission denied (os error   │
     │             13)                                                    │
     │ 12:03:58  ✓ Connected to app@db.local:5432/app                     │
     │ 12:03:40  ✓ Exported 3 connections to /home/u/qry-backup.json      │
     │                                                                    │
     └─ j/k scroll  y copy  c clear  Esc close ───────────────────────────┘
```

### tachy reference

- `~/Projects/tachy/crates/tachy/src/components/toast.rs`: the toast line, ` text… (+N) ` formatting and overlaying the last body row.
- `~/Projects/tachy/crates/tachy/src/theme.rs`: `toast(level)` (coral for Error/Warning, fg for Info, on bg). qry adds a green Success variant using `value_ok`.
- `~/Projects/tachy/crates/tachy/src/app.rs` `draw_frame`: the toast is drawn last.

### Proposed modules

```
crates/qry/src/notice.rs            Notice, Level, NoticeLog (ring buffer)     + notice_tests.rs
crates/qry/src/ui/toast.rs          draw_toast                                  + toast_tests.rs
crates/qry/src/ui/overlay/messages.rs  Messages dialog                          + messages_tests.rs
```

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Level { Info, Success, Warning, Error }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice { pub level: Level, pub text: String }

pub struct Toasts {
    queue: VecDeque<(Notice, u32 /* repeat count */)>,
    visible_since: Option<u64>, // virtual ms
}

impl Toasts {
    pub fn push(&mut self, notice: Notice, now_ms: u64) -> Option<Effect>; // ScheduleTimer for the first one
    pub fn on_timer(&mut self, now_ms: u64) -> Option<Effect>;             // advance the queue
    pub fn dismiss(&mut self, now_ms: u64) -> Option<Effect>;
    pub fn visible(&self) -> Option<(&Notice, u32, usize /* queued */)>;
}

pub fn lifetime(level: Level) -> Duration;
```

### Out of scope

- Desktop notifications (OSC 9 / notify-send). These can come later as an option.
- Persisting the message history across runs. The log file already covers that.

## Acceptance criteria

- [ ] `Statuspanel` and `StatusCode` are removed, and every former `Action::Status` site produces a `Notice`.
- [ ] `Action::Error` produces an Error toast as well as a log line. A failed `connections::save` is visible to the user.
- [ ] Toasts never change the layout: the `ScreenLayout` rects are identical with and without a visible toast.
- [ ] The toast is drawn above dialogs and the backdrop.
- [ ] Level styles and prefixes match the table, and only `Theme` helpers are used.
- [ ] Lifetimes are Info/Success 4 s, Warning 6 s and Error 8 s, measured from when the toast becomes visible. The next queued toast appears as soon as the current one expires.
- [ ] `(+N)` shows the exact number of queued toasts. Identical consecutive notices coalesce into one entry with `×N` instead of queueing.
- [ ] The queue is capped at 20, and errors are kept in preference to info entries.
- [ ] `Esc` dismisses the visible toast only when no overlay is open and the focused pane doesn't consume `Esc`.
- [ ] Truncated error toasts show the ` ctrl-l` hint.
- [ ] `ctrl-l` opens the Messages dialog with every notice newest-first and untruncated (wrapped), up to 500 entries.
- [ ] In the Messages dialog, `y` copies the selected message, `c` clears the history and `Esc` closes.
- [ ] The reducer never reads the clock. Timestamps arrive with the event, and the T002 no-IO scan still passes.

## Tests

`crates/qry/src/notice_tests.rs`:
- `ring_buffer_keeps_last_500`.
- `queue_cap_drops_oldest_non_error_first`.
- `identical_consecutive_notices_coalesce`.

`crates/qry/src/ui/toast_tests.rs`:
- `formats_with_padding_ellipsis_and_queue_count`: widths 120 and 40.
- `truncated_error_shows_messages_hint`.
- `level_styles_and_prefixes`: styled buffer check per level.
- insta snapshots `toast_info_120x1`, `toast_success_120x1`, `toast_error_queued_120x1` and `toast_error_truncated_80x1`, plain and styled.

Reducer and harness tests, in `crates/qry/src/app/toast_tests.rs`:
- `info_expires_after_4s_error_after_8s`: `advance(3999)` still shows the toast, and `advance(1)` clears it.
- `next_toast_appears_when_current_expires`.
- `esc_dismisses_toast_when_nothing_else_uses_it`.
- `esc_in_dialog_closes_dialog_not_toast`.
- `action_error_becomes_a_toast`.
- `failed_save_connections_is_visible`: feed the worker effect failure, then assert the toast text.
- `toast_does_not_move_layout`: compare `compute_layout` with and without a toast.
- `toast_draws_above_dialog`: screen snapshot `screen_toast_over_dialog_120x40`.

`crates/qry/src/ui/overlay/messages_tests.rs`:
- `newest_first_and_wrapped`.
- `y_copies_selected_message`: assert `Effect::CopyToClipboard`.
- `c_clears_history`.
- insta snapshot `messages_80x24`, plain and styled.
