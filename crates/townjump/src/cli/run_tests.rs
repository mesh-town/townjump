// Tests of `cli::run`: mode priority, `--log` levels, randomness source, network failure → code (what the matrix does not cover pointwise).
// The matrix over `cli-matrix.json` is tests/cli_matrix.rs; here `run` with `MockFactory` and a factory that refuses.
use super::*;
use std::sync::{Arc, Mutex};
use townjump_dht::{DhtError, DhtFactory, KeyPair};
use townjump_testkit::mock::{MockDht, MockFactory};
use townjump_testkit::{Capture, QUIET, fixed_random, run_until_quiet};

const Z52: &str = "ybndrfg8ejkmcpqxot1uwisza345h769ybndrfg8ejkmcpqxot1";
const K40: &str = "abcdefghijklmnopqrstuvwxyz0123456789ABCD";
const HELP_TITLE: &str = "Holesail CLI - Instantly share and connect to servers";

mod mode_priority {
    use super::*;

    #[test]
    fn help_beats_live() {
        let dht = MockFactory::new();
        let out = go_with(&["--live", "3000", "--help"], &dht);
        assert_eq!(out.code, Some(0));
        assert!(
            out.stdout.contains(HELP_TITLE),
            "no help text: {}",
            out.stdout
        );
        assert!(
            dht.trace().events().is_empty(),
            "help must not touch the DHT"
        );
    }

    #[test]
    fn help_beats_lookup() {
        let dht = MockFactory::new();
        let out = go_with(&["--lookup", Z52, "--help"], &dht);
        assert_eq!(out.code, Some(0));
        assert!(
            out.stdout.contains(HELP_TITLE),
            "no help text: {}",
            out.stdout
        );
        assert!(
            dht.trace().events().is_empty(),
            "help must not touch the DHT"
        );
    }

    #[test]
    fn live_beats_connect() {
        let dht = MockFactory::new();
        let out = go_with(&["--live", "3000", "--connect", Z52], &dht);
        assert_eq!(out.code, None, "the server runs until stopped");
        assert_eq!(
            dht.trace().named("createServer").len(),
            1,
            "a server is needed, not a client"
        );
    }

    #[test]
    fn live_beats_lookup() {
        let dht = MockFactory::new();
        let out = go_with(&["--live", "3000", "--lookup", Z52], &dht);
        assert_eq!(out.code, None, "the server runs until stopped");
        assert!(
            dht.trace().named("dht.destroy").is_empty(),
            "lookup closes the node"
        );
    }

    #[test]
    fn connect_beats_lookup() {
        let dht = MockFactory::new();
        let out = go_with(&["--connect", Z52, "--port", "0", "--lookup", Z52], &dht);
        assert_eq!(out.code, None, "the client runs until stopped");
        assert!(
            dht.trace().named("dht.destroy").is_empty(),
            "lookup closes the node"
        );
    }
}

mod no_dht_node {
    use super::*;

    #[test]
    fn is_created_for_help_version_unknown_flag_and_bad_input() {
        let cases: [&[&str]; 5] = [
            &[],
            &["--help"],
            &["--version"],
            &["--foo"],
            &["--live", "abc"],
        ];
        for args in cases {
            let dht = MockFactory::new();
            go_with(args, &dht);
            assert!(
                dht.trace().events().is_empty(),
                "{args:?}: DHT node created for nothing"
            );
        }
    }
}

mod log_level {
    use super::*;

    #[test]
    fn negative_level_is_clamped_to_debug() {
        let out = go(&["--live", "3000", "--log=-1"]);
        assert!(out.stdout.contains("[DEBUG]"), "no DEBUG: {}", out.stdout);
        assert!(out.stdout.contains("[INFO]"), "no INFO: {}", out.stdout);
    }

    #[test]
    fn no_log_prints_no_log_lines() {
        let out = go(&["--live", "3000", "--no-log"]);
        assert!(
            !out.stdout.contains("[Holesail]"),
            "log lines: {}",
            out.stdout
        );
        assert!(out.stderr.is_empty(), "stderr: {}", out.stderr);
    }

    #[test]
    fn level_two_keeps_warnings_on_stderr_and_drops_info() {
        let out = go(&["--connect", Z52, "--port", "0", "--log=2"]);
        assert!(out.stderr.contains("[WARN]"), "no WARN: {}", out.stderr);
        assert!(
            !out.stdout.contains("[INFO]"),
            "INFO at level 2: {}",
            out.stdout
        );
    }
}

mod random {
    use super::*;

    #[test]
    fn private_server_without_key_draws_one_32_byte_seed() {
        assert_eq!(go(&["--live", "3000"]).draws, vec![32]);
    }

    #[test]
    fn public_server_without_key_draws_one_32_byte_seed() {
        assert_eq!(go(&["--live", "3000", "--public"]).draws, vec![32]);
    }

    #[test]
    fn server_with_key_draws_nothing() {
        assert!(go(&["--live", "3000", "--key", K40]).draws.is_empty());
    }

    #[test]
    fn client_draws_nothing() {
        assert!(go(&["--connect", Z52, "--port", "0"]).draws.is_empty());
    }

    #[test]
    fn lookup_draws_nothing() {
        assert!(go(&["--lookup", Z52]).draws.is_empty());
    }
}

// An unhandled error exits with code 1, the first message line goes to stderr, no Node stack. A lookup error exits with code 0.
mod network_failure {
    use super::*;

    #[test]
    fn q12_server_failure_is_exit_1_with_the_message_in_stderr() {
        let out = go_with(&["--live", "3000"], &FailingFactory);
        assert_eq!(out.code, Some(1));
        assert!(
            out.stdout.is_empty(),
            "banner before readiness: {}",
            out.stdout
        );
        assert!(out.stderr.contains("boom"), "no message: {}", out.stderr);
        assert!(
            !out.stderr.contains("\n    at "),
            "the Node stack is not copied: {}",
            out.stderr
        );
    }

    #[test]
    fn q12_client_failure_is_exit_1_with_the_message_in_stderr() {
        let out = go_with(&["--connect", Z52, "--port", "0"], &FailingFactory);
        assert_eq!(out.code, Some(1));
        assert!(out.stderr.contains("boom"), "no message: {}", out.stderr);
    }

    #[test]
    fn q14_lookup_failure_is_exit_0_with_the_message_in_stderr() {
        let out = go_with(&["--lookup", Z52], &FailingFactory);
        assert_eq!(out.code, Some(0));
        assert!(out.stdout.is_empty(), "stdout: {}", out.stdout);
        assert_eq!(out.stderr, "\x1b[31mError during lookup:\x1b[0m boom\n");
    }
}

mod filemanager {
    use super::*;
    use std::net::TcpListener;
    use townjump_testkit::run_until_quiet_then;

    /// Port 18933, not 5409: `--port 0` is falsy and gives 5409. The test brings up a real file server on this port.
    #[test]
    fn interrupt_stops_with_130_and_frees_the_port() {
        let (stdout, stderr) = (Capture::new(), Capture::new());
        let (interrupt, signal) = manual_interrupt();
        let io = CliIo {
            stdout: stdout.shared(),
            stderr: stderr.shared(),
            random: Box::new(fixed_random()),
            interrupt,
        };
        let dir = std::env::temp_dir();
        let argv = ["--filemanager", dir.to_str().unwrap(), "--port", "18933"];
        let argv = argv.iter().map(|a| a.to_string()).collect();
        let running = run(argv, io, MockFactory::new());
        let outcome = run_until_quiet_then(running, &[&stdout, &stderr], QUIET, || {
            signal.send(()).unwrap();
        });
        assert_eq!(outcome, (None, Some(130)), "stderr: {}", stderr.text());
        assert!(stdout.text().contains("Holesail Filemanager Started"));
        assert!(TcpListener::bind(("127.0.0.1", 18933)).is_ok());
    }
}

mod shutdown_signal {
    use std::time::Duration;

    // A second signal during shutdown (the first is already consumed) aborts the hung shutdown.
    #[tokio::test]
    async fn signal_aborts_a_hanging_close() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        tx.send(()).unwrap();
        let done = tokio::time::timeout(
            Duration::from_millis(100),
            super::super::filemanager::shutdown_on_signal(&mut rx, std::future::pending::<()>()),
        )
        .await;
        assert!(done.is_ok(), "shutdown was not aborted by the signal");
    }

    // Shutdown is ready at once — no signal needed.
    #[tokio::test]
    async fn finished_close_returns_without_a_signal() {
        let (_tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let done = tokio::time::timeout(
            Duration::from_millis(100),
            super::super::filemanager::shutdown_on_signal(&mut rx, async {}),
        )
        .await;
        assert!(done.is_ok(), "waits for a signal despite ready shutdown");
    }
}

struct Outcome {
    code: Option<i32>,
    stdout: String,
    stderr: String,
    /// Lengths of the buffers that `run` requested from the randomness source.
    draws: Vec<usize>,
}

fn go(args: &[&str]) -> Outcome {
    go_with(args, &MockFactory::new())
}

/// Drives `run` to the end or to silence; `code` is `None` if `run` is still working.
fn go_with<F: DhtFactory + Clone>(args: &[&str], factory: &F) -> Outcome {
    let (stdout, stderr) = (Capture::new(), Capture::new());
    let draws = Arc::new(Mutex::new(Vec::new()));
    let (interrupt, _signal) = manual_interrupt();
    let io = CliIo {
        stdout: stdout.shared(),
        stderr: stderr.shared(),
        random: counting_random(&draws),
        interrupt,
    };
    let argv = args.iter().map(|arg| arg.to_string()).collect();
    let code = run_until_quiet(run(argv, io, factory.clone()), &[&stdout, &stderr], QUIET);
    let draws = draws.lock().expect("randomness log").clone();
    Outcome {
        code,
        stdout: stdout.text(),
        stderr: stderr.text(),
        draws,
    }
}

fn counting_random(draws: &Arc<Mutex<Vec<usize>>>) -> RandomFill {
    let (draws, mut fill) = (Arc::clone(draws), fixed_random());
    Box::new(move |buf: &mut [u8]| {
        draws.lock().expect("randomness log").push(buf.len());
        fill(buf);
    })
}

/// A DHT that does not come up: `create` always returns the error `boom`.
#[derive(Clone)]
struct FailingFactory;

impl DhtFactory for FailingFactory {
    type Dht = MockDht;

    async fn create(&self, _identity: Option<KeyPair>) -> Result<MockDht, DhtError> {
        Err(DhtError::Backend("boom".to_string()))
    }
}
