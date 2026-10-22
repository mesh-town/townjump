// Live-test helper: the JS peer `js/peer.cjs` as a child process (serve — echo server, dial — client).
// JS: events line by line `<Date.now()> <EVENT> [data]` on stdout, commands `send <text>` and `quit` on stdin.
// See also: tunnel_live_tests.rs (scenarios), townjump-testkit/src/js_testnet.rs (same technique: std::process and threads).
#![allow(dead_code)]
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

/// Log polling step: event time is taken in the reader thread, so the step does not hurt precision.
const POLL: Duration = Duration::from_millis(10);
/// Start of `node`, loading hyperdht, bootstrap and `listen` (READY).
const READY_WAIT: Duration = Duration::from_secs(30);

/// A JS peer event: when the line reached Rust, what it is called and what follows.
#[derive(Debug, Clone)]
pub struct Event {
    pub at: Instant,
    pub name: String,
    pub data: String,
    taken: bool,
}

type Journal = Arc<Mutex<Vec<Event>>>;

/// Child `node js/peer.cjs`; `Drop` kills the process.
pub struct JsPeer {
    child: Child,
    stdin: ChildStdin,
    journal: Journal,
}

fn peer_script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("js/peer.cjs")
}

fn lock(journal: &Journal) -> MutexGuard<'_, Vec<Event>> {
    journal.lock().unwrap_or_else(|e| e.into_inner())
}

/// `1700000000000 DATA ping` → (`DATA`, `ping`); a line without a timestamp (foreign output) — `None`.
fn parse_line(line: &str) -> Option<(String, String)> {
    let (stamp, rest) = line.trim_end().split_once(' ')?;
    stamp.parse::<u64>().ok()?;
    let (name, data) = rest.split_once(' ').unwrap_or((rest, ""));
    Some((name.to_owned(), data.to_owned()))
}

/// Reads stdout to the end (a closed pipe would kill node with SIGPIPE) and appends events to the log.
fn read_events(stdout: impl std::io::Read, journal: Journal) {
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        let at = Instant::now();
        if let Some((name, data)) = parse_line(&line) {
            lock(&journal).push(Event {
                at,
                name,
                data,
                taken: false,
            });
        }
    }
}

impl JsPeer {
    /// `node js/peer.cjs serve <bootstrap> <seed>` and waiting for `READY`; no `READY` within 30 s — panic.
    pub async fn serve(bootstrap: &[String], seed: [u8; 32], timeout_ms: Option<u64>) -> JsPeer {
        let peer = JsPeer::spawn("serve", bootstrap, &hex(&seed), timeout_ms, None);
        let ready = peer.wait_event("READY", READY_WAIT).await;
        assert!(ready.is_some(), "JS serve: no READY within 30 s");
        peer
    }

    /// `node js/peer.cjs dial <bootstrap> <pk>`: the connection starts by itself, the caller waits for `OPEN`.
    pub fn dial(bootstrap: &[String], pk: [u8; 32], timeout_ms: Option<u64>) -> JsPeer {
        JsPeer::spawn("dial", bootstrap, &hex(&pk), timeout_ms, None)
    }

    /// `dial` with `relayThrough: <relay>`; this client only sends and records, it does not echo back.
    pub fn dial_via_relay(bootstrap: &[String], pk: [u8; 32], relay: [u8; 32]) -> JsPeer {
        JsPeer::spawn("dial", bootstrap, &hex(&pk), None, Some(hex(&relay)))
    }

    fn spawn(
        mode: &str,
        bootstrap: &[String],
        key_hex: &str,
        timeout_ms: Option<u64>,
        relay_hex: Option<String>,
    ) -> JsPeer {
        let mut cmd = Command::new("node");
        if let Some(relay) = relay_hex {
            cmd.env("PEER_RELAY_THROUGH", relay);
        }
        cmd.arg(peer_script()).arg(mode).arg(bootstrap.join(","));
        cmd.arg(key_hex).args(timeout_ms.map(|ms| ms.to_string()));
        cmd.current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
        let spawned = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn();
        let mut child = spawned.expect("`node js/peer.cjs` did not start: node is required");
        let stdout = child.stdout.take().expect("stdout is set to pipe");
        let stdin = child.stdin.take().expect("stdin is set to pipe");
        let journal = Journal::default();
        let reader = Arc::clone(&journal);
        thread::spawn(move || read_events(stdout, reader));
        JsPeer {
            child,
            stdin,
            journal,
        }
    }

    /// First not-yet-taken event with one of the names; an event taken once is not returned again.
    pub fn take_event(&self, names: &[&str]) -> Option<Event> {
        let mut journal = lock(&self.journal);
        let found = journal
            .iter_mut()
            .find(|e| !e.taken && names.contains(&e.name.as_str()))?;
        found.taken = true;
        Some(found.clone())
    }

    /// Waits for event `name` for at most `limit`; `None` — did not arrive.
    pub async fn wait_event(&self, name: &str, limit: Duration) -> Option<Event> {
        self.wait_any(&[name], limit).await
    }

    /// Waits for the first of events `names` (in arrival order) for at most `limit`.
    pub async fn wait_any(&self, names: &[&str], limit: Duration) -> Option<Event> {
        let deadline = Instant::now() + limit;
        loop {
            if let Some(event) = self.take_event(names) {
                return Some(event);
            }
            if Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(POLL).await;
        }
    }

    /// Whether event `name` happened at all (a taken one too): for "JS printed nothing" checks.
    pub fn seen(&self, name: &str) -> bool {
        lock(&self.journal).iter().any(|e| e.name == name)
    }

    /// All events as one string `name data; …` — for the panic text.
    pub fn dump(&self) -> String {
        let journal = lock(&self.journal);
        let items: Vec<String> = journal
            .iter()
            .map(|e| format!("{} {}", e.name, e.data))
            .collect();
        items.join("; ")
    }

    /// Command to stdin: `send <text>` writes to the last socket, `quit` terminates.
    pub fn send_line(&mut self, line: &str) {
        let written = writeln!(self.stdin, "{line}").and_then(|()| self.stdin.flush());
        written.expect("JS peer stdin is closed");
    }
}

impl Drop for JsPeer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// How many milliseconds from `from` to `to`; negative — `to` is earlier than `from`.
pub fn ms_between(from: Instant, to: Instant) -> f64 {
    match to.checked_duration_since(from) {
        Some(after) => after.as_secs_f64() * 1000.0,
        None => -from.duration_since(to).as_secs_f64() * 1000.0,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
