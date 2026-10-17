//! Start-up time (T006): from spawning `qry` in a pseudo-terminal to its
//! first frame, with 500 stored connections, as the median of 20 runs.
//!
//! Timing only means something in release mode on a quiet machine, so the
//! test is ignored by default; `bench.yml` runs it nightly:
//!
//! ```sh
//! QRY_STARTUP_GATE_MS=150 cargo test --release --locked -p qry --test startup -- --ignored --nocapture
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{
    io::{Read, Write},
    path::Path,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

const RUNS: usize = 20;
const CONNECTIONS: usize = 500;
const DEFAULT_GATE_MS: u64 = 150;
/// How long one run may take before the test gives up on it.
const TIMEOUT: Duration = Duration::from_secs(10);

/// Entering the alternate screen: the app's own output starts here, so a
/// `qry:` warning on stderr before it cannot count as the frame.
const ALT_SCREEN: &[u8] = b"\x1b[?1049h";
/// The top bar's app name and mode pill, both drawn in the first frame.
const APP_NAME: &[u8] = b"qry";
const MODE_PILL: &[u8] = b"NORMAL";
/// A cursor position request, which crossterm may send; answered so the
/// app does not wait for a terminal that is not there.
const CURSOR_QUERY: &[u8] = b"\x1b[6n";

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// A data directory holding `n` Postgres connections that need a password,
/// none of which start-up may open.
fn write_connections(dir: &Path, n: usize) {
    let records: Vec<String> = (0..n)
        .map(|i| {
            format!(
                r#"    {{"id": "{i:016x}", "name": "conn {i}", "driver": "postgres", "host": "db{i}.invalid", "port": 5432, "user": "app", "database": "app", "secret": {{"mode": "prompt"}}}}"#
            )
        })
        .collect();
    let text = format!(
        "{{\n  \"version\": 1,\n  \"connections\": [\n{}\n  ]\n}}\n",
        records.join(",\n")
    );
    std::fs::write(dir.join("connections.json"), text).unwrap();
}

/// Starts `qry` once and returns how long its first frame took.
fn time_to_first_frame(config: &Path, data: &Path) -> Duration {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 40,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_qry"));
    cmd.env("QRY_CONFIG", config);
    cmd.env("QRY_DATA", data);
    cmd.env("QRY_KEYRING", "off");
    cmd.env("TERM", "xterm-256color");
    cmd.env_remove("NO_COLOR");

    let mut reader = pair.master.try_clone_reader().unwrap();
    let mut writer = pair.master.take_writer().unwrap();
    let start = Instant::now();
    let mut child = pair.slave.spawn_command(cmd).unwrap();
    drop(pair.slave);

    let (tx, rx) = mpsc::channel();
    // Not joined: once the child is killed the read fails and it ends.
    thread::spawn(move || {
        let mut seen = Vec::new();
        let mut buf = [0u8; 8192];
        let mut answered = 0;
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            seen.extend_from_slice(&buf[..n]);
            let queries = seen
                .windows(CURSOR_QUERY.len())
                .filter(|w| *w == CURSOR_QUERY)
                .count();
            for _ in answered..queries {
                let _ = writer.write_all(b"\x1b[1;1R");
            }
            answered = queries;
            if let Some(at) = find(&seen, ALT_SCREEN)
                && find(&seen[at..], APP_NAME).is_some()
                && find(&seen[at..], MODE_PILL).is_some()
            {
                let _ = tx.send(Instant::now());
                break;
            }
        }
    });

    let frame = rx.recv_timeout(TIMEOUT);
    let _ = child.kill();
    let _ = child.wait();
    drop(pair.master);
    match frame {
        Ok(at) => at - start,
        Err(_) => panic!("no first frame within {TIMEOUT:?}"),
    }
}

#[test]
#[ignore = "release-mode timing; run by bench.yml"]
fn startup_reaches_the_first_frame_under_the_gate() {
    let gate_ms = std::env::var("QRY_STARTUP_GATE_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_GATE_MS);
    let config = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    write_connections(data.path(), CONNECTIONS);

    // One untimed run warms the page cache.
    time_to_first_frame(config.path(), data.path());
    let mut times: Vec<Duration> = (0..RUNS)
        .map(|_| time_to_first_frame(config.path(), data.path()))
        .collect();
    times.sort();
    let median = (times[RUNS / 2 - 1] + times[RUNS / 2]) / 2;
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    println!(
        "start-up to first frame, {CONNECTIONS} connections, {RUNS} runs: \
         median {:.1} ms (min {:.1} ms, max {:.1} ms), gate {gate_ms} ms",
        ms(median),
        ms(times[0]),
        ms(times[RUNS - 1]),
    );
    assert!(
        median <= Duration::from_millis(gate_ms),
        "median start-up {:.1} ms is over the {gate_ms} ms gate",
        ms(median)
    );
}
