//! Descriptor and task budget of the initiator's puncher.
//!
//! File descriptors are a per-process counter and unit tests share a process, so every scenario runs in
//! a child process: a wrapper test re-executes this test binary with `FD_SCENARIO` set and requires
//! exit code 0. `fd_child` is a no-op unless that variable is present.

use super::*;
use libudx::{UdxRuntime, UdxStream};
use std::collections::HashSet;
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const FD_SLACK_PER_PUNCHER: usize = 2;
const TASK_SLACK_PER_PUNCHER: usize = 4;

/// Entries of /dev/fd minus the directory handle `read_dir` itself holds.
fn fds() -> usize {
    std::fs::read_dir("/dev/fd").map(|d| d.count()).unwrap_or(1) - 1
}

fn tasks() -> usize {
    tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks()
}

fn run_child(scenario: &str, n: usize) {
    let exe = std::env::current_exe().expect("current_exe");
    let out = std::process::Command::new("sh")
        .args(["-c", "ulimit -n 4096 2>/dev/null; exec \"$0\" \"$@\""])
        .arg(exe)
        .args([
            "holepuncher::fd_tests::fd_child",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("FD_SCENARIO", scenario)
        .env("FD_N", n.to_string())
        .output()
        .expect("spawn child");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    println!("--- child {scenario} n={n}\n{text}");
    assert!(
        out.status.success(),
        "child {scenario} n={n} failed ({:?}):\n{text}",
        out.status
    );
}

/// Remote that reflects the first datagram of every distinct source (or stays silent).
fn spawn_remote(reflect: bool) -> (SocketAddr, JoinHandle<()>) {
    let std_sock = std::net::UdpSocket::bind("127.0.0.1:0").expect("remote bind");
    std_sock.set_nonblocking(true).unwrap();
    let addr = std_sock.local_addr().unwrap();
    let h = tokio::spawn(async move {
        let sock = tokio::net::UdpSocket::from_std(std_sock).unwrap();
        let mut seen = HashSet::new();
        let mut buf = [0u8; 64];
        while let Ok((n, from)) = sock.recv_from(&mut buf).await {
            if reflect && seen.insert(from) {
                let _ = sock.send_to(&buf[..n], from).await;
            }
        }
    });
    (addr, h)
}

struct Env {
    pool: Arc<SocketPool>,
    rt: Arc<UdxRuntime>,
}

fn env() -> Env {
    Env {
        pool: Arc::new(SocketPool::new("127.0.0.1".to_string())),
        rt: Arc::new(UdxRuntime::new().expect("runtime")),
    }
}

/// Initiator behind a RANDOM NAT talking to a CONSISTENT remote at `remote`.
async fn make(
    env: &Env,
    remote: SocketAddr,
) -> (Holepuncher, mpsc::UnboundedReceiver<HolepunchEvent>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let mut p = Holepuncher::new(&env.pool, &env.rt, true, true, FIREWALL_CONSISTENT, tx)
        .await
        .expect("Holepuncher::new");
    p.nat.firewall = FIREWALL_RANDOM;
    let peer = Ipv4Peer {
        host: "127.0.0.1".into(),
        port: remote.port(),
    };
    p.update_remote(true, FIREWALL_CONSISTENT, &[peer], Some("127.0.0.1"));
    (p, rx)
}

/// Runs `punch()` the way the product race does: returns as soon as `Connected` arrives.
async fn drive(
    p: &mut Holepuncher,
    rx: &mut mpsc::UnboundedReceiver<HolepunchEvent>,
    env: &Env,
) -> bool {
    let fut = p.punch(&env.pool, &env.rt);
    tokio::pin!(fut);
    let mut done = false;
    loop {
        tokio::select! {
            ev = rx.recv() => return matches!(ev, Some(HolepunchEvent::Connected { .. })),
            _ = fut.as_mut(), if !done => { done = true; }
        }
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Success,
    Fail,
    Cancel,
    Destroy,
}

async fn one(env: Arc<Env>, remote: SocketAddr, kind: Kind) -> (bool, Option<UdxSocket>) {
    let (mut p, mut rx) = make(&env, remote).await;
    let via = p.primary_socket().map(|s| s.socket.clone());
    match kind {
        Kind::Success => {
            let ok = drive(&mut p, &mut rx, &env).await;
            (ok, via)
        }
        Kind::Fail => {
            p.punch(&env.pool, &env.rt).await;
            (false, via)
        }
        Kind::Cancel => {
            let e = env.clone();
            let _ = tokio::time::timeout(Duration::from_secs(3), async move {
                p.punch(&e.pool, &e.rt).await
            })
            .await;
            (false, via)
        }
        Kind::Destroy => {
            let _ = tokio::time::timeout(Duration::from_secs(3), p.punch(&env.pool, &env.rt)).await;
            p.destroy();
            (false, via)
        }
    }
}

async fn settle(label: &str, base_fds: usize, base_tasks: usize, fd_max: usize, task_max: usize) {
    let start = Instant::now();
    loop {
        let (f, t) = (fds(), tasks());
        let msg = format!(
            "{label}: fds {f} (base {base_fds}, max +{fd_max}), tasks {t} (base {base_tasks}, max +{task_max})"
        );
        if f <= base_fds + fd_max && t <= base_tasks + task_max {
            return println!("OK {msg}");
        }
        assert!(start.elapsed() < Duration::from_secs(5), "{msg}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn budget_scenario(kind: Kind, n: usize, reflect: bool, label: &str) {
    let env = Arc::new(env());
    let (remote, _remote_task) = spawn_remote(reflect);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (base_fds, base_tasks) = (fds(), tasks());
    let hs: Vec<_> = (0..n)
        .map(|_| tokio::spawn(one(env.clone(), remote, kind)))
        .collect();
    let mut keep = Vec::new();
    for h in hs {
        let (ok, via) = h.await.expect("join");
        if matches!(kind, Kind::Success) {
            assert!(ok, "{label}: puncher did not reach Connected");
        }
        keep.push(via);
    }
    settle(
        label,
        base_fds,
        base_tasks,
        FD_SLACK_PER_PUNCHER * n,
        TASK_SLACK_PER_PUNCHER * n,
    )
    .await;
    drop(keep);
}

/// After Connected the client builds a stream on the primary socket and exchanges data both ways
/// with the puncher already dropped.
async fn stream_scenario() {
    let env = Arc::new(env());
    let remote_sock = env.rt.create_socket().await.expect("socket");
    remote_sock
        .bind("127.0.0.1:0".parse().unwrap())
        .await
        .expect("bind");
    let remote = remote_sock.local_addr().await.expect("addr");
    let mut probes = remote_sock.recv_start().expect("recv_start");
    let sock = remote_sock.clone();
    let _reflector = tokio::spawn(async move {
        let mut seen = HashSet::new();
        while let Some(d) = probes.recv().await {
            if seen.insert(d.addr) {
                let _ = sock.send_to(&d.data, d.addr);
            }
        }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (base_fds, base_tasks) = (fds(), tasks());

    let (mut p, mut rx) = make(&env, remote).await;
    let via = p.primary_socket().expect("primary").socket.clone();
    assert!(drive(&mut p, &mut rx, &env).await, "no Connected");
    drop(p);
    let via_addr = via.local_addr().await.expect("via addr");

    let client: UdxStream = env.rt.create_stream(1).await.expect("client stream");
    client
        .connect(&via, 2, remote)
        .await
        .expect("client connect");
    let peer: UdxStream = env.rt.create_stream(2).await.expect("peer stream");
    peer.connect(&remote_sock, 1, via_addr)
        .await
        .expect("peer connect");
    let (mut c, mut s) = (client.into_async_stream(), peer.into_async_stream());

    let up: Vec<u8> = (0..16 * 1024).map(|i| (i * 7 % 251) as u8).collect();
    let down: Vec<u8> = (0..16 * 1024).map(|i| (i * 13 % 241) as u8).collect();
    let exchange = async {
        let client_side = async {
            c.write_all(&up).await.expect("client write");
            let mut got = vec![0u8; down.len()];
            c.read_exact(&mut got).await.expect("client read");
            got
        };
        let peer_side = async {
            let mut got = vec![0u8; up.len()];
            s.read_exact(&mut got).await.expect("peer read");
            s.write_all(&down).await.expect("peer write");
            got
        };
        tokio::join!(client_side, peer_side)
    };
    let (got_down, got_up) = tokio::time::timeout(Duration::from_secs(15), exchange)
        .await
        .expect("exchange timed out");
    assert_eq!(got_up, up, "client -> peer payload differs");
    assert_eq!(got_down, down, "peer -> client payload differs");
    settle("stream", base_fds, base_tasks, 1, TASK_SLACK_PER_PUNCHER).await;
    drop(via);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fd_child() {
    let Ok(scenario) = std::env::var("FD_SCENARIO") else {
        return;
    };
    let n: usize = std::env::var("FD_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    match scenario.as_str() {
        "s1" => budget_scenario(Kind::Success, n, true, "s1 connected").await,
        "s2" => budget_scenario(Kind::Fail, n, false, "s2 failed").await,
        "s3" => budget_scenario(Kind::Cancel, n, false, "s3 cancelled").await,
        "s4" => budget_scenario(Kind::Destroy, n, false, "s4 destroy").await,
        "stream" => stream_scenario().await,
        other => panic!("unknown FD_SCENARIO {other}"),
    }
}

macro_rules! shell_tests {
    ($($(#[$m:meta])* $name:ident => ($scenario:expr, $n:expr);)*) => {
        $($(#[$m])* #[test] fn $name() { run_child($scenario, $n); })*
    };
}

shell_tests! {
    punch_success_releases_sockets_n1 => ("s1", 1);
    punch_success_releases_sockets_n4 => ("s1", 4);
    punch_success_releases_sockets_n6 => ("s1", 6);
    punch_cancel_releases_sockets_n1 => ("s3", 1);
    punch_cancel_releases_sockets_n4 => ("s3", 4);
    punch_cancel_releases_sockets_n6 => ("s3", 6);
    punch_destroy_releases_sockets => ("s4", 1);
    punch_then_stream_roundtrip_keeps_one_socket => ("stream", 1);
    #[ignore = "slow (~40 s) control scenario"]
    punch_failure_releases_sockets => ("s2", 1);
}
