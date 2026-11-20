//! Tests for [`crate::server::punch_lan`]: which LAN hosts the puzzle probes, and the probe itself.

use std::time::Duration;

use libudx::UdxRuntime;
use tokio::sync::mpsc;

use super::*;
use crate::holepuncher::{HolepunchEvent, Holepuncher};
use crate::hyperdht_messages::{
    ERROR_NONE, FIREWALL_UNKNOWN, HolepunchMessage, HolepunchPayload, MODE_FROM_RELAY, RelayInfo,
};
use crate::secure_payload::SecurePayload;
use crate::server::punch::{PunchState, Round, round};
use crate::socket_pool::SocketPool;

const SECRET: [u8; 32] = [7; 32];

fn peer(host: &str, port: u16) -> Ipv4Peer {
    Ipv4Peer {
        host: host.into(),
        port,
    }
}

fn hello(hosts: &[&str]) -> Vec<Ipv4Peer> {
    hosts.iter().map(|h| peer(h, 1111)).collect()
}

fn pairs(targets: &[Ipv4Peer]) -> Vec<String> {
    targets
        .iter()
        .map(|t| format!("{}:{}", t.host, t.port))
        .collect()
}

#[test]
fn lan_ping_targets_pair_private_hosts_with_the_peer_port() {
    let hosts = lan_hosts(true, &hello(&["11.0.0.11", "10.9.0.3", "192.168.65.1"]));
    let targets = lan_targets(&hosts, &peer("11.0.0.11", 38872));
    assert_eq!(pairs(&targets), ["10.9.0.3:38872", "192.168.65.1:38872"]);
}

#[test]
fn lan_ping_needs_same_host() {
    assert!(lan_hosts(false, &hello(&["10.9.0.3", "192.168.65.1"])).is_empty());
}

#[test]
fn lan_ping_skips_loopback_and_duplicates() {
    let all = hello(&["127.0.0.1", "10.9.0.3", "10.9.0.3", "0.0.0.0", "10.9.0.4"]);
    assert_eq!(lan_hosts(true, &all), ["10.9.0.3", "10.9.0.4"]);
}

#[test]
fn lan_ping_targets_are_capped_at_eight() {
    let all: Vec<Ipv4Peer> = (1..=20).map(|i| peer(&format!("10.1.0.{i}"), 1)).collect();
    let hosts = lan_hosts(true, &all);
    assert_eq!(hosts.len(), 8);
    assert_eq!(hosts[0], "10.1.0.1");
    assert_eq!(hosts[7], "10.1.0.8");
}

/// A puzzle on loopback with an unknown NAT estimate (never settled), plus a probe receiver.
async fn puzzle(hosts: Vec<String>) -> (PunchState, UdxRuntime, tokio::net::UdpSocket, u16) {
    let rt = UdxRuntime::new().expect("runtime");
    let pool = SocketPool::new("127.0.0.1".into());
    let (tx, _rx) = mpsc::unbounded_channel::<HolepunchEvent>();
    let p = Holepuncher::new(&pool, &rt, true, false, FIREWALL_UNKNOWN, tx)
        .await
        .expect("puzzle");
    let port = p
        .primary_socket()
        .expect("primary")
        .socket
        .local_addr()
        .await
        .expect("addr")
        .port();
    let mut st = PunchState::new(p, vec![peer("10.0.0.2", 4000)]);
    st.arm(SECRET);
    st.lan_hosts = hosts;
    let receiver = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("bind");
    (st, rt, receiver, port)
}

fn message(peer_address: Ipv4Peer, punching: bool) -> HolepunchMessage {
    let payload = HolepunchPayload {
        error: ERROR_NONE,
        firewall: FIREWALL_UNKNOWN,
        round: 1,
        connected: false,
        punching,
        addresses: None,
        remote_address: None,
        token: Some([9; 32]),
        remote_token: None,
    };
    HolepunchMessage {
        mode: MODE_FROM_RELAY,
        id: 0,
        payload: SecurePayload::new(SECRET)
            .encrypt(&payload)
            .expect("encrypt"),
        peer_address: Some(peer_address),
    }
}

fn one_round(st: &mut PunchState, client: &Ipv4Peer) {
    run_round(st, client, false);
}

fn run_round(st: &mut PunchState, client: &Ipv4Peer, punching: bool) {
    let relay = peer("198.51.100.1", 49737);
    let relays = vec![RelayInfo {
        relay_address: relay.clone(),
        peer_address: client.clone(),
    }];
    let r = round(
        st,
        &message(client.clone(), punching),
        &relay,
        client,
        &relays,
        FIREWALL_UNKNOWN,
        None,
    );
    assert!(matches!(r, Round::Reply { .. }), "round must answer");
}

#[tokio::test]
async fn lan_ping_does_not_wait_for_a_settled_nat() {
    let (mut st, _rt, rx, _) = puzzle(vec!["127.0.0.1".into()]).await;
    assert!(!st.puncher.as_ref().expect("puzzle").nat.is_settled());
    let client = peer("203.0.113.5", rx.local_addr().expect("addr").port());
    one_round(&mut st, &client);
    let mut buf = [0u8; 8];
    let got = tokio::time::timeout(Duration::from_secs(2), rx.recv_from(&mut buf)).await;
    assert!(got.is_ok(), "no probe from an unsettled puzzle");
    st.destroy();
}

#[tokio::test]
async fn round_sends_one_probe_byte_to_each_lan_target() {
    let (mut st, _rt, rx, puzzle_port) = puzzle(vec!["127.0.0.1".into()]).await;
    let client = peer("203.0.113.5", rx.local_addr().expect("addr").port());
    for _ in 0..2 {
        one_round(&mut st, &client);
        let mut buf = [0u8; 64];
        let (n, from) = tokio::time::timeout(Duration::from_secs(2), rx.recv_from(&mut buf))
            .await
            .expect("probe in 2s")
            .expect("recv");
        assert_eq!(n, 1, "probe length");
        assert_eq!(from.port(), puzzle_port, "source port");
    }
    let mut buf = [0u8; 64];
    let extra = tokio::time::timeout(Duration::from_millis(300), rx.recv_from(&mut buf)).await;
    assert!(extra.is_err(), "exactly one probe per round");
    st.destroy();
}

#[tokio::test]
async fn final_round_probes_before_the_puzzle_leaves() {
    let (mut st, _rt, rx, puzzle_port) = puzzle(vec!["127.0.0.1".into()]).await;
    let client = peer("203.0.113.5", rx.local_addr().expect("addr").port());
    run_round(&mut st, &client, true);
    let mut buf = [0u8; 64];
    let (n, from) = tokio::time::timeout(Duration::from_secs(2), rx.recv_from(&mut buf))
        .await
        .expect("probe in 2s")
        .expect("recv");
    assert_eq!(n, 1, "probe length");
    assert_eq!(from.port(), puzzle_port, "source port");
    assert!(st.puncher.is_none(), "final round hands the puncher away");
    st.destroy();
}

#[tokio::test]
async fn lan_ping_logs_one_line_per_round_only_with_targets() {
    let count = capture::start();
    let (mut with, _rt, rx, _) = puzzle(vec!["127.0.0.1".into()]).await;
    let (mut without, _rt2, _rx2, _) = puzzle(lan_hosts(false, &hello(&["10.9.0.3"]))).await;
    let port = rx.local_addr().expect("addr").port();
    let needle = format!("127.0.0.1:{port}");
    let client = peer("203.0.113.5", port);
    one_round(&mut without, &client);
    assert_eq!(count(&needle), 0, "no line without targets");
    one_round(&mut with, &client);
    one_round(&mut with, &client);
    assert_eq!(count(&needle), 2, "one line per round");
    with.destroy();
    without.destroy();
}

#[tokio::test]
async fn initiator_puncher_connects_on_a_probe_from_an_unlisted_source() {
    let rt = UdxRuntime::new().expect("runtime");
    let pool = SocketPool::new("127.0.0.1".into());
    let (tx, mut events) = mpsc::unbounded_channel::<HolepunchEvent>();
    let mut p = Holepuncher::new(&pool, &rt, true, true, FIREWALL_UNKNOWN, tx)
        .await
        .expect("initiator");
    p.update_remote(true, FIREWALL_UNKNOWN, &[peer("203.0.113.9", 5000)], None);
    let sock = p.primary_socket().expect("primary").socket.clone();
    let port = sock.local_addr().await.expect("addr").port();
    let stranger = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("bind");
    stranger
        .send_to(&[0u8], ("127.0.0.1", port))
        .await
        .expect("send");
    let ev = tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .expect("event in 2s")
        .expect("event");
    match ev {
        HolepunchEvent::Connected { addr, .. } => {
            assert_eq!(addr, stranger.local_addr().expect("addr"));
        }
        _ => panic!("expected Connected"),
    }
}

mod capture {
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex, OnceLock};

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

    static SINK: OnceLock<Sink> = OnceLock::new();

    /// Installs a process-wide debug log once (a per-thread subscriber loses events to callsite
    /// interest cached by parallel tests); tests tell their lines apart by a unique port.
    pub(super) fn start() -> impl Fn(&str) -> usize {
        let sink = SINK
            .get_or_init(|| {
                let sink = Sink::default();
                let writer = sink.clone();
                let subscriber = tracing_subscriber::fmt()
                    .with_max_level(tracing::Level::DEBUG)
                    .with_ansi(false)
                    .with_writer(move || writer.clone())
                    .finish();
                let _ = tracing::subscriber::set_global_default(subscriber);
                sink
            })
            .clone();
        move |needle: &str| {
            let bytes = sink.0.lock().expect("sink").clone();
            String::from_utf8_lossy(&bytes)
                .lines()
                .filter(|l| l.contains("lan ping sent") && l.contains(needle))
                .count()
        }
    }
}
