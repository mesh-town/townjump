use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use socket2::SockRef;
use tokio::net::UdpSocket;

use crate::codec::{self, Reply, Request};
use crate::responder::bind_group_socket;
use crate::{LanConfig, ProbeTiming, Responder, find};

/// Tests share one host and time-sensitive assertions; running them one at a
/// time keeps interface joins of other tests from delaying multicast delivery.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const TARGET: [u8; 32] = [7; 32];
const NONCE: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

fn lan_host() -> Ipv4Addr {
    if_addrs::get_if_addrs()
        .expect("list interfaces")
        .into_iter()
        .find_map(|i| match i.ip() {
            IpAddr::V4(ip) if !i.is_loopback() => Some(ip),
            _ => None,
        })
        .expect("a non-loopback IPv4 interface is required")
}

fn cfg(port: u16) -> LanConfig {
    LanConfig {
        port,
        ..LanConfig::default()
    }
}

/// Opens a raw asking socket bound to the LAN host interface.
async fn asker() -> UdpSocket {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await.unwrap();
    socket.set_multicast_loop_v4(true).unwrap();
    socket.set_multicast_ttl_v4(1).unwrap();
    SockRef::from(&socket)
        .set_multicast_if_v4(&lan_host())
        .unwrap();
    socket
}

/// Sends one raw request for `target` and collects replies for `wait`.
async fn ask(cfg: &LanConfig, target: [u8; 32], wait: Duration) -> Vec<(Reply, Duration)> {
    ask_on(&asker().await, cfg, target, wait).await
}

async fn ask_on(
    socket: &UdpSocket,
    cfg: &LanConfig,
    target: [u8; 32],
    wait: Duration,
) -> Vec<(Reply, Duration)> {
    let packet = codec::encode_request(&Request {
        nonce: NONCE,
        target,
    });
    socket
        .send_to(&packet, SocketAddr::new(cfg.group.into(), cfg.port))
        .await
        .unwrap();
    // Latency is counted from the end of the send call: the first multicast
    // send of a process can itself take long on some hosts.
    let start = Instant::now();
    let mut out = Vec::new();
    let mut buf = [0u8; 128];
    // `wait` bounds the silence before the first reply (a new flow can be slow
    // on some hosts); once something arrives, stragglers get a short gap.
    let mut gap = wait;
    while let Ok(Ok((n, _))) = tokio::time::timeout(gap, socket.recv_from(&mut buf)).await {
        if let Some(r) = codec::decode_reply(&buf[..n]) {
            out.push((r, start.elapsed()));
            gap = Duration::from_millis(200);
        }
    }
    out
}

#[tokio::test]
async fn responder_answers_a_served_target() {
    let _serial = SERIAL.lock().await;
    let cfg = cfg(28601);
    let r = Responder::start(&cfg).unwrap();
    r.register(TARGET, 4242);
    // Warm-up on the same socket: the first packets of a new flow can take
    // 100+ ms on some hosts; the latency bound applies to the steady state.
    let socket = asker().await;
    ask_on(&socket, &cfg, TARGET, Duration::from_millis(1000)).await;
    let got = ask_on(&socket, &cfg, TARGET, Duration::from_millis(1000)).await;
    assert_eq!(got.len(), 1, "expected one reply");
    let (reply, took) = got[0];
    assert_eq!(reply.port, 4242);
    assert_eq!(reply.nonce, NONCE);
    assert_eq!(reply.target, TARGET);
    assert!(took <= Duration::from_millis(100), "slow reply: {took:?}");
}

#[tokio::test]
async fn responder_stays_silent_for_an_unknown_target() {
    let _serial = SERIAL.lock().await;
    let cfg = cfg(28602);
    let r = Responder::start(&cfg).unwrap();
    r.register(TARGET, 4242);
    assert!(
        ask(&cfg, [8; 32], Duration::from_millis(1000))
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn two_responders_on_one_host_both_answer() {
    let _serial = SERIAL.lock().await;
    let cfg = cfg(28603);
    let a = Responder::start(&cfg).unwrap();
    let b = Responder::start(&cfg).unwrap();
    a.register(TARGET, 1001);
    b.register(TARGET, 1002);
    let mut ports: Vec<u16> = ask(&cfg, TARGET, Duration::from_millis(1000))
        .await
        .iter()
        .map(|(r, _)| r.port)
        .collect();
    ports.sort();
    assert_eq!(ports, vec![1001, 1002]);
}

#[tokio::test]
async fn prober_returns_none_when_nobody_answers_within_the_window() {
    let _serial = SERIAL.lock().await;
    let cfg = cfg(28604);
    let start = Instant::now();
    let got = find(&cfg, TARGET, &ProbeTiming::default()).await;
    let took = start.elapsed();
    assert_eq!(got, None);
    assert!(took >= Duration::from_millis(1400), "too early: {took:?}");
    assert!(took < Duration::from_millis(2500), "too late: {took:?}");
}

#[tokio::test]
async fn prober_finds_a_served_target() {
    let _serial = SERIAL.lock().await;
    let cfg = cfg(28607);
    let r = Responder::start(&cfg).unwrap();
    r.register(TARGET, 4242);
    let got = find(&cfg, TARGET, &ProbeTiming::default()).await.unwrap();
    assert_eq!(got.port(), 4242);
    assert!(
        crate::interfaces::ipv4_interfaces().contains(got.ip()),
        "reply source is not a local interface: {got}"
    );
}

#[tokio::test]
async fn prober_ignores_a_reply_with_a_foreign_nonce() {
    let _serial = SERIAL.lock().await;
    let cfg = cfg(28605);
    let socket = bind_group_socket(&cfg).unwrap();
    let fake = tokio::spawn(async move {
        let mut buf = [0u8; 128];
        let (n, src) = socket.recv_from(&mut buf).await.unwrap();
        let req = codec::decode_request(&buf[..n]).unwrap();
        let mut bad = req.nonce;
        bad[0] ^= 0xff;
        for (nonce, port) in [(bad, 1111), (req.nonce, 2222)] {
            let reply = codec::encode_reply(&Reply {
                nonce,
                target: req.target,
                port,
            });
            socket.send_to(&reply, src).await.unwrap();
        }
    });
    let got = find(&cfg, TARGET, &ProbeTiming::default()).await;
    fake.abort();
    assert_eq!(got.map(|a| a.port()), Some(2222));
}

#[tokio::test]
async fn unregister_stops_answering() {
    let _serial = SERIAL.lock().await;
    let cfg = cfg(28606);
    let r = Responder::start(&cfg).unwrap();
    r.register(TARGET, 4242);
    assert_eq!(
        ask(&cfg, TARGET, Duration::from_millis(1000)).await.len(),
        1
    );
    r.unregister(&TARGET);
    assert!(
        ask(&cfg, TARGET, Duration::from_millis(1000))
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn paused_responder_stays_silent() {
    let _serial = SERIAL.lock().await;
    let cfg = cfg(28608);
    let r = Responder::start(&cfg).unwrap();
    r.register(TARGET, 4242);
    r.set_paused(true);
    assert!(
        ask(&cfg, TARGET, Duration::from_millis(1000))
            .await
            .is_empty()
    );
    r.set_paused(false);
    assert_eq!(
        ask(&cfg, TARGET, Duration::from_millis(1000)).await.len(),
        1
    );
}

#[tokio::test]
async fn prober_ignores_a_foreign_target_and_a_zero_port() {
    let _serial = SERIAL.lock().await;
    let cfg = cfg(28610);
    let socket = bind_group_socket(&cfg).unwrap();
    let fake = tokio::spawn(async move {
        let mut buf = [0u8; 128];
        let (n, src) = socket.recv_from(&mut buf).await.unwrap();
        let req = codec::decode_request(&buf[..n]).unwrap();
        for (target, port) in [([8; 32], 1111), (req.target, 0), (req.target, 2222)] {
            let reply = codec::encode_reply(&Reply {
                nonce: req.nonce,
                target,
                port,
            });
            socket.send_to(&reply, src).await.unwrap();
        }
    });
    let got = find(&cfg, TARGET, &ProbeTiming::default()).await;
    fake.abort();
    assert_eq!(got.map(|a| a.port()), Some(2222));
}

#[tokio::test]
async fn prober_repeats_the_request_on_schedule() {
    let _serial = SERIAL.lock().await;
    let cfg = cfg(28611);
    let socket = bind_group_socket(&cfg).unwrap();
    let fake = tokio::spawn(async move {
        let mut buf = [0u8; 128];
        let mut first: Option<Instant> = None;
        loop {
            let (n, src) = socket.recv_from(&mut buf).await.unwrap();
            let first = *first.get_or_insert_with(Instant::now);
            if first.elapsed() < Duration::from_millis(600) {
                continue;
            }
            let req = codec::decode_request(&buf[..n]).unwrap();
            let reply = codec::encode_reply(&Reply {
                nonce: req.nonce,
                target: req.target,
                port: 3333,
            });
            socket.send_to(&reply, src).await.unwrap();
        }
    });
    let timing = ProbeTiming {
        sends: vec![Duration::ZERO, Duration::from_millis(1000)],
        window: Duration::from_millis(2000),
    };
    let got = find(&cfg, TARGET, &timing).await;
    fake.abort();
    assert_eq!(got.map(|a| a.port()), Some(3333));
}

#[tokio::test]
async fn closed_or_dropped_responder_stays_silent() {
    let _serial = SERIAL.lock().await;
    let wait = Duration::from_millis(1000);
    let a_cfg = cfg(28612);
    let a = Responder::start(&a_cfg).unwrap();
    a.register(TARGET, 4242);
    assert_eq!(ask(&a_cfg, TARGET, wait).await.len(), 1);
    a.close();
    assert!(ask(&a_cfg, TARGET, wait).await.is_empty());

    let b_cfg = cfg(28613);
    let b = Responder::start(&b_cfg).unwrap();
    b.register(TARGET, 4242);
    assert_eq!(ask(&b_cfg, TARGET, wait).await.len(), 1);
    drop(b);
    assert!(ask(&b_cfg, TARGET, wait).await.is_empty());
}
