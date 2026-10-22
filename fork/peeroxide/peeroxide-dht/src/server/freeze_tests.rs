//! `HyperDhtHandle::suspend`/`resume` with a listening `Server` on a local cluster:
//! a suspended server withdraws its announcement, a resumed one accepts again.

use std::time::Duration;

use libudx::UdxRuntime;

use crate::crypto::hash;
use crate::hyperdht::{HyperDhtError, PeerConnection};
use crate::rpc::DhtError;
use crate::server::ServerOpts;
use crate::server::test_net::{Node, Served, cluster, key, node, serve, within};

async fn connect(client: &Node, server_pk: [u8; 32]) -> Result<PeerConnection, HyperDhtError> {
    client.handle.connect(&key(2), server_pk, &client.rt).await
}

/// Whether the DHT still holds the server's announcement (a FIND_PEER through `client`).
async fn announced(client: &Node, server_pk: [u8; 32]) -> bool {
    let found = client.handle.find_peer(hash(&server_pk)).await;
    found.expect("find_peer").is_some()
}

/// The next connection the server accepted (10 s at most).
async fn accepted(server: &mut Served) -> PeerConnection {
    let next = tokio::time::timeout(Duration::from_secs(10), server.incoming.recv()).await;
    next.expect("no incoming connection in 10 s")
        .expect("incoming channel closed")
}

#[tokio::test]
async fn suspended_server_is_unreachable_and_resume_serves_again() {
    within(async {
        let cl = cluster(3).await;
        let own = key(1);
        let mut server = serve(node(&cl, false).await, &own, ServerOpts::default()).await;
        let client = node(&cl, false).await;
        let first = connect(&client, own.public_key)
            .await
            .expect("before suspend");
        accepted(&mut server).await;
        drop(first);
        assert!(announced(&client, own.public_key).await, "announced");

        server.handle.suspend().await.expect("suspend");
        assert!(server.handle.is_suspended());
        let rt = UdxRuntime::new().expect("runtime");
        let own_connect = server.handle.connect(&key(3), key(9).public_key, &rt).await;
        assert!(
            matches!(own_connect, Err(HyperDhtError::Dht(DhtError::Suspended))),
            "a suspended node connects nowhere: {:?}",
            own_connect.err()
        );
        assert!(!announced(&client, own.public_key).await, "unannounced");

        server.handle.resume().await.expect("resume");
        assert!(announced(&client, own.public_key).await, "announced again");
        let second = connect(&client, own.public_key)
            .await
            .expect("after resume");
        accepted(&mut server).await;
        drop(second);
    })
    .await;
}

#[tokio::test]
async fn suspend_without_server_does_not_wait() {
    within(async {
        let cl = cluster(2).await;
        let lone = node(&cl, false).await;
        let quick = Duration::from_secs(5);
        tokio::time::timeout(quick, lone.handle.suspend())
            .await
            .expect("no server to wait for")
            .expect("suspend");
        tokio::time::timeout(quick, lone.handle.resume())
            .await
            .expect("resume is quick")
            .expect("resume");
    })
    .await;
}
