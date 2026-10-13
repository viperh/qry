//! The runtime: the one place that touches the terminal, spawns tasks, reads
//! and writes files and calls the keychain.
//!
//! It turns terminal input, timer ticks, worker answers and file outcomes
//! into [`UiEvent`]s, feeds them to [`App::handle`], carries out the
//! [`Effect`]s that come back and draws when something changed. Every
//! decision is the reducer's; this only does what it is told.

use std::{path::PathBuf, time::Instant};

use ratatui::prelude::Rect;
use tokio::{
    sync::mpsc::{self, UnboundedReceiver, UnboundedSender},
    task::JoinHandle,
};
use tracing::debug;

use crate::{
    app::{App, DbAnswer, Effect, FileAnswer, SessionId, UiEvent},
    config::{Config, get_data_dir},
    connections::{self, StoredConnection},
    db::{self, DbCommand},
    import::Format,
    secrets,
    theme::{self, ColorSupport},
    tui::{Event, Tui},
    views,
};

pub struct Runtime {
    app: App,
    tick_rate: f64,
    frame_rate: f64,
    executor: Executor,
    /// The workers' answers.
    answers: UnboundedReceiver<(SessionId, DbAnswer)>,
    /// What the blocking tasks found, and anything else the executor reports.
    events: UnboundedReceiver<UiEvent>,
}

/// Shows the app and the rates; the channels have no useful `Debug`.
impl std::fmt::Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runtime")
            .field("app", &self.app)
            .field("tick_rate", &self.tick_rate)
            .field("frame_rate", &self.frame_rate)
            .finish_non_exhaustive()
    }
}

impl Runtime {
    /// Reads the config and resolves the theme. `theme` is the `--theme`
    /// flag, which wins over the config's `theme`. An unknown theme name is
    /// an error, before the terminal is touched.
    pub fn new(tick_rate: f64, frame_rate: f64, theme: Option<&str>) -> color_eyre::Result<Self> {
        let config = Config::new()?;
        for warning in config.warnings() {
            tracing::warn!("{warning}");
            eprintln!("qry: {warning}");
        }
        let theme = theme::resolve(theme, config.theme.as_deref(), ColorSupport::detect())?;
        let (executor, answers, events) = Executor::new(get_data_dir());
        Ok(Self {
            app: App::new(config, theme, connections::id_seed()),
            tick_rate,
            frame_rate,
            executor,
            answers,
            events,
        })
    }

    pub async fn run(&mut self) -> color_eyre::Result<()> {
        let mut tui = Tui::new()?
            // .mouse(true) // uncomment this line to enable mouse support
            .tick_rate(self.tick_rate)
            .frame_rate(self.frame_rate);
        tui.enter()?;

        let mut next = Some(UiEvent::Started);
        loop {
            let event = match next.take() {
                Some(event) => event,
                None => {
                    let incoming = tokio::select! {
                        event = tui.next_event() => Incoming::Terminal(event),
                        Some((id, answer)) = self.answers.recv() => Incoming::Event(UiEvent::Db(id, answer)),
                        Some(event) = self.events.recv() => Incoming::Event(event),
                    };
                    match incoming {
                        Incoming::Event(event) => event,
                        Incoming::Terminal(Some(Event::Key(key))) => UiEvent::Key(key),
                        Incoming::Terminal(Some(Event::Tick)) => UiEvent::Tick(Instant::now()),
                        Incoming::Terminal(Some(Event::Resize(w, h))) => {
                            tui.resize(Rect::new(0, 0, w, h))?;
                            UiEvent::Resize(w, h)
                        }
                        Incoming::Terminal(Some(Event::Render)) => {
                            self.draw(&mut tui)?;
                            continue;
                        }
                        Incoming::Terminal(_) => continue,
                    }
                }
            };
            log(&event);

            let mut quit = false;
            for effect in self.app.handle(event) {
                debug!("{effect:?}");
                match self.executor.execute(effect) {
                    Executed::Terminal(Effect::Quit) => quit = true,
                    Executed::Terminal(Effect::Suspend) => {
                        tui.suspend()?;
                        // tui.mouse(true);
                        tui.enter()?;
                        tui.terminal.clear()?;
                    }
                    Executed::Terminal(_) => tui.terminal.clear()?,
                    Executed::Spawned(_) | Executed::Done => {}
                }
            }
            if quit {
                tui.stop()?;
                break;
            }
            if self.app.take_dirty() {
                self.draw(&mut tui)?;
            }
        }
        tui.exit()?;
        Ok(())
    }

    fn draw(&mut self, tui: &mut Tui) -> color_eyre::Result<()> {
        let state = self.app.state();
        tui.draw(|frame| views::draw(state, &state.theme, frame, frame.area()))?;
        Ok(())
    }
}

/// What woke the loop.
enum Incoming {
    Terminal(Option<Event>),
    Event(UiEvent),
}

/// Logs what came in. Ticks are too many to be worth it, and a query's
/// result is never logged in full: `DbAnswer`'s `Debug` gives only its size,
/// and it is skipped here besides.
fn log(event: &UiEvent) {
    if let Some(line) = log_line(event) {
        debug!("{line}");
    }
}

pub(crate) fn log_line(event: &UiEvent) -> Option<String> {
    match event {
        UiEvent::Tick(_) | UiEvent::Db(_, DbAnswer::QueryDone(_)) => None,
        event => Some(format!("{event:?}")),
    }
}

/// How the executor dealt with an effect.
#[derive(Debug)]
pub(crate) enum Executed {
    /// It needs the terminal (`Quit`, `Suspend`, `ClearScreen`), which the
    /// loop holds, so it is handed back.
    Terminal(Effect),
    /// It runs off the loop; its outcome arrives as an event.
    #[allow(
        dead_code,
        reason = "the loop lets the task run; tests await the handle"
    )]
    Spawned(JoinHandle<()>),
    /// It is done.
    Done,
}

/// Carries out effects that need no terminal: the worker, the files and the
/// keychain. Blocking work goes to `spawn_blocking`, so the loop never waits
/// on a disk or a keychain.
pub(crate) struct Executor {
    /// Where the connections file lives.
    data_dir: PathBuf,
    /// The one session's worker.
    db: UnboundedSender<DbCommand>,
    events: UnboundedSender<UiEvent>,
}

impl Executor {
    /// An executor writing under `data_dir`, with its worker started, and
    /// where the worker's answers and the files' outcomes arrive.
    pub(crate) fn new(
        data_dir: PathBuf,
    ) -> (
        Self,
        UnboundedReceiver<(SessionId, DbAnswer)>,
        UnboundedReceiver<UiEvent>,
    ) {
        let (answers_tx, answers) = mpsc::unbounded_channel();
        let (events_tx, events) = mpsc::unbounded_channel();
        let executor = Self {
            data_dir,
            db: db::spawn(SessionId::FIRST, answers_tx),
            events: events_tx,
        };
        (executor, answers, events)
    }

    pub(crate) fn execute(&self, effect: Effect) -> Executed {
        match effect {
            Effect::Db(id, command) => {
                self.send_db(id, command);
                Executed::Done
            }
            // There is one worker for now: opening a session connects it.
            Effect::OpenSession(id, record) => {
                self.send_db(id, DbCommand::Connect(record));
                Executed::Done
            }
            Effect::SaveConnections(list) => Executed::Spawned(self.save_connections(list)),
            Effect::LoadConnections => Executed::Spawned(self.load_connections()),
            Effect::Import { path, format } => Executed::Spawned(self.import(path, format)),
            Effect::ExportConnections { path, connections } => {
                Executed::Spawned(self.export_connections(path, connections))
            }
            Effect::ForgetSecret(id) => Executed::Spawned(tokio::task::spawn_blocking(move || {
                if let Err(e) = secrets::delete(&id) {
                    tracing::error!("cannot remove the stored password: {e:#}");
                }
            })),
            Effect::StashPassword {
                connection,
                password,
            } => {
                secrets::stash(&connection, password);
                Executed::Done
            }
            Effect::ScheduleTimer { timer, after } => {
                let events = self.events.clone();
                Executed::Spawned(tokio::spawn(async move {
                    tokio::time::sleep(after).await;
                    let _ = events.send(UiEvent::Timer(timer));
                }))
            }
            effect @ (Effect::ClearScreen | Effect::Suspend | Effect::Quit) => {
                Executed::Terminal(effect)
            }
        }
    }

    /// Hands a command to the database worker. If the worker has stopped,
    /// says so instead of failing silently.
    fn send_db(&self, id: SessionId, command: DbCommand) {
        if self.db.send(command).is_err() {
            self.report(UiEvent::Db(id, DbAnswer::WorkerStopped));
        }
    }

    fn report(&self, event: UiEvent) {
        // The loop holds the receiver for as long as it runs.
        let _ = self.events.send(event);
    }

    /// Writes the connections off the event loop, reporting a failure
    /// without losing the change the user just made.
    fn save_connections(&self, list: Vec<StoredConnection>) -> JoinHandle<()> {
        let (dir, events) = (self.data_dir.clone(), self.events.clone());
        tokio::task::spawn_blocking(move || {
            if let Err(e) = connections::save_to(&dir, &list) {
                let message = format!("cannot save connections: {e:#}");
                tracing::error!("{message}");
                let _ = events.send(UiEvent::File(FileAnswer::SaveFailed(message)));
            }
        })
    }

    /// Reads the saved connections off the event loop.
    fn load_connections(&self) -> JoinHandle<()> {
        let (dir, events) = (self.data_dir.clone(), self.events.clone());
        tokio::task::spawn_blocking(move || {
            let answer = match connections::load_from(&dir) {
                Ok(list) => FileAnswer::ConnectionsLoaded(list),
                Err(e) => {
                    let message = format!("{e:#}");
                    tracing::error!("{message}");
                    FileAnswer::LoadFailed(message)
                }
            };
            let _ = events.send(UiEvent::File(answer));
        })
    }

    /// Reads a file of connections off the event loop. The answer goes to the
    /// modal that asked and to the tree, which adds what is new.
    fn import(&self, path: String, format: Format) -> JoinHandle<()> {
        let events = self.events.clone();
        tokio::task::spawn_blocking(move || {
            let answer = match crate::import::from_file(&path, format) {
                Ok(imported) if imported.connections.is_empty() => {
                    FileAnswer::ImportFailed(match imported.skipped.first() {
                        Some(first) => format!(
                            "No connection qry can open; {} {}",
                            first.name, first.reason
                        ),
                        None => "The file holds no connections".into(),
                    })
                }
                Ok(imported) => FileAnswer::Imported(Box::new(imported)),
                Err(e) => FileAnswer::ImportFailed(format!("{e:#}")),
            };
            let _ = events.send(UiEvent::File(answer));
        })
    }

    /// Writes a backup of the connections off the event loop, answering the
    /// modal that asked.
    fn export_connections(&self, path: String, list: Vec<StoredConnection>) -> JoinHandle<()> {
        let events = self.events.clone();
        tokio::task::spawn_blocking(move || {
            let path = crate::import::expand_home(&path);
            let answer = match connections::export(&path, &list) {
                Ok(written) => FileAnswer::ConnectionsExported {
                    path: written.display().to_string(),
                    count: list.len(),
                },
                Err(e) => FileAnswer::ConnectionsExportFailed(format!("{e:#}")),
            };
            let _ = events.send(UiEvent::File(answer));
        })
    }
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod runtime_tests;
