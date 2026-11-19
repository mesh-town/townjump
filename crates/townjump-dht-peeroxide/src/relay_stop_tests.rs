//! A relayed connection ends when its relay stops: both ends see the end of the stream without traffic.
//! The relay stops three ways: `close`, drop, and `SIGKILL` of a separate process (`probe_relay`).
use crate::native_testnet_tests::{
    NativeTestnet, free_udp_port, lan_host, native_testnet, relay_only,
};
use crate::{PeeroxideRelay, RelayOpts};
use std::io::{BufRead, BufReader};
use std::net::SocketAddrV4;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use townjump_dht::{ConnectOpts, DhtConnection, RelayAddress, RelayThrough, ServerOpts};
use townjump_testkit::conformance::wait::{ACCEPT_WAIT, within};
use townjump_testkit::key_pair;

const RELAY_TAG: u8 = 0xb1;
const SERVER_TAG: u8 = 0xb2;
const CLIENT_TAG: u8 = 0xb3;

#[derive(Clone, Copy, Debug)]
enum Stop {
    Close,
    Drop,
    Kill,
}

enum Relay {
    Native(Box<PeeroxideRelay>),
    Process(Child),
}

/// The `probe_relay` example binary, built (a no-op when current) into the target directory of this run.
fn probe_relay_bin() -> PathBuf {
    let target = match std::env::var_os("CARGO_TARGET_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => {
            let exe = std::env::current_exe().expect("test binary path");
            let deps = exe.parent().and_then(|p| p.parent());
            deps.and_then(|p| p.parent())
                .expect("target dir")
                .to_path_buf()
        }
    };
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let status = Command::new(cargo)
        .args(["build", "--manifest-path"])
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fork/peeroxide/Cargo.toml"
        ))
        .args(["-p", "peeroxide-dht", "--example", "probe_relay"])
        // The fork is a separate workspace: without this its build lands in the fork's own target.
        .arg("--target-dir")
        .arg(&target)
        .status()
        .expect("run cargo build");
    assert!(status.success(), "building probe_relay failed");
    let bin = target.join("debug").join("examples").join("probe_relay");
    assert!(bin.exists(), "probe_relay not found at {}", bin.display());
    bin
}

async fn start_native_relay(testnet: &NativeTestnet) -> (Relay, RelayAddress) {
    let opts = RelayOpts {
        identity: Some(key_pair(RELAY_TAG)),
        ..Default::default()
    };
    let relay = testnet.relay_with(opts).await;
    for _ in 0..50 {
        if let Ok(Some(address)) = relay.relay_address().await {
            return (Relay::Native(Box::new(relay)), address);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("the relay never announced its address");
}

async fn start_process_relay(testnet: &NativeTestnet) -> (Relay, RelayAddress) {
    let bootstrap = testnet
        .factory
        .bootstrap
        .as_ref()
        .expect("bootstrap")
        .join(",");
    let port = free_udp_port();
    let mut child = Command::new(probe_relay_bin())
        .args([
            "relay",
            "--bootstrap",
            &bootstrap,
            "--port",
            &port.to_string(),
        ])
        .args(["--seed", &"ab".repeat(32)])
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn probe_relay");
    let public_key = read_listening(&mut child).await;
    let addr = SocketAddrV4::new(lan_host().parse().expect("ip"), port);
    tokio::time::sleep(Duration::from_secs(1)).await;
    (Relay::Process(child), RelayAddress { public_key, addr })
}

/// The relay's public key from the `LISTENING <hex>` line of its stdout.
async fn read_listening(child: &mut Child) -> [u8; 32] {
    let out = child.stdout.take().expect("stdout");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if let Some(pk) = line.strip_prefix("LISTENING ") {
                let _ = tx.send(pk.to_string());
            }
        }
    });
    let pk = tokio::task::spawn_blocking(move || rx.recv_timeout(Duration::from_secs(20)))
        .await
        .expect("join")
        .expect("probe_relay prints LISTENING");
    let mut public_key = [0u8; 32];
    for (i, b) in public_key.iter_mut().enumerate() {
        *b = u8::from_str_radix(&pk[2 * i..2 * i + 2], 16).expect("hex key");
    }
    public_key
}

/// Reads until the end of the stream (`Ok(0)` or an error) and reports the moment.
fn watch_end(
    conn: DhtConnection,
    side: &'static str,
    tx: mpsc::UnboundedSender<(&'static str, Instant)>,
) {
    tokio::spawn(async move {
        let _keep_open = conn.stream;
        let (mut rd, _wr) = tokio::io::split(_keep_open);
        let mut buf = [0u8; 64];
        while matches!(rd.read(&mut buf).await, Ok(n) if n > 0) {}
        let _ = tx.send((side, Instant::now()));
    });
}

/// One 1 KiB message from the client, echoed back by the server.
async fn echo_once(client: &mut DhtConnection, server: &mut DhtConnection) {
    let sent = vec![5u8; 1024];
    client.stream.write_all(&sent).await.expect("client write");
    let mut got = vec![0u8; 1024];
    server
        .stream
        .read_exact(&mut got)
        .await
        .expect("server read");
    server.stream.write_all(&got).await.expect("server echo");
    let mut back = vec![0u8; 1024];
    client
        .stream
        .read_exact(&mut back)
        .await
        .expect("client read");
    assert_eq!(sent, back, "echo");
}

/// A relayed pair through `address` (no hole punch on either side) that has exchanged one echo.
async fn relayed_pair(
    testnet: &NativeTestnet,
    address: RelayAddress,
) -> (DhtConnection, DhtConnection) {
    let sopts = ServerOpts {
        relay_addresses: vec![address],
        holepunch: Some(Arc::new(|_| false)),
        ..Default::default()
    };
    let (_snode, _server, mut incoming) = testnet.listening(SERVER_TAG, sopts).await;
    let mut copts = ConnectOpts {
        relay_through: Some(RelayThrough::Key(address.public_key)),
        relay_addresses: vec![address],
        punch_delay: None,
        ..Default::default()
    };
    relay_only(&mut copts);
    let (_cnode, conn) = testnet
        .connect_from(CLIENT_TAG, key_pair(SERVER_TAG).public_key, copts)
        .await;
    let mut client = conn.expect("connect");
    let mut server = within(ACCEPT_WAIT, "accept", incoming.accept())
        .await
        .expect("accept");
    assert!(
        client.is_relayed() && server.is_relayed(),
        "must be relayed"
    );
    echo_once(&mut client, &mut server).await;
    (client, server)
}

/// Brings up a relayed pair without traffic, stops the relay and checks both ends end within `limit`.
async fn both_ends_close(stop: Stop, limit: Duration) {
    let testnet = native_testnet(4).await;
    let (relay, address) = match stop {
        Stop::Kill => start_process_relay(&testnet).await,
        _ => start_native_relay(&testnet).await,
    };
    let (client, server) = relayed_pair(&testnet, address).await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    watch_end(client, "client", tx.clone());
    watch_end(server, "server", tx);
    tokio::time::sleep(Duration::from_secs(1)).await;

    let t0 = Instant::now();
    match relay {
        Relay::Native(r) if matches!(stop, Stop::Close) => r.close().await,
        Relay::Native(r) => drop(r),
        Relay::Process(mut child) => {
            child.kill().expect("SIGKILL");
            let _ = child.wait();
        }
    }
    let deadline = tokio::time::Instant::now() + limit;
    let mut seen = Vec::new();
    while seen.len() < 2 {
        let event = tokio::select! {
            e = rx.recv() => e,
            () = tokio::time::sleep_until(deadline) => None,
        };
        let Some((side, at)) = event else { break };
        let after = at.saturating_duration_since(t0).as_millis();
        println!("T3A stop={stop:?} side={side} after_ms={after}");
        seen.push(side);
    }
    assert_eq!(seen.len(), 2, "ends seen within {limit:?}: {seen:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn idle_relayed_conn_closes_when_relay_closes() {
    both_ends_close(Stop::Close, Duration::from_secs(10)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn idle_relayed_conn_closes_when_relay_dropped() {
    both_ends_close(Stop::Drop, Duration::from_secs(10)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn idle_relayed_conn_closes_when_relay_killed() {
    both_ends_close(Stop::Kill, Duration::from_secs(40)).await;
}
