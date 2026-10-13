//! What the app tells the user: [`Notice`]s, shown one at a time as toasts
//! ([`Toasts`]) and kept in a history ([`NoticeLog`]) the Messages dialog
//! lists.
//!
//! Plain data and pure functions: the reducer owns both, and the time they
//! need arrives in events, never from the clock.

use std::{collections::VecDeque, time::Duration};

/// How loud a notice is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Level {
    Info,
    Success,
    Warning,
    Error,
}

impl Level {
    /// The mark before a toast's text: `✓`, `!` or `✗`; none for info.
    pub fn glyph(self) -> &'static str {
        match self {
            Level::Info => "",
            Level::Success => "✓",
            Level::Warning => "!",
            Level::Error => "✗",
        }
    }

    /// How long its toast stays, from when it shows.
    pub fn lifetime(self) -> Duration {
        Duration::from_secs(match self {
            Level::Info | Level::Success => 4,
            Level::Warning => 6,
            Level::Error => 8,
        })
    }
}

/// One thing to tell the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub level: Level,
    pub text: String,
}

impl Notice {
    pub fn new(level: Level, text: impl Into<String>) -> Self {
        Self {
            level,
            text: text.into(),
        }
    }

    pub fn info(text: impl Into<String>) -> Self {
        Self::new(Level::Info, text)
    }

    pub fn success(text: impl Into<String>) -> Self {
        Self::new(Level::Success, text)
    }

    pub fn warning(text: impl Into<String>) -> Self {
        Self::new(Level::Warning, text)
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self::new(Level::Error, text)
    }
}

/// A wall-clock time of day, in seconds since midnight. The runtime reads
/// the clock and sends it in `UiEvent::Clock`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct TimeOfDay(pub u32);

impl std::fmt::Display for TimeOfDay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = self.0 % 86_400;
        write!(f, "{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    }
}

/// The most notices the history keeps.
pub const LOG_CAPACITY: usize = 500;
/// The most toasts kept, the visible one included.
pub const QUEUE_CAPACITY: usize = 20;

/// Every notice, newest last, up to [`LOG_CAPACITY`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoticeLog {
    entries: VecDeque<(TimeOfDay, Notice)>,
}

impl NoticeLog {
    pub fn push(&mut self, at: TimeOfDay, notice: Notice) {
        if self.entries.len() == LOG_CAPACITY {
            self.entries.pop_front();
        }
        self.entries.push_back((at, notice));
    }

    /// Newest first.
    pub fn newest_first(&self) -> impl Iterator<Item = &(TimeOfDay, Notice)> + '_ {
        self.entries.iter().rev()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// A toast and how many identical notices it stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    pub notice: Notice,
    pub count: u32,
}

/// A timer to start: deliver `Timer::Toast(generation)` after `after`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Expiry {
    pub generation: u64,
    pub after: Duration,
}

/// The toasts: the first is on screen, the rest wait, oldest first. Each
/// lives for its level's lifetime from when it shows; a timer whose
/// generation is not the current one is stale and changes nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Toasts {
    queue: VecDeque<Toast>,
    generation: u64,
}

impl Toasts {
    /// Queues `notice`. One identical to the newest is counted on it
    /// instead; if that one is on screen, its time starts again. Returns
    /// the timer to start, if the visible toast changed or restarted.
    pub fn push(&mut self, notice: Notice) -> Option<Expiry> {
        if let Some(last) = self.queue.back_mut()
            && last.notice == notice
        {
            last.count += 1;
            return (self.queue.len() == 1).then(|| self.restart());
        }
        self.queue.push_back(Toast { notice, count: 1 });
        if self.queue.len() > QUEUE_CAPACITY {
            self.drop_one();
        }
        (self.queue.len() == 1).then(|| self.restart())
    }

    /// Makes room by dropping the oldest waiting toast that is not an error,
    /// or the oldest waiting one if all are.
    fn drop_one(&mut self) {
        let waiting = 1..self.queue.len();
        let victim = waiting
            .clone()
            .find(|&i| self.queue[i].notice.level != Level::Error)
            .or_else(|| waiting.clone().next());
        if let Some(i) = victim {
            self.queue.remove(i);
        }
    }

    /// A new timer for the visible toast.
    fn restart(&mut self) -> Expiry {
        self.generation += 1;
        Expiry {
            generation: self.generation,
            after: self
                .queue
                .front()
                .map_or(Duration::ZERO, |t| t.notice.level.lifetime()),
        }
    }

    /// The visible toast's timer ran out: the next one shows. Returns its
    /// timer, if there is a next one.
    pub fn expire(&mut self, generation: u64) -> Option<Expiry> {
        if generation != self.generation || self.queue.is_empty() {
            return None;
        }
        self.queue.pop_front();
        (!self.queue.is_empty()).then(|| self.restart())
    }

    /// Takes the visible toast away early.
    pub fn dismiss(&mut self) -> Option<Expiry> {
        self.expire(self.generation)
    }

    /// The toast on screen, and how many wait behind it.
    pub fn visible(&self) -> Option<(&Toast, usize)> {
        self.queue.front().map(|t| (t, self.queue.len() - 1))
    }

    /// Every toast, the visible one first.
    pub fn iter(&self) -> impl Iterator<Item = &Toast> + '_ {
        self.queue.iter()
    }
}

#[cfg(test)]
#[path = "notice_tests.rs"]
mod notice_tests;
