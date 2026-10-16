//! Unit tests for sessions in the reducer: opening, switching, closing, and
//! answers routed by session. [`FakeSessions`] stands in for the runtime's
//! `SessionManager`: it records the effects and lets a test deliver any
//! session's answers.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use super::{state::Focus, *};
use crate::{
    connections::{Driver, Secret, StoredConnection},
    db::DbCommand,
    testing::{AppHarness, buffer_lines, grid, render_with},
    views,
};

/// A saved SQLite connection whose id is its name.
fn sqlite(name: &str) -> StoredConnection {
    let mut record = StoredConnection::with_id(name, name, Driver::Sqlite);
    record.path = Some(format!("{name}.db"));
    record.secret = Secret::None;
    record
}

/// The app after startup with `prod` and `analytics` saved, the tree
/// focused on the scratch row.
struct FakeSessions {
    h: AppHarness,
}

impl FakeSessions {
    fn new() -> Self {
        let mut h = AppHarness::new(Config::embedded());
        h.send(UiEvent::Started);
        h.file(FileAnswer::ConnectionsLoaded(vec![
            sqlite("prod"),
            sqlite("analytics"),
        ]));
        h.db_for(SessionId::FIRST, DbAnswer::Connected(":memory:".into()));
        h.take_effects();
        Self { h }
    }

    fn keys(&mut self, keys: &str) -> &mut Self {
        self.h.keys(keys);
        self
    }

    /// Delivers session `id`'s answer.
    fn answer(&mut self, id: u32, answer: DbAnswer) -> &mut Self {
        self.h.db_for(SessionId(id), answer);
        self
    }

    fn effects(&mut self) -> Vec<Effect> {
        self.h.take_effects()
    }

    fn active(&self) -> SessionId {
        self.h.active_session()
    }

    /// Selects row `row` of the tree (0 is scratch, 1 prod, 2 analytics).
    fn select(&mut self, row: usize) -> &mut Self {
        self.h.state_mut().focus = Focus::Tree;
        self.h.state_mut().tree.selected = row;
        self
    }

    /// Opens the connection on tree row `row` with Enter, and has it
    /// connect.
    fn open(&mut self, row: usize) -> SessionId {
        self.select(row).keys("enter");
        let id = self.active();
        let label = format!("{}.db", self.h.state().sessions.active().connection_id);
        self.answer(id.0, DbAnswer::Connected(label));
        self.effects();
        id
    }

    fn editor_text(&self) -> String {
        self.h.state().editor().query()
    }

    /// Everything but the redraw flag, to compare states.
    fn snapshot(&self) -> State {
        let mut state = self.h.state().clone();
        state.dirty = false;
        state
    }
}

#[test]
fn startup_has_scratch_session_one() {
    let mut h = AppHarness::new(Config::embedded());
    assert_eq!(h.sessions(), [SessionId::FIRST]);
    assert_eq!(SessionId::FIRST, SessionId(1));
    h.send(UiEvent::Started);
    let effects = h.take_effects();
    let [Effect::LoadConnections, Effect::OpenSession(id, scratch)] = effects.as_slice() else {
        panic!("expected the scratch session, got {effects:?}");
    };
    assert_eq!(*id, SessionId(1));
    assert!(scratch.ephemeral);
    assert_eq!(h.active_session(), SessionId(1));
    assert!(
        h.render(80, 24)
            .lines()
            .next()
            .unwrap()
            .contains(" 1 scratch ")
    );
}

#[test]
fn enter_on_connection_opens_new_session_and_focuses_it() {
    let mut s = FakeSessions::new();
    s.select(1).keys("enter");
    assert_eq!(
        s.effects(),
        [Effect::OpenSession(SessionId(2), Box::new(sqlite("prod")))]
    );
    assert_eq!(s.active(), SessionId(2));
    assert_eq!(s.h.state().focus, Focus::Tree, "the focus stays");
    assert!(s.h.render(80, 24).contains(" 2 prod "));
}

#[test]
fn enter_on_open_connection_switches_instead() {
    let mut s = FakeSessions::new();
    let prod = s.open(1);
    s.keys("ctrl-1");
    assert_eq!(s.active(), SessionId::FIRST);

    s.select(1).keys("enter");
    assert!(s.effects().is_empty(), "no second session");
    assert_eq!(s.active(), prod);
    assert_eq!(s.h.sessions().len(), 2);
}

#[test]
fn shift_enter_opens_second_session_same_connection() {
    let mut s = FakeSessions::new();
    let first = s.open(1);
    s.select(1).keys("shift-enter");
    let effects = s.effects();
    let [Effect::OpenSession(second, record)] = effects.as_slice() else {
        panic!("expected another session, got {effects:?}");
    };
    assert_ne!(*second, first);
    assert_eq!(record.id, "prod");
    assert_eq!(s.active(), *second);
    assert_eq!(s.h.sessions(), [SessionId(1), first, *second]);
}

#[test]
fn switching_sessions_swaps_editor_and_results() {
    let mut s = FakeSessions::new();
    let prod = s.open(1);
    s.keys("ctrl-1 tab").h.type_str("select 1");
    s.answer(1, DbAnswer::QueryDone(Arc::new(grid(3, 2))));

    s.keys("ctrl-2");
    assert_eq!(s.active(), prod);
    assert_eq!(s.h.state().focus, Focus::Editor, "the focus stays");
    assert_eq!(s.editor_text(), "", "session 2 has its own buffer");
    assert!(s.h.state().results().result.is_none());
    s.h.type_str("select 2");

    s.keys("ctrl-1");
    assert_eq!(s.editor_text(), "select 1");
    assert_eq!(s.h.state().editor().textarea.cursor(), (0, 8), "at its end");
    assert_eq!(s.h.state().results().result.as_ref().unwrap().rows.len(), 3);
    // The undo history came along: undoing takes back session 1's typing
    // only.
    s.keys("ctrl-z");
    assert_ne!(s.editor_text(), "select 1");
    s.keys("ctrl-PgDn");
    assert_eq!(s.editor_text(), "select 2");
}

#[test]
fn queries_run_in_the_active_session_only() {
    let mut s = FakeSessions::new();
    let prod = s.open(1);
    s.keys("tab").h.type_str("select 2");
    s.keys("f8");
    assert_eq!(
        s.effects(),
        [Effect::Db(prod, DbCommand::Query("select 2".into()))]
    );
    assert!(s.h.render(120, 40).contains("◐"), "the tab shows it runs");

    // Its result lands in session 2, whichever is shown.
    s.keys("ctrl-1");
    s.answer(prod.0, DbAnswer::QueryDone(Arc::new(grid(2, 2))));
    assert!(s.h.state().results().result.is_none());
    s.keys("ctrl-2");
    assert_eq!(s.h.state().results().result.as_ref().unwrap().rows.len(), 2);
    assert_eq!(s.h.state().sessions.active().running, 0);
}

#[test]
fn answer_for_closed_session_is_dropped() {
    let mut s = FakeSessions::new();
    let prod = s.open(1);
    s.keys("ctrl-2 ctrl-w");
    assert_eq!(s.effects(), [Effect::CloseSession(prod)]);
    assert_eq!(s.active(), SessionId::FIRST);

    let before = s.snapshot();
    s.answer(prod.0, DbAnswer::QueryDone(Arc::new(grid(1, 1))));
    s.answer(prod.0, DbAnswer::TablesLoaded(vec!["t".into()]));
    s.answer(prod.0, DbAnswer::WorkerStopped);
    assert_eq!(s.snapshot(), before);
    assert!(s.effects().is_empty());
}

#[test]
fn closing_last_session_reopens_scratch() {
    let mut s = FakeSessions::new();
    s.select(0).keys("ctrl-w");
    let effects = s.effects();
    let [
        Effect::CloseSession(SessionId(1)),
        Effect::OpenSession(fresh, scratch),
    ] = effects.as_slice()
    else {
        panic!("expected scratch closed and opened again, got {effects:?}");
    };
    assert_eq!(*fresh, SessionId(2), "ids are never reused");
    assert!(scratch.ephemeral);
    assert_eq!(s.h.sessions(), [*fresh]);
    assert_eq!(s.active(), *fresh);

    // It is still never saved.
    s.answer(fresh.0, DbAnswer::Connected(":memory:".into()));
    assert!(
        !s.effects()
            .iter()
            .any(|e| matches!(e, Effect::SaveConnections(_)))
    );
}

#[test]
fn close_with_running_job_asks_first() {
    let mut s = FakeSessions::new();
    let prod = s.open(1);
    s.keys("tab").h.type_str("select pg_sleep(60)");
    s.keys("f8 f6");
    s.effects();

    s.keys("ctrl-w");
    assert!(s.effects().is_empty(), "nothing closes before confirming");
    assert_eq!(s.h.dialog_title(), Some("Close Session"));
    assert!(s.h.render(80, 24).contains("prod is running a query"));

    // Esc keeps it.
    s.keys("esc");
    assert!(s.effects().is_empty());
    assert_eq!(s.h.sessions().len(), 2);

    // Enter starts on Cancel, as closing cannot be undone.
    s.keys("ctrl-w enter");
    assert!(s.effects().is_empty());
    assert_eq!(s.h.sessions().len(), 2);
    s.keys("ctrl-w right enter");
    assert_eq!(s.effects(), [Effect::CloseSession(prod)]);
    assert!(s.h.state().dialog().is_none());
    assert_eq!(s.active(), SessionId::FIRST);
    let (_, notice) = s.h.state().messages.newest_first().next().unwrap();
    assert!(notice.text.contains("cancelled"), "{notice:?}");
}

#[test]
fn worker_stopped_marks_session_failed() {
    let mut s = FakeSessions::new();
    s.answer(1, DbAnswer::WorkerStopped);
    assert_eq!(
        s.h.state().sessions.active().status,
        SessionStatus::Failed("database worker has stopped".into())
    );
    let text = s.h.render(80, 24);
    assert!(
        text.contains("scratch: database worker has stopped"),
        "{text}"
    );
    assert!(text.contains("✗ scratch"), "the tree marks it: {text}");
}

#[test]
fn ctrl_digit_out_of_range_is_ignored() {
    let mut s = FakeSessions::new();
    s.open(1);
    s.open(2);
    assert_eq!(s.h.sessions().len(), 3);
    let before = s.snapshot();
    s.keys("ctrl-7");
    assert_eq!(s.snapshot(), before);
    assert!(s.effects().is_empty());

    s.keys("ctrl-2");
    assert_eq!(s.active(), SessionId(2));
}

#[test]
fn next_and_previous_session_wrap_around() {
    let mut s = FakeSessions::new();
    s.open(1);
    s.open(2);
    s.keys("ctrl-PgDn");
    assert_eq!(s.active(), SessionId(1));
    s.keys("ctrl-PgUp");
    assert_eq!(s.active(), SessionId(3));
}

#[test]
fn ctrl_w_in_the_editor_deletes_a_word_instead() {
    let mut s = FakeSessions::new();
    s.keys("tab").h.type_str("select one");
    s.keys("ctrl-w");
    assert!(s.effects().is_empty());
    assert_eq!(s.editor_text(), "select ");
}

#[test]
fn two_password_prompts_each_reach_their_own_session() {
    const CANARY_A: &str = "CANARY-PW-a";
    const CANARY_B: &str = "CANARY-PW-b";
    let mut s = FakeSessions::new();
    s.select(1).keys("enter");
    s.select(2).keys("enter");
    s.effects();
    let (a, b) = (SessionId(2), SessionId(3));

    // Both workers ask at once; the second waits its turn.
    s.answer(
        a.0,
        DbAnswer::NeedPassword {
            id: "prod".into(),
            name: "prod".into(),
        },
    );
    s.answer(
        b.0,
        DbAnswer::NeedPassword {
            id: "analytics".into(),
            name: "analytics".into(),
        },
    );
    assert!(s.h.render(80, 24).contains("for prod"));

    s.h.type_str(CANARY_A);
    s.keys("enter");
    let effects = s.effects();
    let [
        Effect::StashPassword {
            connection,
            password,
        },
        Effect::Db(to, DbCommand::PasswordEntered),
    ] = effects.as_slice()
    else {
        panic!("expected the stash and session 2's worker, got {effects:?}");
    };
    assert_eq!(
        (connection.as_str(), password.as_str(), *to),
        ("prod", CANARY_A, a)
    );

    // Session 2 connects, and analytics' prompt comes up.
    s.answer(a.0, DbAnswer::Connected("prod".into()));
    assert!(s.h.render(80, 24).contains("for analytics"));
    s.h.type_str(CANARY_B);
    s.keys("enter");
    let effects = s.effects();
    let [
        Effect::StashPassword {
            connection,
            password,
        },
        Effect::Db(to, DbCommand::PasswordEntered),
    ] = effects.as_slice()
    else {
        panic!("expected the stash and session 3's worker, got {effects:?}");
    };
    assert_eq!(
        (connection.as_str(), password.as_str(), *to),
        ("analytics", CANARY_B, b)
    );
}

#[test]
fn closing_a_session_drops_its_waiting_password_prompt() {
    let mut s = FakeSessions::new();
    s.select(1).keys("enter");
    s.answer(
        2,
        DbAnswer::NeedPassword {
            id: "prod".into(),
            name: "prod".into(),
        },
    );
    assert_eq!(s.h.dialog_title(), Some("Password"));
    s.h.state_mut().close_dialog();
    s.keys("ctrl-w");
    assert!(s.h.state().dialog().is_none());
    assert!(s.h.state().password_queue.is_empty());
}

#[test]
fn a_failed_saved_connection_reconnects_on_enter() {
    let mut s = FakeSessions::new();
    s.select(1).keys("enter");
    s.answer(2, DbAnswer::ConnectFailed("refused".into()));
    s.effects();
    assert!(s.h.render(80, 24).contains("✗ prod"));
    s.keys("ctrl-1").select(1).keys("enter");
    assert_eq!(
        s.effects(),
        [Effect::Db(
            SessionId(2),
            DbCommand::Connect(Box::new(sqlite("prod")))
        )]
    );
    assert_eq!(s.active(), SessionId(2));
    assert!(s.h.render(80, 24).contains("◌ prod"));
}

#[test]
fn the_status_line_shows_the_active_session() {
    let mut s = FakeSessions::new();
    s.select(1).keys("enter");
    s.answer(2, DbAnswer::Connected("alice@db.local:5432/app".into()));
    // The status line is the one above the key hints.
    let status = |s: &mut FakeSessions| {
        let text = s.h.render(80, 24);
        text.lines().nth_back(1).unwrap().to_string()
    };
    let line = status(&mut s);
    assert!(line.contains("prod · sqlite · prod.db"), "{line}");
    s.keys("ctrl-1");
    let line = status(&mut s);
    assert!(line.contains("scratch · sqlite · :memory:"), "{line}");
}

/// Three sessions, the second running a query and the third active.
fn three_sessions() -> FakeSessions {
    let mut s = FakeSessions::new();
    let prod = s.open(1);
    s.open(2);
    s.keys("ctrl-2 tab").h.type_str("select 1");
    s.keys("f8 ctrl-3 shift-tab");
    assert_eq!(s.h.state().sessions.get(prod).unwrap().running, 1);
    s
}

#[test]
fn top_bar_three_sessions_snapshot() {
    let mut s = three_sessions();
    insta::assert_snapshot!("top_bar_three_sessions_120x40", s.h.render(120, 40));

    // The active tab takes the active style, the running one its own.
    let theme = *s.h.theme();
    let bar = render_with(120, 1, |frame| {
        let state = s.h.state();
        views::top_bar::draw(state, &theme, frame, frame.area());
    });
    let line = &buffer_lines(&bar)[0];
    let drawn = |label: &str| {
        let cell = &bar[(u16::try_from(line.find(label).unwrap()).unwrap(), 0)];
        (Some(cell.fg), Some(cell.bg), cell.modifier)
    };
    let wanted = |style: ratatui::style::Style| (style.fg, style.bg, style.add_modifier);
    assert_eq!(drawn(" 3 analytics"), wanted(theme.tab_active()));
    assert_eq!(drawn(" 2 prod"), wanted(theme.tab_running()));
    assert_eq!(drawn(" 1 scratch"), wanted(theme.tab()));
}

#[test]
fn top_bar_overflow_snapshot() {
    let mut s = FakeSessions::new();
    for name in ["orders", "inventory", "reporting", "warehouse", "billing"] {
        let mut record = sqlite(name);
        record.name = format!("{name}-replica-eu-west");
        s.h.file(FileAnswer::ConnectionsLoaded(vec![record]));
    }
    for row in 3..8 {
        s.open(row);
    }
    // The last is active, so the first tabs give way to `…`.
    let bar = render_with(80, 1, |frame| {
        let state = s.h.state();
        views::top_bar::draw(state, &state.theme, frame, frame.area());
    });
    let line = buffer_lines(&bar)[0].clone();
    assert!(line.starts_with(" qry  … "), "{line}");
    assert!(line.contains("billing"), "the active tab is shown: {line}");
    insta::assert_snapshot!("top_bar_overflow_80x24", s.h.render(80, 24));

    // With the first active, the overflow is at the end instead.
    s.keys("ctrl-1");
    let bar = render_with(80, 1, |frame| {
        let state = s.h.state();
        views::top_bar::draw(state, &state.theme, frame, frame.area());
    });
    let line = buffer_lines(&bar)[0].clone();
    assert!(line.starts_with(" qry  1 scratch"), "{line}");
    assert!(line.contains(" … "), "{line}");
}

#[test]
fn two_sessions_run_jobs_at_once() {
    let mut s = FakeSessions::new();
    let prod = s.open(1);
    s.keys("tab").h.type_str("select 2");
    s.keys("f8");
    s.keys("ctrl-1");
    s.keys("tab").h.type_str("select 1");
    s.keys("f8");
    let jobs = &s.h.state().jobs;
    assert_eq!(jobs.running_count(), 2);
    assert!(jobs.running(prod).is_some() && jobs.running(SessionId::FIRST).is_some());
}
