//! The relay-to-direct switch over a real blind relay on loopback (two peers, two sockets each).
// JS: hyperdht/lib/connect.js:793-838

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use libudx::{StreamControl, UdxRuntime, UdxSocket, UdxStream};
use sha2::{Digest, Sha256};
use tokio::sync::watch;

use super::super::RelayPath;
use super::super::pair::{RelayPaired, RelaySession, pair_via_relay};
use super::super::test_support::{TestNet, TestPeer, net};
use super::*;

const KEEP_ALIVE: Duration = Duration::from_secs(5);
const CHUNK: usize = 512 * 1024;

/// One peer: its stream on the relay path, a second socket for the direct path.
struct Side {
    stream: UdxStream,
    control: StreamControl,
    session: Option<RelaySession>,
    second: UdxSocket,
    second_addr: SocketAddr,
    own_id: u32,
}

async fn pair(n: &TestNet, p: &TestPeer, token: [u8; 32], init: bool, id: u32) -> RelayPaired {
    let relay = n.relay_pk;
    pair_via_relay(
        &p.handle,
        &p.key_pair,
        &p.rt,
        relay,
        token,
        init,
        id,
        KEEP_ALIVE,
    )
    .await
    .expect("pairs")
}

async fn side(p: &TestPeer, paired: RelayPaired, id: u32) -> Side {
    let stream = p.rt.create_stream(id).await.expect("stream");
    let (relay_addr, remote_id) = (paired.relay_addr, paired.remote_id);
    stream
        .connect(&paired.socket, remote_id, relay_addr)
        .await
        .expect("connect");
    let second = p.rt.create_socket().await.expect("socket");
    second
        .bind("127.0.0.1:0".parse().unwrap())
        .await
        .expect("bind");
    let second_addr = second.local_addr().await.expect("addr");
    Side {
        control: stream.control(),
        stream,
        session: Some(paired.session),
        second,
        second_addr,
        own_id: id,
    }
}

/// Two connected sides; the relay learns each address from the first packet, so both say hello.
async fn sides(n: &TestNet) -> (Side, Side) {
    let token = rand::random::<[u8; 32]>();
    let (ida, idb) = (0x1001, 0x2002);
    let (pa, pb) = tokio::join!(
        pair(n, &n.a, token, true, ida),
        pair(n, &n.b, token, false, idb)
    );
    let (mut a, mut b) = (side(&n.a, pa, ida).await, side(&n.b, pb, idb).await);
    let (wa, wb) = tokio::join!(a.stream.write(b"hello"), b.stream.write(b"hello"));
    wa.expect("hello a");
    wb.expect("hello b");
    assert_eq!(
        a.stream.read().await.expect("read").expect("open"),
        b"hello"
    );
    assert_eq!(
        b.stream.read().await.expect("read").expect("open"),
        b"hello"
    );
    (a, b)
}

fn bytes(seed: u8) -> Vec<u8> {
    (0..CHUNK).map(|i| (i % 251) as u8 ^ seed).collect()
}

/// Send `data` from `from` to `to`; the hash of what arrived.
async fn transfer(from: &Side, to: &mut Side, data: &[u8]) -> Vec<u8> {
    let read = async {
        let mut h = Sha256::new();
        let mut n = 0;
        while n < data.len() {
            let chunk = to.stream.read().await.expect("read").expect("open");
            n += chunk.len();
            h.update(&chunk);
        }
        h.finalize().to_vec()
    };
    let (w, got) = tokio::join!(from.stream.write(data), read);
    w.expect("write");
    got
}

async fn becomes_direct(mut rx: watch::Receiver<RelayPath>) -> bool {
    rx.wait_for(|p| *p == RelayPath::Direct).await.is_ok()
}

async fn wait_for(limit: Duration, mut ok: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while !ok() {
        if start.elapsed() > limit {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    true
}

#[tokio::test]
async fn both_peers_upgrade_without_loss() {
    let n = net().await;
    let (mut a, mut b) = sides(&n).await;
    let first = bytes(1);
    assert_eq!(
        transfer(&a, &mut b, &first).await,
        Sha256::digest(&first).to_vec()
    );

    let punched = |s: &Side, other: &Side| DirectSource::Punched {
        socket: s.second.clone(),
        addr: other.second_addr,
        remote_id: other.own_id,
    };
    let la = Upgrade::spawn(
        a.control.clone(),
        a.session.take().unwrap(),
        punched(&a, &b),
    );
    let lb = Upgrade::spawn(
        b.control.clone(),
        b.session.take().unwrap(),
        punched(&b, &a),
    );
    let second = bytes(2);
    let (got, da, db) = tokio::join!(
        transfer(&a, &mut b, &second),
        tokio::time::timeout(Duration::from_secs(10), becomes_direct(la.path())),
        tokio::time::timeout(Duration::from_secs(10), becomes_direct(lb.path())),
    );
    assert_eq!(got, Sha256::digest(&second).to_vec());
    assert!(matches!(da, Ok(true)), "a not direct in 10 s");
    assert!(matches!(db, Ok(true)), "b not direct in 10 s");
    assert_eq!(a.control.remote(), Some((b.second_addr, b.own_id)));
    assert_eq!(b.control.remote(), Some((a.second_addr, a.own_id)));

    let idle = wait_for(Duration::from_secs(5), || {
        n.relay.stats().pairings_active == 0
    })
    .await;
    assert!(
        idle,
        "pairings_active = {}",
        n.relay.stats().pairings_active
    );
    let closed = wait_for(Duration::from_secs(5), || {
        n.relay.stats().streams_closed >= 2
    })
    .await;
    assert!(
        closed,
        "streams_closed = {}",
        n.relay.stats().streams_closed
    );
    a.stream.destroy().await.expect("destroy a");
    b.stream.destroy().await.expect("destroy b");
}

/// A lone stream (nobody answers) with a fake session that signals on close.
async fn lone() -> (UdxRuntime, UdxStream, Side2) {
    let rt = UdxRuntime::new().expect("runtime");
    let socket = rt.create_socket().await.expect("socket");
    socket
        .bind("127.0.0.1:0".parse().unwrap())
        .await
        .expect("bind");
    let stream = rt.create_stream(1).await.expect("stream");
    stream
        .connect(&socket, 100, "127.0.0.1:9".parse().unwrap())
        .await
        .expect("connect");
    (rt, stream, Side2(socket))
}
struct Side2(UdxSocket);

async fn closes_within(rx: tokio::sync::oneshot::Receiver<()>, limit: Duration) -> bool {
    matches!(tokio::time::timeout(limit, rx).await, Ok(Ok(())))
}

#[tokio::test]
async fn stream_close_closes_session() {
    let (_rt, stream, Side2(socket)) = lone().await;
    let (session, mut closed) = RelaySession::probe();
    let direct = DirectSource::Punched {
        socket,
        addr: "127.0.0.1:9".parse().unwrap(),
        remote_id: 100,
    };
    let link = Upgrade::spawn(stream.control(), session, direct);
    let early = tokio::time::timeout(Duration::from_millis(300), &mut closed).await;
    assert!(early.is_err(), "session closed while the stream is open");
    stream.destroy().await.expect("destroy");
    assert!(
        closes_within(closed, Duration::from_secs(2)).await,
        "session must close with the stream"
    );
    assert_eq!(*link.path().borrow(), RelayPath::Relayed);
}

#[tokio::test]
async fn change_remote_error_keeps_relay() {
    let (_rt, stream, Side2(socket)) = lone().await;
    let (session, mut closed) = RelaySession::probe();
    let direct = DirectSource::Punched {
        socket,
        addr: "127.0.0.1:0".parse().unwrap(),
        remote_id: 100,
    };
    let link = Upgrade::spawn(stream.control(), session, direct);
    let early = tokio::time::timeout(Duration::from_millis(500), &mut closed).await;
    assert!(early.is_err(), "a failed change must keep the relay");
    assert_eq!(*link.path().borrow(), RelayPath::Relayed);
    stream.destroy().await.expect("destroy");
    assert!(
        closes_within(closed, Duration::from_secs(2)).await,
        "session must close with the stream"
    );
}

#[tokio::test]
async fn dropped_link_keeps_session_until_stream_closed() {
    let n = net().await;
    let (mut a, mut b) = sides(&n).await;
    let pending = AbortOnDrop(tokio::spawn(std::future::pending()));
    let link = Upgrade::spawn_after_punch(a.control.clone(), a.session.take().unwrap(), pending);
    drop(link);
    let data = bytes(3);
    assert_eq!(
        transfer(&a, &mut b, &data).await,
        Sha256::digest(&data).to_vec()
    );
    assert_eq!(n.relay.stats().pairings_active, 1);
    let start = Instant::now();
    a.stream.destroy().await.expect("destroy");
    let idle = wait_for(Duration::from_secs(2), || {
        n.relay.stats().pairings_active == 0
    })
    .await;
    assert!(
        idle,
        "pairings_active = {} after {:?}",
        n.relay.stats().pairings_active,
        start.elapsed()
    );
    drop(b.session.take());
}

#[test]
fn relay_replying_from_another_local_address_is_not_a_direct_path() {
    let relay: SocketAddr = "192.168.1.5:4000".parse().unwrap();
    let loopback: SocketAddr = "127.0.0.1:4000".parse().unwrap();
    let other_port: SocketAddr = "127.0.0.1:4001".parse().unwrap();
    let remote: SocketAddr = "203.0.113.9:4000".parse().unwrap();
    assert!(is_relay_alias(relay, loopback));
    assert!(is_relay_alias(relay, relay));
    assert!(!is_relay_alias(relay, other_port));
    assert!(!is_relay_alias(relay, remote));
}
