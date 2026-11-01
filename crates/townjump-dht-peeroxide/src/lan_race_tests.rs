// Tests of the LAN part of the adapter: responder lifecycle, the race of `connect`, the probe trigger (virtual time).
// Every test has its own group port; nodes without peers talk over loopback, the JS testnet gives a live DHT.
use crate::dht::PeeroxideDht;
use crate::factory::PeeroxideFactory;
use crate::lan_race::{ProbeStart, SILENCE, Table, wait_start};
use crate::node::LanSetup;
use crate::server::PeeroxideServer;
use std::net::{SocketAddrV4, UdpSocket};
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Notify;
use townjump_dht::{Dht, DhtError, DhtServer, Incoming, ServerOpts};
use townjump_keys::KeyPair;
use townjump_lan::{LanConfig, ProbeTiming, Responder, find};
use townjump_testkit::conformance::wait::{accept_one, within};
use townjump_testkit::key_pair;

#[allow(clippy::duplicate_mod)]
#[path = "support_tests.rs"]
mod support;

const WAIT: Duration = Duration::from_secs(30);
const CLOSED: &str = "127.0.0.1:1";
const SEED: u8 = 0xe1;

fn free_port() -> u16 {
    let socket = UdpSocket::bind("0.0.0.0:0").expect("bind");
    socket.local_addr().expect("addr").port()
}

/// Held by every test that sends or answers on the LAN. On the development Mac two probers of one process, each
/// against its own responder, lose their replies (reproduced in `townjump-lan` alone), so these tests run one by one.
static LAN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A group port of one test: unique within the process, so parallel tests never share a group.
fn group_port() -> u16 {
    static NEXT: AtomicU16 = AtomicU16::new(0);
    30_000 + (std::process::id() % 1000) as u16 * 20 + NEXT.fetch_add(1, Ordering::SeqCst)
}

fn setup(port: u16, start: ProbeStart) -> LanSetup {
    let config = LanConfig {
        port,
        ..LanConfig::default()
    };
    let timing = ProbeTiming::default();
    LanSetup {
        enabled: true,
        config,
        timing,
        start,
    }
}

async fn node(factory: &PeeroxideFactory, lan: LanSetup) -> PeeroxideDht {
    let created = within(WAIT, "create", factory.create_with_lan(None, lan)).await;
    created.expect("create")
}

fn closed_factory() -> PeeroxideFactory {
    PeeroxideFactory {
        bootstrap: Some(vec![CLOSED.to_string()]),
        ..Default::default()
    }
}

/// Peerless nodes whose sockets accept the LAN address: a loopback bootstrap would bind them to 127.0.0.1 only.
fn lan_closed_factory() -> PeeroxideFactory {
    let ip = if_addrs::get_if_addrs()
        .expect("interfaces")
        .into_iter()
        .map(|i| i.ip())
        .find(|ip| ip.is_ipv4() && !ip.is_loopback())
        .expect("a non-loopback IPv4 interface");
    PeeroxideFactory {
        bootstrap: Some(vec![format!("{ip}:1")]),
        ..Default::default()
    }
}

/// A listening server of a peerless node; `listen` returns after the bootstrap (about 4 s).
async fn serving(
    port: u16,
    factory: &PeeroxideFactory,
) -> (PeeroxideDht, PeeroxideServer, Incoming, KeyPair) {
    let dht = node(factory, setup(port, ProbeStart::Now)).await;
    let (server, incoming) = dht.create_server(ServerOpts::default()).expect("server");
    let key = key_pair(SEED);
    within(WAIT, "listen", server.listen(key.clone()))
        .await
        .expect("listen");
    (dht, server, incoming, key)
}

/// Up to three probes: a positive answer must not hang on one lost multicast frame under load.
async fn ask_some(port: u16, key: &KeyPair) -> Option<SocketAddrV4> {
    for _ in 0..3 {
        if let Some(found) = ask(port, key).await {
            return Some(found);
        }
    }
    None
}

async fn ask(port: u16, key: &KeyPair) -> Option<SocketAddrV4> {
    let target = peeroxide_dht::crypto::hash(&key.public_key);
    let timing = ProbeTiming {
        sends: vec![
            Duration::ZERO,
            Duration::from_millis(150),
            Duration::from_millis(500),
        ],
        window: Duration::from_millis(3000),
    };
    find(
        &LanConfig {
            port,
            ..LanConfig::default()
        },
        target,
        &timing,
    )
    .await
}

async fn carry<R, W>(from: &mut W, to: &mut R, bytes: &[u8]) -> Vec<u8>
where
    R: AsyncReadExt + Unpin + ?Sized,
    W: AsyncWriteExt + Unpin + ?Sized,
{
    let write = async {
        from.write_all(bytes).await.expect("write");
        from.flush().await.expect("flush")
    };
    let read = async {
        let mut got = vec![0; bytes.len()];
        to.read_exact(&mut got).await.expect("read");
        got
    };
    within(WAIT, "transfer", async { tokio::join!(write, read).1 }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lan_wins_when_the_dht_has_no_nodes() {
    let _lan = LAN.lock().await;
    let port = group_port();
    let (srv_dht, _server, mut incoming, key) = serving(port, &lan_closed_factory()).await;
    let client = node(&lan_closed_factory(), setup(port, ProbeStart::Quiet)).await;
    within(WAIT, "bootstrap", client.node.ready())
        .await
        .expect("bootstrap");
    let started = Instant::now();
    let connect = client.connect(key.public_key, Default::default());
    let (client_conn, accepted) = tokio::join!(
        within(WAIT, "connect", connect),
        accept_one(&mut incoming, "accept")
    );
    let took = started.elapsed();
    let (mut c, mut s) = (client_conn.expect("connect over the LAN"), accepted);
    // Contract: under 1 s. The macOS firewall decides per new incoming flow and slows down with the file count of the
    // binary's directory; the test runner starts the binary from a clean directory (about 50 ms here).
    assert!(took < Duration::from_secs(1), "took {took:?}");
    assert_eq!(carry(&mut c.stream, &mut s.stream, b"ping").await, b"ping");
    assert_eq!(carry(&mut s.stream, &mut c.stream, b"pong").await, b"pong");
    let _ = tokio::join!(client.destroy(), srv_dht.destroy());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dht_wins_when_nobody_answers_on_the_lan() {
    let _lan = LAN.lock().await;
    let net = support::start_testnet();
    let port = group_port();
    let off = PeeroxideFactory {
        lan_discovery: false,
        ..net.factory.clone()
    };
    let server = node(&off, setup(port, ProbeStart::Now)).await;
    let (srv, mut incoming) = server.create_server(ServerOpts::default()).expect("server");
    let key = key_pair(SEED);
    within(WAIT, "listen", srv.listen(key.clone()))
        .await
        .expect("listen");
    let client = node(&net.factory, setup(port, ProbeStart::Now)).await;
    let (conn, _) = tokio::join!(
        within(
            WAIT,
            "connect",
            client.connect(key.public_key, Default::default())
        ),
        accept_one(&mut incoming, "accept")
    );
    assert!(conn.is_ok(), "{:?}", conn.err());
    let _ = tokio::join!(client.destroy(), server.destroy());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn falls_back_to_the_dht_when_the_lan_reply_points_to_nobody() {
    let _lan = LAN.lock().await;
    let net = support::start_testnet();
    let port = group_port();
    let off = PeeroxideFactory {
        lan_discovery: false,
        ..net.factory.clone()
    };
    let server = node(&off, setup(port, ProbeStart::Now)).await;
    let (srv, mut incoming) = server.create_server(ServerOpts::default()).expect("server");
    let key = key_pair(SEED);
    within(WAIT, "listen", srv.listen(key.clone()))
        .await
        .expect("listen");
    let liar = Responder::start(&LanConfig {
        port,
        ..LanConfig::default()
    })
    .expect("fake responder");
    liar.register(peeroxide_dht::crypto::hash(&key.public_key), free_port());
    let client = node(&net.factory, setup(port, ProbeStart::Now)).await;
    let (conn, _) = tokio::join!(
        within(
            WAIT,
            "connect",
            client.connect(key.public_key, Default::default())
        ),
        accept_one(&mut incoming, "accept")
    );
    assert!(conn.is_ok(), "{:?}", conn.err());
    let _ = tokio::join!(client.destroy(), server.destroy());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fails_with_peer_not_found_when_both_paths_fail() {
    let _lan = LAN.lock().await;
    let client = node(&closed_factory(), setup(group_port(), ProbeStart::Quiet)).await;
    let failed = within(WAIT, "connect", client.connect([7; 32], Default::default())).await;
    assert!(
        matches!(failed, Err(DhtError::PeerNotFound)),
        "got {:?}",
        failed.err()
    );
    let _ = client.destroy().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn discovery_off_sends_no_probe() {
    let _lan = LAN.lock().await;
    let port = group_port();
    let sniffer = UdpSocket::bind(("0.0.0.0", port)).expect("sniffer");
    sniffer
        .set_read_timeout(Some(Duration::from_millis(200)))
        .expect("timeout");
    for iface in if_addrs::get_if_addrs().expect("interfaces") {
        if let std::net::IpAddr::V4(ip) = iface.ip() {
            let _ = sniffer.join_multicast_v4(&LanConfig::default().group, &ip);
        }
    }
    let mut buf = [0u8; 64];
    let control = ask(port, &key_pair(SEED)).await;
    assert!(
        control.is_none() && sniffer.recv(&mut buf).is_ok(),
        "the sniffer must hear a control probe"
    );
    while sniffer.recv(&mut buf).is_ok() {}
    let off = PeeroxideFactory {
        lan_discovery: false,
        ..closed_factory()
    };
    let client = node(&off, setup(port, ProbeStart::Now)).await;
    let failed = within(WAIT, "connect", client.connect([8; 32], Default::default())).await;
    assert!(
        matches!(failed, Err(DhtError::PeerNotFound)),
        "got {:?}",
        failed.err()
    );
    assert!(
        sniffer.recv(&mut buf).is_err(),
        "a probe was sent with discovery off"
    );
    let _ = client.destroy().await;
}

#[path = "lan_responder_tests.rs"]
mod responder;
