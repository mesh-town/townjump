//! `Server::listen_unannounced`: a server that answers handshakes at its node without publishing a
//! record (the caller owns the announcement), and that refuses a second or a late start like `listen`.
// JS: hyperdht/lib/server.js

use std::time::Duration;

use crate::crypto::hash;
use crate::hyperdht::{HyperDhtError, PeerConnection};
use crate::messages::Ipv4Peer;
use crate::server::ServerOpts;
use crate::server::test_net::{Cluster, Node, Served, cluster, key, node, start, within};

mod listen_unannounced {
    use super::*;

    #[tokio::test]
    async fn listen_unannounced_answers_without_a_record() {
        within(async {
            // given: a server on a cluster node, started without an announce
            let cl = cluster(3).await;
            let mut s = setup(&cl).await;
            let client = node(&cl, false).await;

            // when: the client looks the server up and dials the node's own address
            let found = no_record_for_2s(&client, key(1).public_key).await;
            let conn = dial_node(&client, &s).await;

            // then: the DHT holds no record, yet the server accepts the connection
            assert!(found, "no record may appear in the DHT");
            let conn = conn.expect("connects by the node's address");
            assert_eq!(conn.remote_public_key, key(1).public_key);
            assert_eq!(
                accepted(&mut s).await.remote_public_key,
                key(2).public_key,
                "the server sees the client's key"
            );
        })
        .await;
    }

    #[tokio::test]
    async fn second_listen_is_refused() {
        within(async {
            // given: a server already listening unannounced
            let cl = cluster(3).await;
            let s = setup(&cl).await;

            // when: it is started again
            let again = s.server.listen_unannounced(key(1));

            // then: the start is refused
            assert!(
                matches!(again, Err(HyperDhtError::AlreadyListening)),
                "got {again:?}"
            );
        })
        .await;
    }

    #[tokio::test]
    async fn announcing_listen_after_it_is_refused() {
        within(async {
            let cl = cluster(3).await;
            let s = setup(&cl).await;

            let again = s.server.listen(key(1)).await;

            assert!(
                matches!(again, Err(HyperDhtError::AlreadyListening)),
                "got {again:?}"
            );
        })
        .await;
    }

    #[tokio::test]
    async fn start_after_close_is_refused() {
        within(async {
            let cl = cluster(3).await;
            let s = start(node(&cl, false).await, ServerOpts::default());
            s.server.close().await.expect("close");

            let late = s.server.listen_unannounced(key(1));

            assert!(
                matches!(late, Err(HyperDhtError::ServerClosed)),
                "got {late:?}"
            );
        })
        .await;
    }
}

/// A server on a fresh cluster node, listening on `key(1)` without an announce.
async fn setup(cl: &Cluster) -> Served {
    let s = start(node(cl, false).await, ServerOpts::default());
    s.server
        .listen_unannounced(key(1))
        .expect("listen unannounced");
    s
}

/// `true` when `find_peer` finds no record of `pk` at any of four looks over 2 s.
async fn no_record_for_2s(client: &Node, pk: [u8; 32]) -> bool {
    for _ in 0..4 {
        let found = client.handle.find_peer(hash(&pk)).await;
        if !matches!(found, Ok(None)) {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    true
}

/// The client (key 2) connects to the server's key naming only the server's node address.
async fn dial_node(client: &Node, s: &Served) -> Result<PeerConnection, HyperDhtError> {
    let port = s.handle.local_port().await.expect("server node port");
    let address = Ipv4Peer {
        host: "127.0.0.1".into(),
        port,
    };
    client
        .handle
        .connect_with_nodes(&key(2), key(1).public_key, &[address], &client.rt)
        .await
}

/// The next connection the server accepted (10 s at most).
async fn accepted(s: &mut Served) -> PeerConnection {
    let next = tokio::time::timeout(Duration::from_secs(10), s.incoming.recv()).await;
    next.expect("no incoming connection in 10 s")
        .expect("incoming channel closed")
}
