// JS: none — a port extension (`--save-nodes`); holesail JS keeps no DHT table between runs.
// `run` with `MockFactory` (it echoes the seeded nodes back from `to_array`) and a real file in a temp directory.
use std::net::SocketAddrV4;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc::UnboundedSender;
use townjump::cli::run::{CliIo, Interrupt, manual_interrupt, run};
use townjump_testkit::mock::MockFactory;
use townjump_testkit::{Capture, fixed_random};

const A: &str = "10.1.2.3:49737";
const B: &str = "10.4.5.6:49738";
const LIVE_PORT: &str = "18934";
const FILE_PORT: &str = "18933";

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn addrs(list: &[&str]) -> Vec<SocketAddrV4> {
    list.iter().map(|a| a.parse().unwrap()).collect()
}

/// A fresh directory per test (no `tempfile` crate).
fn test_dir(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("townjump-save-nodes-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_file(path: &Path, saved: u64, nodes: &[&str]) {
    let mut text = format!("townjump-nodes 1\nsaved {saved}\n");
    for node in nodes {
        text.push_str(&format!("{node}\n"));
    }
    std::fs::write(path, text).unwrap();
}

/// (`saved` stamp, node lines) of a file written by the CLI.
fn read_file(path: &Path) -> (u64, Vec<SocketAddrV4>) {
    let text = std::fs::read_to_string(path).unwrap();
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("townjump-nodes 1"));
    let saved = lines.next().unwrap().strip_prefix("saved ").unwrap();
    (
        saved.parse().unwrap(),
        lines.map(|l| l.parse().unwrap()).collect(),
    )
}

fn leftover_tmp(dir: &Path) -> Vec<String> {
    let names = std::fs::read_dir(dir).unwrap();
    let names = names.map(|e| e.unwrap().file_name().to_string_lossy().into_owned());
    names.filter(|n| n.ends_with(".tmp")).collect()
}

/// A running `run` with captured output and a manual stop signal.
struct Session {
    exit: Receiver<i32>,
    finished: Arc<AtomicBool>,
    signal: UnboundedSender<()>,
    called: Arc<AtomicBool>,
    stdout: Capture,
    stderr: Capture,
    factory: MockFactory,
}

fn start(argv: &[&str]) -> Session {
    let (interrupt, signal) = manual_interrupt();
    let called = Arc::new(AtomicBool::new(false));
    let seen = called.clone();
    let interrupt: Interrupt = Box::new(move || {
        seen.store(true, Ordering::SeqCst);
        interrupt()
    });
    let (stdout, stderr) = (Capture::new(), Capture::new());
    let io = CliIo {
        stdout: stdout.shared(),
        stderr: stderr.shared(),
        random: Box::new(fixed_random()),
        interrupt,
    };
    let factory = MockFactory::new();
    let argv = argv.iter().map(|a| a.to_string()).collect();
    let (tx, exit) = channel();
    let finished = Arc::new(AtomicBool::new(false));
    let done = finished.clone();
    let node = factory.clone();
    // `run` is not `Send`: it runs on its own thread with its own runtime.
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let code = rt.block_on(run(argv, io, node));
        done.store(true, Ordering::SeqCst);
        let _ = tx.send(code);
    });
    Session {
        exit,
        finished,
        signal,
        called,
        stdout,
        stderr,
        factory,
    }
}

impl Session {
    fn wait_for(&self, what: &str, limit: Duration, check: impl Fn(&Session) -> bool) {
        let deadline = std::time::Instant::now() + limit;
        while !check(self) {
            assert!(std::time::Instant::now() < deadline, "no {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn banner(&self) {
        let limit = Duration::from_secs(5);
        self.wait_for("banner", limit, |s| s.stdout.text().contains(" Started"));
    }

    /// Sends the signal and waits for the exit code.
    fn interrupt(&self) -> i32 {
        self.signal.send(()).unwrap();
        self.exit_code()
    }

    fn exit_code(&self) -> i32 {
        let code = self.exit.recv_timeout(Duration::from_secs(5));
        code.expect("run did not exit")
    }
}

mod flag_off {
    use super::*;

    #[test]
    fn no_handler_no_seeding_no_file() {
        let dir = test_dir("off");
        let s = start(&["--live", LIVE_PORT]);
        s.banner();
        std::thread::sleep(Duration::from_secs(1));
        assert!(
            !s.called.load(Ordering::SeqCst),
            "the signal handler was installed"
        );
        assert_eq!(s.factory.seeded_nodes(), None);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        assert!(!s.finished.load(Ordering::SeqCst));
    }
}

mod flag_on {
    use super::*;

    /// `--live` with the saved-nodes file; `extra` goes after the flag.
    fn live(file: &Path, extra: &[&str]) -> Session {
        let path = file.to_str().unwrap();
        let mut argv = vec!["--live", LIVE_PORT, "--save-nodes", path];
        argv.extend_from_slice(extra);
        start(&argv)
    }

    #[test]
    fn seeds_from_fresh_file() {
        let file = test_dir("seeds").join("nodes");
        write_file(&file, now() - 60, &[A, B]);
        let s = live(&file, &[]);
        s.banner();
        assert_eq!(s.factory.seeded_nodes(), Some(addrs(&[A, B])));
        assert_eq!(s.interrupt(), 130);
    }

    #[test]
    fn signal_saves_and_exits_130() {
        let began = now();
        let dir = test_dir("signal");
        let file = dir.join("nodes");
        write_file(&file, began - 60, &[A, B]);
        let s = live(&file, &[]);
        s.banner();
        assert_eq!(s.interrupt(), 130, "stderr: {}", s.stderr.text());
        let (saved, nodes) = read_file(&file);
        assert_eq!(nodes, addrs(&[A, B]));
        assert!(saved >= began);
        assert!(leftover_tmp(&dir).is_empty());
    }

    #[test]
    fn expired_file_is_ignored() {
        let file = test_dir("expired").join("nodes");
        write_file(&file, now() - 169 * 3600, &[A]);
        let before = std::fs::read(&file).unwrap();
        let s = live(&file, &[]);
        s.banner();
        assert_eq!(s.factory.seeded_nodes(), Some(Vec::new()));
        assert_eq!(s.interrupt(), 130);
        assert_eq!(std::fs::read(&file).unwrap(), before);
    }

    #[test]
    fn broken_file_does_not_break_start() {
        let file = test_dir("broken").join("nodes");
        std::fs::write(&file, b"\xff\xfe garbage\n\0\n").unwrap();
        let s = live(&file, &[]);
        s.banner();
        assert_eq!(s.factory.seeded_nodes(), Some(Vec::new()));
        assert_eq!(s.interrupt(), 130);
    }

    #[test]
    fn missing_file_stays_missing() {
        let file = test_dir("missing").join("nodes");
        let s = live(&file, &[]);
        s.banner();
        assert_eq!(s.interrupt(), 130);
        assert!(!file.exists());
    }

    #[test]
    fn periodic_write() {
        let began = now();
        let file = test_dir("periodic").join("nodes");
        write_file(&file, began - 60, &[A]);
        let s = live(&file, &["--save-nodes-every", "1"]);
        s.banner();
        let grown = |_: &Session| read_file(&file).0 >= began;
        s.wait_for("periodic write", Duration::from_secs(5), grown);
        assert_eq!(read_file(&file).1, addrs(&[A]));
        assert_eq!(s.interrupt(), 130);
    }

    #[test]
    fn filemanager_saves_on_signal() {
        let began = now();
        let dir = test_dir("files");
        let file = dir.join("nodes");
        write_file(&file, began - 60, &[A]);
        let (root, path) = (dir.to_str().unwrap(), file.to_str().unwrap());
        let s = start(&[
            "--filemanager",
            root,
            "--port",
            FILE_PORT,
            "--save-nodes",
            path,
        ]);
        s.banner();
        assert_eq!(s.interrupt(), 130, "stderr: {}", s.stderr.text());
        let (saved, nodes) = read_file(&file);
        assert!(saved >= began);
        assert_eq!(nodes, addrs(&[A]));
    }

    #[test]
    fn bare_flag_fails_before_node() {
        let s = start(&["--live", LIVE_PORT, "--save-nodes"]);
        assert_eq!(s.exit_code(), 1);
        assert_eq!(s.stderr.text(), "--save-nodes needs a file path\n");
        assert!(s.factory.trace().named("HyperDHT.new").is_empty());
    }

    #[test]
    fn help_ignores_flag() {
        let s = start(&["--help", "--save-nodes"]);
        assert_eq!(s.exit_code(), 0);
        let golden = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../testvectors/holesail-cli/golden/help-noargs.txt"
        );
        assert_eq!(s.stdout.text(), std::fs::read_to_string(golden).unwrap());
    }
}
