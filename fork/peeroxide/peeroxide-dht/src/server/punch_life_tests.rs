//! Lifetime of the passive puzzle after `punch()` refused early.
//!
//! File descriptors are per process, so the counting scenarios run in a child process: a wrapper test
//! re-executes this test binary with `PUZZLE_SCENARIO` set and requires exit code 0. `puzzle_child` is
//! a no-op unless that variable is present.

use super::*;
use crate::hyperdht_messages::{FIREWALL_CONSISTENT, FIREWALL_OPEN, FIREWALL_RANDOM};
use std::time::{Duration, Instant};

const N: usize = 8;
const FD_SLACK: usize = 2;
const TASK_SLACK: usize = 2;

/// Entries of /dev/fd minus the directory handle `read_dir` itself holds.
fn fds() -> usize {
    std::fs::read_dir("/dev/fd").map(|d| d.count()).unwrap_or(1) - 1
}

fn tasks() -> usize {
    tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks()
}

fn run_child(scenario: &str) {
    let exe = std::env::current_exe().expect("current_exe");
    let out = std::process::Command::new("sh")
        .args(["-c", "ulimit -n 4096 2>/dev/null; exec \"$0\" \"$@\""])
        .arg(exe)
        .args([
            "server::punch::life_tests::puzzle_child",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("PUZZLE_SCENARIO", scenario)
        .output()
        .expect("spawn child");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    println!("--- child {scenario}\n{text}");
    assert!(
        out.status.success(),
        "child {scenario} failed ({:?}):\n{text}",
        out.status
    );
}

async fn settle(label: &str, base_fds: usize, base_tasks: usize) {
    let start = Instant::now();
    loop {
        let (f, t) = (fds(), tasks());
        let msg = format!(
            "{label}: fds {f} (base {base_fds}, max +{FD_SLACK}), tasks {t} (base {base_tasks}, max +{TASK_SLACK})"
        );
        if f <= base_fds + FD_SLACK && t <= base_tasks + TASK_SLACK {
            return println!("OK {msg}");
        }
        assert!(start.elapsed() < Duration::from_secs(5), "{msg}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// A passive puzzle for a server behind `local_fw`; its only remote address is unverified, so
/// `punch()` refuses at once.
async fn server_puzzle(pool: &SocketPool, rt: &UdxRuntime, local_fw: u64) -> (Holepuncher, u16) {
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut puncher = Holepuncher::new(pool, rt, true, false, FIREWALL_RANDOM, tx)
        .await
        .expect("Holepuncher::new");
    puncher.nat.firewall = local_fw;
    let unverified = Ipv4Peer {
        host: "127.0.0.1".into(),
        port: 9,
    };
    puncher.update_remote(true, FIREWALL_RANDOM, &[unverified], None);
    let (_socket, port) = socket_and_port(&puncher).await.expect("socket_and_port");
    (puncher, port)
}

async fn dropped_state_scenario() {
    let pool = SocketPool::new("127.0.0.1".into());
    let rt = UdxRuntime::new().expect("runtime");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (base_fds, base_tasks) = (fds(), tasks());
    let mut states = Vec::new();
    for _ in 0..N {
        let (puncher, _port) = server_puzzle(&pool, &rt, FIREWALL_RANDOM).await;
        let mut state = PunchState::new(puncher, Vec::new());
        assert!(state.start_punch(true));
        states.push(state);
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    drop(states);
    settle("dropped state", base_fds, base_tasks).await;
}

async fn linger_scenario() {
    let pool = SocketPool::new("127.0.0.1".into());
    let rt = UdxRuntime::new().expect("runtime");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (base_fds, base_tasks) = (fds(), tasks());
    let mut stops = Vec::new();
    for _ in 0..N {
        let (puncher, _port) = server_puzzle(&pool, &rt, FIREWALL_RANDOM).await;
        stops.push(spawn_puzzle(puncher, Duration::from_secs(1)));
    }
    tokio::time::sleep(Duration::from_millis(1500)).await;
    settle("linger timeout", base_fds, base_tasks).await;
    drop(stops);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn puzzle_child() {
    let Ok(scenario) = std::env::var("PUZZLE_SCENARIO") else {
        return;
    };
    match scenario.as_str() {
        "dropped" => dropped_state_scenario().await,
        "linger" => linger_scenario().await,
        other => panic!("unknown PUZZLE_SCENARIO {other}"),
    }
}

#[test]
fn dropped_state_releases_early_refused_puzzle() {
    run_child("dropped");
}

#[test]
fn linger_timeout_releases_early_refused_puzzle() {
    run_child("linger");
}

#[test]
fn puzzle_linger_covers_full_punch_window() {
    assert!(PUZZLE_LINGER >= Duration::from_secs(35));
}

/// A client with a RANDOM NAT punches an OPEN server whose puzzle refused the punch early: the
/// reflector must still answer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_server_without_verified_still_reflects_client_punch() {
    let pool = SocketPool::new("127.0.0.1".into());
    let rt = UdxRuntime::new().expect("runtime");
    let (puncher, port) = server_puzzle(&pool, &rt, FIREWALL_OPEN).await;
    let mut state = PunchState::new(puncher, Vec::new());
    assert!(state.start_punch(true));
    tokio::time::sleep(Duration::from_secs(2)).await;

    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut client = Holepuncher::new(&pool, &rt, true, true, FIREWALL_CONSISTENT, tx)
        .await
        .expect("client");
    client.nat.firewall = FIREWALL_RANDOM;
    let server = Ipv4Peer {
        host: "127.0.0.1".into(),
        port,
    };
    client.update_remote(true, FIREWALL_CONSISTENT, &[server], Some("127.0.0.1"));
    let fut = client.punch(&pool, &rt);
    tokio::pin!(fut);
    let mut done = false;
    let connected = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            tokio::select! {
                ev = rx.recv() => return matches!(ev, Some(HolepunchEvent::Connected { .. })),
                _ = fut.as_mut(), if !done => { done = true; }
            }
        }
    })
    .await;
    assert_eq!(connected, Ok(true), "client never reached Connected");
    drop(state);
}
