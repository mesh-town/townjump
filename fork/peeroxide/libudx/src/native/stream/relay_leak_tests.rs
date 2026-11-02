//! A bridged pair (`relay_to` both ways) must be freed once its handles are closed or dropped.

use super::test_support::*;
use super::*;
use crate::{UdxRuntime, UdxSocket};

type Weak2 = (
    std::sync::Weak<Mutex<StreamInner>>,
    std::sync::Weak<Mutex<StreamInner>>,
);

struct Bridge {
    _runtime: UdxRuntime,
    a: UdxStream,
    b: UdxStream,
    raw_b: tokio::net::UdpSocket,
    addr_a: SocketAddr,
    raw_a: tokio::net::UdpSocket,
    sock_a: UdxSocket,
}

async fn bridged() -> Bridge {
    let runtime = UdxRuntime::new().expect("runtime");
    let (sa, addr_a) = bound_socket(&runtime).await;
    let (sb, _addr_b) = bound_socket(&runtime).await;
    let (raw_a, raddr_a) = raw_peer().await;
    let (raw_b, raddr_b) = raw_peer().await;
    let a = runtime.create_stream(LOCAL_ID).await.expect("a");
    let b = runtime.create_stream(2).await.expect("b");
    a.relay_to(&b).expect("a->b");
    b.relay_to(&a).expect("b->a");
    a.connect(&sa, REMOTE_ID, raddr_a).await.expect("connect a");
    b.connect(&sb, REMOTE_ID, raddr_b).await.expect("connect b");
    Bridge {
        _runtime: runtime,
        a,
        b,
        raw_b,
        addr_a,
        raw_a,
        sock_a: sa,
    }
}

fn weaks(a: &UdxStream, b: &UdxStream) -> Weak2 {
    (Arc::downgrade(&a.inner), Arc::downgrade(&b.inner))
}

async fn freed(w: &std::sync::Weak<Mutex<StreamInner>>) -> bool {
    for _ in 0..50 {
        if w.upgrade().is_none() {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    false
}

#[tokio::test]
async fn bridged_pair_is_freed_after_destroy() {
    let Bridge { a, b, .. } = bridged().await;
    let (wa, wb) = weaks(&a, &b);
    a.destroy().await.expect("destroy a");
    b.destroy().await.expect("destroy b");
    assert!(freed(&wa).await, "stream a leaked (relay_target cycle)");
    assert!(freed(&wb).await, "stream b leaked (relay_target cycle)");
}

#[tokio::test]
async fn bridged_pair_is_freed_after_drop() {
    let Bridge { a, b, .. } = bridged().await;
    let (wa, wb) = weaks(&a, &b);
    drop(a);
    drop(b);
    assert!(freed(&wa).await, "stream a leaked (relay_target cycle)");
    assert!(freed(&wb).await, "stream b leaked (relay_target cycle)");
}

#[tokio::test]
async fn packet_for_live_side_is_dropped_when_peer_is_gone() {
    let br = bridged().await;
    let (wa, wb) = weaks(&br.a, &br.b);
    let Bridge {
        a,
        b,
        raw_b,
        raw_a,
        addr_a,
        ..
    } = br;
    drop(b);
    assert!(freed(&wb).await, "closed stream b leaked while a is alive");
    raw_a
        .send_to(&peer_packet(FLAG_DATA, 0, 0, b"x"), addr_a)
        .await
        .expect("send");
    assert!(
        recv_raw(&raw_b).await.is_none(),
        "packet must not be forwarded to a freed target"
    );
    assert!(wa.upgrade().is_some(), "live stream a must stay alive");
    drop(a);
    assert!(freed(&wa).await, "stream a leaked after drop");
}

/// A relayed stream moves packets outside the engine, so its `next_seq` and `next_remote_seq` mean nothing:
/// closing it sets no tombstone (`relay` refusal in `linger.rs`); the entry goes at once and the peer's late DATA is met with silence.
#[tokio::test]
async fn closed_relayed_stream_sets_no_tombstone() {
    let Bridge {
        a,
        b: _b,
        raw_a,
        addr_a,
        sock_a,
        ..
    } = bridged().await;
    // the adapter's drop is the path that buries a plain stream (a bare handle's drop leaves no tombstone)
    drop(a.into_async_stream());
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let len = sock_a
        .streams_ref()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .len();
    assert_eq!(
        len, 0,
        "entry of the closed relayed stream is still in the table"
    );
    // packets the close itself sent (if any) are not an answer: drain them first
    while recv_raw(&raw_a).await.is_some() {}
    raw_a
        .send_to(&peer_packet(FLAG_DATA, 0, 0, b"x"), addr_a)
        .await
        .expect("send");
    // 2·rto with the initial rto of 1 s: two one-second waits
    for _ in 0..2 {
        assert!(
            recv_raw(&raw_a).await.is_none(),
            "closed relayed stream answered the peer (ACK or DESTROY)"
        );
    }
}
