use super::*;
use std::time::Instant;

// Child processes for `spawn_read`: a shell script on Unix, a `cmd` one-liner on Windows (no `sh`, `cat`, `sleep`, `kill` there).
// `sort` waits for the end of stdin and prints nothing before it, like `cat` with no input.

#[cfg(unix)]
fn shell(script: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(script);
    cmd
}

#[cfg(windows)]
fn shell(script: &str) -> Command {
    let mut cmd = Command::new("cmd");
    cmd.arg("/C").arg(script);
    cmd
}

/// Prints `line`, then waits until its stdin is closed.
#[cfg(unix)]
fn prints_then_waits(line: &str) -> Command {
    shell(&format!("echo {line}; cat"))
}

#[cfg(windows)]
fn prints_then_waits(line: &str) -> Command {
    shell(&format!("echo {line}& sort"))
}

/// Prints nothing and keeps running for several seconds whatever happens to its stdin.
#[cfg(unix)]
fn silent_for_long() -> Command {
    shell("sleep 30")
}

#[cfg(windows)]
fn silent_for_long() -> Command {
    shell("ping -n 11 127.0.0.1 >nul")
}

fn exits_with_3() -> Command {
    shell("exit 3")
}

/// Is the process with this pid still there? (`kill -0` / `tasklist`).
#[cfg(unix)]
fn is_alive(pid: &str) -> bool {
    let probe = Command::new("kill").args(["-0", pid]).output().unwrap();
    probe.status.success()
}

#[cfg(windows)]
fn is_alive(pid: &str) -> bool {
    let filter = format!("PID eq {pid}");
    let listing = Command::new("tasklist")
        .args(["/FI", &filter, "/NH", "/FO", "CSV"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&listing.stdout).contains(&format!("\"{pid}\""))
}

fn strs(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn parse_single_and_multi() {
    let one = parse_bootstrap_line("TOWNJUMP_BOOTSTRAP=127.0.0.1:49737");
    assert_eq!(one, Some(strs(&["127.0.0.1:49737"])));
    let many = parse_bootstrap_line("TOWNJUMP_BOOTSTRAP=127.0.0.1:1,[::1]:49737,host.name:65535");
    assert_eq!(
        many,
        Some(strs(&["127.0.0.1:1", "[::1]:49737", "host.name:65535"]))
    );
}

#[test]
fn parse_trims_newline() {
    let line = "  TOWNJUMP_BOOTSTRAP=127.0.0.1:5\r\n";
    assert_eq!(parse_bootstrap_line(line), Some(strs(&["127.0.0.1:5"])));
}

#[test]
fn parse_rejects() {
    let bad = [
        "BOOTSTRAP=127.0.0.1:1",
        "hello",
        "",
        "TOWNJUMP_BOOTSTRAP=",
        "TOWNJUMP_BOOTSTRAP=127.0.0.1:0",
        "TOWNJUMP_BOOTSTRAP=127.0.0.1",
        "TOWNJUMP_BOOTSTRAP=127.0.0.1:port",
        "TOWNJUMP_BOOTSTRAP=127.0.0.1:65536",
        "TOWNJUMP_BOOTSTRAP=127.0.0.1:+80",
        "TOWNJUMP_BOOTSTRAP=:80",
        "TOWNJUMP_BOOTSTRAP=127.0.0.1:1,",
    ];
    for line in bad {
        assert_eq!(parse_bootstrap_line(line), None, "{line:?}");
    }
}

#[test]
fn script_exists() {
    let script = script_path();
    assert!(script.is_file(), "no {}", script.display());
    assert!(script.ends_with("interop/js/testnet.mjs"));
}

#[test]
fn spawn_read_ok_then_stop() {
    let cmd = prints_then_waits("TOWNJUMP_BOOTSTRAP=127.0.0.1:1,127.0.0.1:2");
    let mut testnet = spawn_read(cmd, Duration::from_secs(10)).unwrap();
    assert_eq!(testnet.bootstrap(), strs(&["127.0.0.1:1", "127.0.0.1:2"]));
    assert!(testnet.is_running());
    testnet.stop();
    assert!(!testnet.is_running());
}

#[test]
fn spawn_read_times_out_and_kills() {
    let started = Instant::now();
    let failed = spawn_read(silent_for_long(), Duration::from_millis(300))
        .err()
        .unwrap();
    assert_eq!(failed.kind(), io::ErrorKind::TimedOut);
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn spawn_read_bad_first_line() {
    let failed = spawn_read(prints_then_waits("hello"), Duration::from_secs(10))
        .err()
        .unwrap();
    assert_eq!(failed.kind(), io::ErrorKind::InvalidData);
    assert!(failed.to_string().contains("hello"), "{failed}");
}

#[test]
fn spawn_read_early_exit() {
    let failed = spawn_read(exits_with_3(), Duration::from_secs(10))
        .err()
        .unwrap();
    assert_eq!(failed.kind(), io::ErrorKind::UnexpectedEof);
}

#[test]
fn drop_kills_process() {
    let cmd = prints_then_waits("TOWNJUMP_BOOTSTRAP=127.0.0.1:1");
    let testnet = spawn_read(cmd, Duration::from_secs(10)).unwrap();
    let pid = testnet.pid().to_string();
    drop(testnet);
    assert!(!is_alive(&pid), "process {pid} is alive after drop");
}

#[test]
fn stop_is_idempotent() {
    let cmd = prints_then_waits("TOWNJUMP_BOOTSTRAP=127.0.0.1:1");
    let mut testnet = spawn_read(cmd, Duration::from_secs(10)).unwrap();
    testnet.stop();
    testnet.stop();
    assert!(!testnet.is_running());
}

#[test]
#[ignore = "needs node and .tmp/npm-holesail"]
fn real_testnet_starts_and_stops() {
    let mut testnet = JsTestnet::start(2).expect("testnet.mjs did not start");
    let bootstrap = testnet.bootstrap();
    assert!(!bootstrap.is_empty());
    for addr in &bootstrap {
        let line = format!("{PREFIX}{addr}");
        assert!(parse_bootstrap_line(&line).is_some(), "{addr}");
    }
    testnet.stop();
    assert!(!testnet.is_running());
}
