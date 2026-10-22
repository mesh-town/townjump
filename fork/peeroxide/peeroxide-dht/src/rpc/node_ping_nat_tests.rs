// JS: dht-rpc/index.js
//! PING_NAT responder: the reply goes to the requester's host at the port in the value.
//! Fixture: `node_test_support`.

use super::super::consts::CMD_PING_NAT;
use super::super::node_test_support::*;
use crate::io::{IncomingRequest, ReplyContext, SocketKind};
use crate::messages::Ipv4Peer;
use std::time::Duration;
use tokio::net::UdpSocket;

async fn listener() -> UdpSocket {
    UdpSocket::bind("127.0.0.1:0").await.expect("bind")
}

fn ping_nat(from_port: u16, value: Option<Vec<u8>>) -> IncomingRequest {
    IncomingRequest {
        tid: 77,
        from: Ipv4Peer {
            host: "127.0.0.1".into(),
            port: from_port,
        },
        to: Ipv4Peer::default(),
        id: None,
        token: None,
        internal: true,
        command: CMD_PING_NAT,
        target: None,
        value,
        reply_ctx: ReplyContext {
            socket_kind: SocketKind::Server,
            via: None,
        },
    }
}

async fn got(sock: &UdpSocket, wait: Duration) -> bool {
    let mut buf = [0u8; 2048];
    matches!(
        tokio::time::timeout(wait, sock.recv_from(&mut buf)).await,
        Ok(Ok(_))
    )
}

const WAIT: Duration = Duration::from_secs(2);
const QUIET: Duration = Duration::from_millis(300);

#[tokio::test]
async fn replies_to_the_port_in_the_value() {
    // given: A is the request's source port, B the port named in the value
    let mut f = setup(Search::Absent).await;
    let (a, b) = (listener().await, listener().await);
    let port_b = b.local_addr().expect("addr").port();
    let value = port_b.to_le_bytes().to_vec();
    let req = ping_nat(a.local_addr().expect("addr").port(), Some(value));
    // when: the node handles it
    f.node.handle_incoming_request(req);
    // then: B receives the reply, A does not
    assert!(got(&b, WAIT).await, "no reply on the value port");
    assert!(!got(&a, QUIET).await, "reply went to the source port");
}

#[tokio::test]
async fn zero_port_gets_no_reply() {
    let mut f = setup(Search::Absent).await;
    let a = listener().await;
    let port_a = a.local_addr().expect("addr").port();
    f.node
        .handle_incoming_request(ping_nat(port_a, Some(vec![0, 0])));
    assert!(!got(&a, QUIET).await);
}

#[tokio::test]
async fn short_value_gets_no_reply() {
    let mut f = setup(Search::Absent).await;
    let a = listener().await;
    let port_a = a.local_addr().expect("addr").port();
    f.node
        .handle_incoming_request(ping_nat(port_a, Some(vec![7])));
    assert!(!got(&a, QUIET).await);
}
