//! The test harness is usable from outside the crate: the self
//! dev-dependency turns on `test-util`, so `qry::testing` and
//! `qry_core::testing` are both here.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use qry::{
    action::Action,
    app::Mode,
    config::Config,
    testing::{AppHarness, Reply, result},
};
use qry_core::testing::FakeDatabase;

#[test]
fn the_harness_drives_the_app_from_an_integration_test() {
    let mut h = AppHarness::new(Config::embedded());
    h.keys("ctrl-n");
    assert_eq!(h.mode(), Mode::AddConnModal);
    assert!(h.render(80, 24).contains("New Connection"));
}

#[test]
fn a_fake_database_answers_through_the_harness() {
    let db = FakeDatabase::default().reply(Reply::Rows(result(&["name"], &[&[Some("alice")]])));
    let mut h = AppHarness::new(Config::embedded()).with_database(db.clone());
    h.send(Action::Execute("SELECT name FROM users".into()));
    assert!(h.render(80, 24).contains("alice"));
    assert_eq!(db.calls(), ["SELECT name FROM users"]);
}
