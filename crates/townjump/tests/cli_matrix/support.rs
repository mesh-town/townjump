// Check of a single `cli-matrix.json` case: run `run` on `MockFactory` and compare stdout, stderr, exit code and trace.
// See also: `../cli_matrix.rs` (the case list, one test per case).
// A case with a number in `afterSigint.code` (`--filemanager`): on silence a signal is sent, the code after it is compared with this number.
use serde_json::Value;
use std::future::Future;
use std::net::TcpListener;
use std::time::Duration;
use townjump::cli::run::manual_interrupt;
use townjump::{CliIo, run};
use townjump_testkit::mock::{MockConfig, MockFactory, strip_untracked};
use townjump_testkit::{
    Capture, QUIET, fixed_random, run_until_quiet, run_until_quiet_then, vectors,
};

/// Secrets of a private server with `random` = `000102…1f`: seed in hex and z32; logs keep only the fingerprint.
const SEED_HEX: &str = "6c86c6aac5fb24bcf5d9939cb7d7d5645ce39418f449e03b262dd4fa14b4b92b";
const SEED_Z32: &str = "p1dcpksf9c1m37q31qqmxi6ictqq8fya6tr6yq3gfzkxwffwzrio";
/// Log lines whose order among neighbors is undefined: `listen` goes without `await`.
const ORDER_FREE: [&str; 2] = ["Server listening on key: ", "Proxy listening on "];
/// After these words comes the OS error text, which differs in Rust.
const OS_TEXT_AFTER: &str = "Destroying connection piper due to error: ";
/// Shape of the time in the log: `d` is a digit.
const TIMESTAMP_SHAPE: &[u8; 24] = b"dddd-dd-ddTdd:dd:dd.dddZ";
/// Silence that ends `live-fake-conn` on Windows. The case waits for a refused connect to the closed port 3000:
/// Linux and macOS refuse at once, Windows retransmits the SYN and reports the refusal only after ~2 s,
/// far past `QUIET`; the 8 s ceiling of `run_until_quiet` still holds.
const WINDOWS_REFUSAL_QUIET: Duration = Duration::from_millis(4000);
/// Ports occupied before a case starts (the JSON has no `pre` step): id → port.
const BUSY_PORTS: [(&str, u16); 2] = [("conn-port-inuse", 18920), ("fm-port-inuse", 18932)];

struct Played {
    code: Option<i32>,
    /// Code after the signal: `None` — no signal was sent or `run` did not exit within 2500 ms.
    after_signal: Option<i32>,
    stdout: String,
    stderr: String,
    trace: Vec<Value>,
}

pub fn check(id: &str, needle: Option<&str>) {
    let case = load_case(id);
    let played = play(&case);
    let why = format!(
        "{id}: exit code (None — still running); stderr: {}",
        played.stderr
    );
    assert_eq!(played.code, exit_code(&case["exit"]), "{why}");
    let why = format!(
        "{id}: code after the signal (None — no signal or no exit within 2500 ms); stderr: {}",
        played.stderr
    );
    assert_eq!(
        played.after_signal,
        exit_code(&case["afterSigint"]),
        "{why}"
    );
    assert_stream(id, "stdout", &played.stdout, text(&case, "stdout"));
    match needle {
        Some(needle) => assert_error_message(id, &played.stderr, needle),
        None => assert_stream(id, "stderr", &played.stderr, text(&case, "stderr")),
    }
    let js_trace = strip_untracked(case["trace"].as_array().expect("trace"));
    assert_eq!(
        without_opts(&played.trace),
        without_opts(&js_trace),
        "{id}: DHT trace"
    );
}

/// `{ "code": number | null, .. }` → code; `null` and a missing field are `None`.
fn exit_code(exit: &Value) -> Option<i32> {
    let code = exit["code"].as_i64();
    code.and_then(|code| i32::try_from(code).ok())
}

fn text<'a>(case: &'a Value, field: &str) -> &'a str {
    case[field].as_str().expect("case field is a string")
}

fn load_case(id: &str) -> Value {
    let matrix = vectors::load_value("holesail-cli/cli-matrix.json");
    let cases = matrix.as_array().expect("the matrix is an array");
    cases
        .iter()
        .find(|case| case["id"] == id)
        .unwrap_or_else(|| panic!("no case {id}"))
        .clone()
}

fn play(case: &Value) -> Played {
    let factory = MockFactory::with_config(mock_config(case));
    let (stdout, stderr) = (Capture::new(), Capture::new());
    let random = Box::new(fixed_random());
    let (interrupt, signal) = manual_interrupt();
    let io = CliIo {
        stdout: stdout.shared(),
        stderr: stderr.shared(),
        random,
        interrupt,
    };
    let _busy = occupy_port(case);
    assert_default_port_free(case);
    let running = run(argv(case), io, factory.clone());
    let send = || signal.send(()).expect("run ignores the stop signal");
    let (code, after_signal) = drive(case, running, &[&stdout, &stderr], send);
    Played {
        code,
        after_signal,
        stdout: stdout.text(),
        stderr: stderr.text(),
        trace: factory.trace().events(),
    }
}

/// Drives `run` to silence; with a number in `afterSigint.code` sends the signal (`send`) and waits for the exit.
fn drive<Fut: Future<Output = i32>>(
    case: &Value,
    running: Fut,
    captures: &[&Capture],
    send: impl FnOnce(),
) -> (Option<i32>, Option<i32>) {
    let quiet = quiet_for(case);
    if case["afterSigint"]["code"].is_number() {
        return run_until_quiet_then(running, captures, quiet, send);
    }
    (run_until_quiet(running, captures, quiet), None)
}

/// Silence after which a case counts as finished: `QUIET`, except the refused connect of `live-fake-conn` on Windows.
fn quiet_for(case: &Value) -> Duration {
    if cfg!(windows) && case["id"] == "live-fake-conn" {
        WINDOWS_REFUSAL_QUIET
    } else {
        QUIET
    }
}

fn argv(case: &Value) -> Vec<String> {
    let args = case["args"].as_array().expect("args");
    args.iter()
        .map(|arg| arg.as_str().expect("argument is a string").to_string())
        .collect()
}

fn mock_config(case: &Value) -> MockConfig {
    let empty = serde_json::Map::new();
    let env = case["env"].as_object().unwrap_or(&empty);
    let pairs = env
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str().expect("env is a string")));
    MockConfig::from_env_pairs(pairs)
}

/// Cases from `BUSY_PORTS` (`conn-port-inuse`, `fm-port-inuse`): the port is busy before startup; we hold the socket until the case ends.
fn occupy_port(case: &Value) -> Option<TcpListener> {
    let id = case["id"].as_str()?;
    let (_, port) = BUSY_PORTS.iter().find(|(busy, _)| *busy == id)?;
    let held = TcpListener::bind(("127.0.0.1", *port));
    Some(held.unwrap_or_else(|err| panic!("port {port} is busy outside the test: {err}")))
}

/// `fm-default` listens on the default port, as JS does (`127.0.0.1:5409` in the JSON): a busy port would make the case fail for an unrelated reason.
fn assert_default_port_free(case: &Value) {
    let is_default = case["id"] == "fm-default";
    let free = || TcpListener::bind("127.0.0.1:5409").is_ok();
    assert!(
        !is_default || free(),
        "port 5409 is busy — fm-default listens on it, as JS does"
    );
}

fn assert_stream(id: &str, name: &str, actual: &str, expected: &str) {
    let actual = hide_timestamps(actual);
    let expected = if id == "help-filemanager" && name == "stdout" {
        townjump::cli::help::with_port_lines(expected)
    } else {
        expected.to_owned()
    };
    let expected = hide_timestamps(&masked_secrets(&expected));
    if id == "live-fake-conn" {
        assert_eq!(
            any_order(&actual),
            any_order(&expected),
            "{id}: {name} (lines without order)"
        );
        return;
    }
    let (rest, free) = split_order_free(&actual);
    let (want_rest, want_free) = split_order_free(&expected);
    assert_eq!(rest, want_rest, "{id}: {name}");
    assert_eq!(
        free, want_free,
        "{id}: {name}, lines with an undefined order"
    );
}

fn assert_error_message(id: &str, stderr: &str, needle: &str) {
    assert!(
        stderr.contains(needle),
        "{id}: no {needle:?} in stderr; stderr: {stderr:?}"
    );
    let has_stack =
        stderr.contains("node:") || stderr.contains("\n    at ") || stderr.contains(".js:");
    assert!(!has_stack, "{id}: the Node stack is not copied: {stderr:?}");
}

/// `HyperDHT.new` without the `opts` field: JS puts the node object there, the mock leaves it empty.
fn without_opts(events: &[Value]) -> Vec<Value> {
    let strip = |event: &Value| {
        let mut event = event.clone();
        if event["ev"] == "HyperDHT.new" {
            event.as_object_mut().map(|fields| fields.remove("opts"));
        }
        event
    };
    events.iter().map(strip).collect()
}

fn split_order_free(text: &str) -> (Vec<&str>, Vec<&str>) {
    let is_free = |line: &&str| ORDER_FREE.iter().any(|marker| line.contains(marker));
    let (mut free, rest): (Vec<&str>, Vec<&str>) = text.split_inclusive('\n').partition(is_free);
    free.sort_unstable();
    (rest, free)
}

fn any_order(text: &str) -> Vec<String> {
    let cut = |line: &str| match line.find(OS_TEXT_AFTER) {
        Some(at) => line[..at + OS_TEXT_AFTER.len()].to_string(),
        None => line.to_string(),
    };
    let mut lines: Vec<String> = text.split_inclusive('\n').map(cut).collect();
    lines.sort_unstable();
    lines
}

/// Expected JS output with raw secrets → as in Rust: the first 8 characters and `*`.
fn masked_secrets(text: &str) -> String {
    text.replace(SEED_HEX, &masked(SEED_HEX))
        .replace(SEED_Z32, &masked(SEED_Z32))
}

fn masked(secret: &str) -> String {
    let keep = |(i, c): (usize, char)| if i < 8 { c } else { '*' };
    secret.chars().enumerate().map(keep).collect()
}

/// `\x1b[90m<ISO-time>\x1b[0m` → `<ts>`: the time in the log differs every time.
fn hide_timestamps(text: &str) -> String {
    const OPEN: &str = "\x1b[90m";
    const CLOSE: &str = "\x1b[0m";
    let (mut out, mut rest) = (String::new(), text);
    while let Some(at) = rest.find(OPEN) {
        let after = &rest[at + OPEN.len()..];
        let stamped =
            is_timestamp(after.as_bytes()) && after[TIMESTAMP_SHAPE.len()..].starts_with(CLOSE);
        out.push_str(&rest[..at]);
        out.push_str(if stamped { "<ts>" } else { OPEN });
        rest = if stamped {
            &after[TIMESTAMP_SHAPE.len() + CLOSE.len()..]
        } else {
            after
        };
    }
    out.push_str(rest);
    out
}

fn is_timestamp(bytes: &[u8]) -> bool {
    let fits = |(shape, byte): (&u8, &u8)| {
        if *shape == b'd' {
            byte.is_ascii_digit()
        } else {
            shape == byte
        }
    };
    bytes.len() >= TIMESTAMP_SHAPE.len() && TIMESTAMP_SHAPE.iter().zip(bytes).all(fits)
}
