//! Unit tests for `notice.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn ring_buffer_keeps_last_500() {
    let mut log = NoticeLog::default();
    for i in 0..600 {
        log.push(TimeOfDay(i), Notice::info(format!("n{i}")));
    }
    assert_eq!(log.len(), LOG_CAPACITY);
    let newest: Vec<_> = log.newest_first().map(|(_, n)| n.text.clone()).collect();
    assert_eq!(newest.first().unwrap(), "n599");
    assert_eq!(newest.last().unwrap(), "n100");
    log.clear();
    assert!(log.is_empty());
}

#[test]
fn queue_cap_drops_oldest_non_error_first() {
    let mut toasts = Toasts::default();
    toasts.push(Notice::info("visible"));
    toasts.push(Notice::error("e1"));
    toasts.push(Notice::info("i1"));
    for i in 0..30 {
        toasts.push(Notice::warning(format!("w{i}")));
    }
    let texts: Vec<_> = toasts.iter().map(|t| t.notice.text.clone()).collect();
    assert_eq!(texts.len(), QUEUE_CAPACITY);
    assert_eq!(texts[0], "visible", "the one on screen stays");
    assert_eq!(texts[1], "e1", "errors are kept");
    assert!(!texts.contains(&"i1".to_string()));
    assert_eq!(texts.last().unwrap(), "w29");

    // All errors: the oldest waiting one goes.
    let mut errors = Toasts::default();
    for i in 0..25 {
        errors.push(Notice::error(format!("e{i}")));
    }
    let texts: Vec<_> = errors.iter().map(|t| t.notice.text.clone()).collect();
    assert_eq!(texts.len(), QUEUE_CAPACITY);
    assert_eq!((texts[0].as_str(), texts[1].as_str()), ("e0", "e6"));
}

#[test]
fn identical_consecutive_notices_coalesce() {
    let mut toasts = Toasts::default();
    let first = toasts.push(Notice::error("boom")).unwrap();
    // The visible one counts it and starts its time again.
    let again = toasts.push(Notice::error("boom")).unwrap();
    assert!(again.generation > first.generation);
    assert_eq!(toasts.visible().unwrap().0.count, 2);
    assert_eq!(toasts.visible().unwrap().1, 0, "nothing queued");

    // Behind another one, it counts on the newest without a timer.
    assert_eq!(toasts.push(Notice::info("other")), None);
    assert_eq!(toasts.push(Notice::info("other")), None);
    let counts: Vec<_> = toasts.iter().map(|t| t.count).collect();
    assert_eq!(counts, [2, 2]);
    // A different one in between breaks the run.
    toasts.push(Notice::error("boom"));
    assert_eq!(toasts.iter().count(), 3);
}

#[test]
fn expiry_moves_on_and_stale_timers_do_nothing() {
    let mut toasts = Toasts::default();
    let first = toasts.push(Notice::success("one")).unwrap();
    assert_eq!(first.after, Duration::from_secs(4));
    toasts.push(Notice::warning("two"));
    assert_eq!(toasts.expire(first.generation + 7), None);
    let next = toasts.expire(first.generation).unwrap();
    assert_eq!(next.after, Duration::from_secs(6));
    assert_eq!(toasts.visible().unwrap().0.notice.text, "two");
    assert_eq!(toasts.dismiss(), None);
    assert!(toasts.visible().is_none());
    assert_eq!(Level::Error.lifetime(), Duration::from_secs(8));
}

#[test]
fn time_of_day_prints_as_a_clock() {
    assert_eq!(TimeOfDay(0).to_string(), "00:00:00");
    assert_eq!(TimeOfDay(12 * 3600 + 4 * 60 + 31).to_string(), "12:04:31");
}
