// JS: dht-rpc/lib/io.js
//! IO log format on request creation: the line `create_request: new inflight … via=…`
//! is a stable format for log parsers: `via=true` marks a relayed request, and the fields
//! follow in the order `tid`, `command`, `to`, then `inflight_count` and `owner`. Fixture and
//! helpers are in the parent (`io_via_tests.rs`), included from there.

use std::io::{self, Write};

use tracing::Level;

use super::*;

/// Line fields in the order the log-parsing tooling expects them.
const FIELDS_IN_ORDER: [&str; 6] = [
    "create_request: new inflight tid=",
    " command=",
    " to=",
    " via=",
    " inflight_count=",
    " owner=",
];

mod request_via_e {
    use super::*;

    #[tokio::test]
    async fn is_logged_with_via_true() {
        // given: a node and external socket E, DEBUG-level log collected into a buffer
        let mut f = setup(Opts::default()).await;
        // when: a request via E
        let lines = logged(|| {
            f.create_via(f.r_peer(), Some(Owner::Session(7)));
        });
        // then: one request-creation line, and it has via=true
        let line = single_creation_line(&lines);
        assert!(line.contains(" via=true "), "no via=true in line: {line}");
    }

    #[tokio::test]
    async fn fields_follow_the_order_of_the_via_log_parser() {
        // given: a node and external socket E, log collected into a buffer
        let mut f = setup(Opts::default()).await;
        // when: a request via E with a session owner
        let lines = logged(|| {
            f.create_via(f.r_peer(), Some(Owner::Session(7)));
        });
        // then: tid, command, to, via, inflight_count, owner, exactly in this order; owner last
        let line = single_creation_line(&lines);
        assert_in_order(line, &FIELDS_IN_ORDER);
        assert!(
            line.trim_end().ends_with(" owner=Some(Session(7))"),
            "owner must be last: {line}"
        );
    }
}

mod plain_request {
    use super::*;

    #[tokio::test]
    async fn is_logged_with_via_false_after_the_via_one() {
        // given: a node and external socket E, log collected into a buffer
        let mut f = setup(Opts::default()).await;
        // when: first a request via E, then an ordinary one with an owner
        let lines = logged(|| {
            f.create_via(f.r_peer(), Some(Owner::Session(7)));
            let params = RequestParams {
                to: f.r_peer(),
                command: COMMAND,
                ..RequestParams::default()
            };
            f.io.create_request_owned(params, Some(Owner::Session(8)));
        });
        // then: two lines in a row: first via=true, second via=false, both with fields in the expected order
        let creations = creation_lines(&lines);
        assert_eq!(creations.len(), 2, "two request-creation lines: {lines:?}");
        assert!(creations[0].contains(" via=true "), "{}", creations[0]);
        assert!(creations[1].contains(" via=false "), "{}", creations[1]);
        assert_in_order(creations[1], &FIELDS_IN_ORDER);
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Shared buffer the log subscriber writes into.
#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("sink").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Log lines (DEBUG level, no colors) printed by `body`.
fn logged(body: impl FnOnce()) -> Vec<String> {
    let sink = Sink::default();
    let writer = sink.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(Level::DEBUG)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    tracing::subscriber::with_default(subscriber, body);
    let bytes = sink.0.lock().expect("sink").clone();
    let text = String::from_utf8(bytes).expect("utf-8 log");
    text.lines().map(str::to_owned).collect()
}

fn creation_lines(lines: &[String]) -> Vec<&str> {
    let creation = |l: &&String| l.contains("create_request: new inflight");
    lines.iter().filter(creation).map(String::as_str).collect()
}

fn single_creation_line(lines: &[String]) -> &str {
    let creations = creation_lines(lines);
    assert_eq!(creations.len(), 1, "expected one creation line: {lines:?}");
    creations[0]
}

/// Each of `parts` is in `line`, and they appear in this order.
fn assert_in_order(line: &str, parts: &[&str]) {
    let mut from = 0;
    for part in parts {
        let Some(at) = line[from..].find(part) else {
            panic!("`{part}` not found after position {from} in line: {line}");
        };
        from += at + part.len();
    }
}
