//! `StreamControl::abort`: both sides see the end, `closed()` resolves, a repeated call is harmless.

use std::time::Duration;

use tokio::io::AsyncReadExt;

use super::test_support::bound_socket;
use crate::native::runtime::UdxRuntime;

#[tokio::test]
async fn abort_closes_the_stream() {
    let runtime = UdxRuntime::new().expect("runtime");
    let (sock_a, addr_a) = bound_socket(&runtime).await;
    let (sock_b, addr_b) = bound_socket(&runtime).await;
    let a = runtime.create_stream(1).await.expect("stream a");
    let b = runtime.create_stream(2).await.expect("stream b");
    a.connect(&sock_a, 2, addr_b).await.expect("connect a");
    b.connect(&sock_b, 1, addr_a).await.expect("connect b");
    let (control_a, control_b) = (a.control(), b.control());
    let (mut own, mut peer) = (a.into_async_stream(), b.into_async_stream());

    control_a.abort();

    let one_second = Duration::from_secs(1);
    let mut buf = [0u8; 8];
    let own_read = tokio::time::timeout(one_second, own.read(&mut buf)).await;
    assert!(matches!(own_read, Ok(Err(_))), "own read: {own_read:?}");
    let peer_read = tokio::time::timeout(one_second, peer.read(&mut buf)).await;
    assert!(matches!(peer_read, Ok(Err(_))), "peer read: {peer_read:?}");
    tokio::time::timeout(one_second, control_a.closed())
        .await
        .expect("own closed() resolves");
    tokio::time::timeout(one_second, control_b.closed())
        .await
        .expect("peer closed() resolves");

    control_a.abort();
    control_b.abort();
}
