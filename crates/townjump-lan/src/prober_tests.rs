// Tests of `drain`, the last look at the socket buffer at the probe deadline.
use super::*;
use crate::codec::{Reply, encode_reply};
use std::time::Duration;

#[tokio::test]
async fn drain_takes_a_valid_reply_behind_a_foreign_one() {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("bind");
    let to = socket.local_addr().expect("addr");
    let (nonce, target) = ([1u8; 8], [2u8; 32]);
    let foreign = encode_reply(&Reply {
        nonce: [9; 8],
        target,
        port: 1111,
    });
    let valid = encode_reply(&Reply {
        nonce,
        target,
        port: 4242,
    });
    let sender = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("sender");
    sender.send_to(&foreign, to).expect("send foreign");
    sender.send_to(&valid, to).expect("send valid");
    socket.readable().await.expect("readable");
    // Loopback delivers both before this pause ends; the pause only orders the test, not the code under test.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let found = drain(&socket, &nonce, &target);
    assert_eq!(found, Some(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 4242)));
}

#[tokio::test]
async fn drain_of_an_empty_socket_is_none() {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("bind");
    assert_eq!(drain(&socket, &[1; 8], &[2; 32]), None);
}
