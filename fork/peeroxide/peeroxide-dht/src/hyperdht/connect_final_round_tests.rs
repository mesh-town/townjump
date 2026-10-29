//! Tests for [`super`]: where the client's UDX stream lands after the final punch round.
//!
//! The peer stands for a CONSISTENT server behind a port-restricted NAT: it answers a probe from one
//! birthday socket and accepts UDX packets only from that socket's port. Each scenario runs in a child
//! process (as in `holepuncher::fd_tests`): the puncher opens up to 256 sockets without pauses and a
//! default descriptor limit may be as low as 256. A wrapper test re-executes this test binary with
//! `C1_SCENARIO` set and requires exit code 0; `c1_child` is a no-op unless that variable is present.

use super::*;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::task::JoinHandle;

use crate::hyperdht_messages::{FIREWALL_CONSISTENT, FIREWALL_RANDOM, HolepunchInfo};

/// The peer replies to the K-th distinct source other than the primary: the winner is a birthday socket.
const K: usize = 3;
const EXCHANGE: Duration = Duration::from_secs(8);
const PAYLOAD: usize = 16 * 1024;

mod port_restricted_peer {
    use super::*;

    #[test]
    fn stream_follows_probe_socket_under_port_restricted_peer() {
        run_child("strict");
    }
}

mod unrestricted_peer {
    use super::*;

    #[test]
    fn stream_roundtrip_when_peer_accepts_any_port() {
        run_child("any");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c1_child() {
    let Ok(scenario) = std::env::var("C1_SCENARIO") else {
        return;
    };
    match scenario.as_str() {
        "strict" => scenario_run(true).await,
        "any" => scenario_run(false).await,
        other => panic!("unknown C1_SCENARIO {other}"),
    }
}

// ---- setup and helpers ----

fn run_child(scenario: &str) {
    let exe = std::env::current_exe().expect("current_exe");
    let out = std::process::Command::new("sh")
        .args(["-c", "ulimit -n 4096 2>/dev/null; exec \"$0\" \"$@\""])
        .arg(exe)
        .args([
            "hyperdht::connect_final_round::tests::c1_child",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("C1_SCENARIO", scenario)
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

/// Peer standing for a CONSISTENT server behind a port-restricted NAT.
struct Peer {
    sock: UdxSocket,
    addr: SocketAddr,
    winner: Arc<Mutex<Option<SocketAddr>>>,
}

async fn bind_peer(rt: &UdxRuntime) -> Peer {
    let sock = rt.create_socket().await.expect("peer socket");
    sock.bind("127.0.0.1:0".parse().unwrap())
        .await
        .expect("peer bind");
    let addr = sock.local_addr().await.expect("peer addr");
    Peer {
        sock,
        addr,
        winner: Arc::new(Mutex::new(None)),
    }
}

/// Echoes the probe of the K-th distinct source other than `primary`, once; records it as the winner.
fn start_reflector(peer: &Peer, primary: SocketAddr) -> JoinHandle<()> {
    let mut probes = peer.sock.recv_start().expect("recv_start");
    let sock = peer.sock.clone();
    let winner = peer.winner.clone();
    tokio::spawn(async move {
        let mut seen = HashSet::new();
        while let Some(d) = probes.recv().await {
            if d.addr == primary || !seen.insert(d.addr) || seen.len() != K {
                continue;
            }
            *winner.lock().unwrap() = Some(d.addr);
            let _ = sock.send_to(&d.data, d.addr);
        }
    })
}

fn nw_result() -> NoiseWrapResult {
    NoiseWrapResult {
        is_initiator: true,
        remote_public_key: [1; 32],
        holepunch_secret: [7; 32],
        handshake_hash: [0; 64],
        tx: [0; 32],
        rx: [0; 32],
    }
}

fn remote_payload() -> NoisePayload {
    NoisePayload {
        version: 1,
        error: 0,
        firewall: FIREWALL_CONSISTENT,
        holepunch: Some(HolepunchInfo {
            id: 1,
            relays: vec![],
        }),
        addresses4: vec![],
        addresses6: vec![],
        udx: None,
        secret_stream: None,
        relay_through: None,
        relay_addresses: None,
    }
}

/// Client puncher (RANDOM vs CONSISTENT `peer`) driven through `race_punch` to a `ConnectResult`.
async fn punch(
    env: &Env,
    peer: &Peer,
) -> (
    ConnectResult,
    Holepuncher,
    UnboundedReceiver<HolepunchEvent>,
) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut p = Holepuncher::new(&env.pool, &env.rt, true, true, FIREWALL_CONSISTENT, tx)
        .await
        .expect("Holepuncher::new");
    p.nat.firewall = FIREWALL_RANDOM;
    let peer_ipv4 = Ipv4Peer {
        host: "127.0.0.1".into(),
        port: peer.addr.port(),
    };
    p.update_remote(
        true,
        FIREWALL_CONSISTENT,
        std::slice::from_ref(&peer_ipv4),
        Some("127.0.0.1"),
    );
    let primary = p.primary_socket().expect("primary").socket.clone();
    let primary_addr = primary.local_addr().await.expect("primary addr");
    let _reflector = start_reflector(peer, primary_addr);

    let (nw, payload, sp) = (nw_result(), remote_payload(), SecurePayload::new([7; 32]));
    let fr = FinalRound {
        nw_result: &nw,
        remote_payload: &payload,
        relay: &peer_ipv4,
        target: &[9; 32],
        runtime: &env.rt,
        pool: &env.pool,
        sp: &sp,
        local_stream_id: 1,
        hp_id: 1,
        via: Some(primary),
        deadline: tokio::time::Instant::now() + Duration::from_secs(10),
        verified_addr: peer_ipv4.clone(),
        remote_token: None,
        round: 1,
        addresses: vec![],
    };
    let result = race_punch(fr, &mut p, &mut rx).await.expect("Connected");
    (result, p, rx)
}

/// Punch, drop the puncher, run a stream on `result.udx_socket` to the peer and exchange data both ways.
/// `strict`: the peer's firewall hook accepts the first UDX packet only from the winner's port.
async fn scenario_run(strict: bool) {
    let env = env();
    let peer = bind_peer(&env.rt).await;
    let (result, p, _rx) = punch(&env, &peer).await;
    drop(p);
    let winner = peer.winner.lock().unwrap().expect("reflected");
    let udx_socket = result.udx_socket.as_ref().expect("udx_socket in result");

    let peer_stream = env.rt.create_stream(2).await.expect("peer stream");
    peer_stream
        .set_firewall_hook(&peer.sock, 1, move |_, port, _| {
            !strict || port == winner.port()
        })
        .expect("firewall hook");
    let client_stream = env.rt.create_stream(1).await.expect("client stream");
    client_stream
        .connect(udx_socket, 2, peer.addr)
        .await
        .expect("client connect");
    let (mut c, mut s) = (
        client_stream.into_async_stream(),
        peer_stream.into_async_stream(),
    );

    let up: Vec<u8> = (0..PAYLOAD).map(|i| (i * 7 % 251) as u8).collect();
    let down: Vec<u8> = (0..PAYLOAD).map(|i| (i * 13 % 241) as u8).collect();
    let exchange = async {
        let client_side = async {
            c.write_all(&up).await.expect("client write");
            let mut got = vec![0u8; down.len()];
            c.read_exact(&mut got).await.expect("client read");
            got
        };
        // The peer reads first: its stream has no remote address until the hook fires.
        let peer_side = async {
            let mut got = vec![0u8; up.len()];
            s.read_exact(&mut got).await.expect("peer read");
            s.write_all(&down).await.expect("peer write");
            got
        };
        tokio::join!(client_side, peer_side)
    };
    let (got_down, got_up) = tokio::time::timeout(EXCHANGE, exchange)
        .await
        .expect("exchange timed out: stream not on the probe socket?");
    assert_eq!(got_up, up, "client -> peer payload differs");
    assert_eq!(got_down, down, "peer -> client payload differs");
    if strict {
        let port = udx_socket.local_addr().await.expect("local addr").port();
        assert_eq!(port, winner.port(), "stream is not on the winner socket");
    }
}
