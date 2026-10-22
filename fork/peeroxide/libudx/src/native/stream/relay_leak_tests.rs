//! A bridged pair (`relay_to` both ways) must be freed once its handles are closed or dropped.

use super::test_support::*;
use super::*;
use crate::UdxRuntime;

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
