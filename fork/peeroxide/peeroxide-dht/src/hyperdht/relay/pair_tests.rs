//! Two peers pairing through a real blind relay on loopback.
// JS: hyperdht/lib/connect.js:793-838

use std::time::{Duration, Instant};

use super::super::test_support::{TestNet, TestPeer, net};
use super::{RelayPaired, RelaySession, pair_via_relay};
use crate::hyperdht::HyperDhtError;

const KEEP_ALIVE: Duration = Duration::from_secs(5);

async fn pair(
    n: &TestNet,
    p: &TestPeer,
    token: [u8; 32],
    initiator: bool,
    id: u32,
) -> Result<RelayPaired, HyperDhtError> {
    pair_via_relay(
        &p.handle,
        &p.key_pair,
        &p.rt,
        n.relay_pk,
        token,
        initiator,
        id,
        KEEP_ALIVE,
    )
    .await
}

async fn pair_both(n: &TestNet) -> (RelayPaired, RelayPaired, u32, u32) {
    let token = rand::random::<[u8; 32]>();
    let (ida, idb) = (0x1001, 0x2002);
    let (a, b) = tokio::join!(
        pair(n, &n.a, token, true, ida),
        pair(n, &n.b, token, false, idb)
    );
    (a.expect("a pairs"), b.expect("b pairs"), ida, idb)
}

async fn until_zero_active(n: &TestNet, start: Instant, limit: Duration) -> bool {
    while n.relay.stats().pairings_active != 0 {
        if start.elapsed() > limit {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    true
}

#[tokio::test]
async fn two_peers_pair_and_echo() {
    let n = net().await;
    let (pa, pb, ida, idb) = pair_both(&n).await;
    let sa = n.a.rt.create_stream(ida).await.expect("stream a");
    let mut sb = n.b.rt.create_stream(idb).await.expect("stream b");
    let (ca, cb) = tokio::join!(
        sa.connect(&pa.socket, pa.remote_id, pa.relay_addr),
        sb.connect(&pb.socket, pb.remote_id, pb.relay_addr)
    );
    ca.expect("connect a");
    cb.expect("connect b");
    // The relay learns a peer's address from its first packet, so both speak first (like a handshake).
    let mut sa = sa;
    let (wa, wb) = tokio::join!(sa.write(b"hello"), sb.write(b"hello"));
    wa.expect("hello a");
    wb.expect("hello b");
    assert_eq!(sb.read().await.expect("hello").expect("open"), b"hello");
    let echo = tokio::spawn(async move {
        while let Ok(Some(chunk)) = sb.read().await {
            sb.write(&chunk).await.expect("echo write");
        }
    });
    let data: Vec<u8> = (0..64 * 1024u32).map(|i| (i % 251) as u8).collect();
    let mut back = Vec::new();
    let run = async {
        assert_eq!(sa.read().await.expect("hello").expect("open"), b"hello");
        sa.write(&data).await.expect("write");
        while back.len() < data.len() {
            back.extend(sa.read().await.expect("read").expect("open"));
        }
    };
    tokio::time::timeout(Duration::from_secs(20), run)
        .await
        .expect("echo timed out");
    assert_eq!(back, data);
    let stats = n.relay.stats();
    assert_eq!(stats.pairings_matched, 1);
    assert_eq!(stats.streams_opened, 2);
    echo.abort();
}

#[tokio::test]
async fn lone_pair_aborts_after_15s() {
    let n = net().await;
    let start = Instant::now();
    let r = pair(&n, &n.a, rand::random(), true, 0x1001).await;
    let took = start.elapsed();
    assert!(
        matches!(r, Err(HyperDhtError::RelayAborted)),
        "got an unexpected outcome"
    );
    assert!(
        took >= Duration::from_secs(15) && took < Duration::from_secs(18),
        "aborted after {took:?}"
    );
}

#[tokio::test]
async fn session_close_unpairs() {
    let n = net().await;
    let (pa, _pb, _, _) = pair_both(&n).await;
    assert!(n.relay.stats().pairings_active > 0);
    let start = Instant::now();
    pa.session.close().await;
    assert!(
        until_zero_active(&n, start, Duration::from_secs(2)).await,
        "pairings_active = {}",
        n.relay.stats().pairings_active
    );
}

#[tokio::test]
async fn probe_signals_on_close_and_drop() {
    let (s, rx) = RelaySession::probe();
    s.close().await;
    assert!(rx.await.is_ok(), "close must signal");
    let (s, rx) = RelaySession::probe();
    drop(s);
    assert!(rx.await.is_ok(), "drop must signal");
}
