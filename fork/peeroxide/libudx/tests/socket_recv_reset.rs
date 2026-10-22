#![cfg(windows)]
// Windows: ICMP "port unreachable" on a send to a closed port must not mute the socket's receive
// (WSAECONNRESET 10054 on `recv_from`). The tests call `recv_start` once and open no threads.
// JS: libuv src/win/udp.c (WSAECONNRESET — skip)
#![deny(clippy::all)]

mod common;

use std::net::SocketAddr;
use std::time::Duration;

use common::{DEFAULT_TIMEOUT, create_bound_socket, create_runtime, with_timeout};

async fn datagram_survives_hits(hits: usize) {
    let _ = tracing_subscriber::fmt::try_init();

    with_timeout(DEFAULT_TIMEOUT, async {
        let runtime = create_runtime();
        let (socket_a, addr_a) = create_bound_socket(&runtime).await;
        let (socket_b, _addr_b) = create_bound_socket(&runtime).await;

        let mut rx = socket_a.recv_start().expect("recv_start failed");

        // We take the port after A and B so that it does not go to them, and close it.
        let dead: SocketAddr = {
            let probe = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind probe failed");
            probe.local_addr().expect("local_addr failed")
        };

        for _ in 0..hits {
            socket_a
                .send_to(b"ping", dead)
                .expect("send_to dead failed");
        }
        tokio::time::sleep(Duration::from_millis(300)).await;

        let payload = b"after reset";
        socket_b.send_to(payload, addr_a).expect("send_to a failed");

        let datagram = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("recv timed out")
            .expect("channel closed");
        assert_eq!(datagram.data, payload);

        socket_a.close().await.expect("close a failed");
        socket_b.close().await.expect("close b failed");
    })
    .await;
}

#[tokio::test]
async fn datagram_arrives_after_send_to_closed_port() {
    datagram_survives_hits(1).await;
}

#[tokio::test]
async fn datagram_arrives_after_burst_to_closed_port() {
    datagram_survives_hits(300).await;
}
