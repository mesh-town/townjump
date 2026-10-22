//! connect on a key nobody serves: `PeerNotFound`; on a key whose holder does not let us in:
//! `NoRelayNodes` (the adapter shows it as `PEER_CONNECTION_FAILED`). JS reference:
//! `hyperdht/lib/connect.js:findAndConnect`, run on a local testnet (`PEER_NOT_FOUND` for
//! no holder, `PEER_CONNECTION_FAILED` for a holder that refuses or is gone). 127.0.0.1 only.
mod common;

use common::{Node, cluster, serve_key, spawn_node, within};
use peeroxide_dht::hyperdht::{HyperDhtError, KeyPair};

async fn connect_error(client: &Node, server_pk: [u8; 32]) -> HyperDhtError {
    let me = KeyPair::from_seed([0x61; 32]);
    match client.handle.connect(&me, server_pk, &client.rt).await {
        Err(e) => e,
        Ok(_) => panic!("connect must fail"),
    }
}

#[tokio::test]
async fn connect_to_a_key_nobody_serves_is_peer_not_found() {
    within(async {
        let (_nodes, bootstrap) = cluster(2).await;
        let client = spawn_node(bootstrap, true).await;
        let err = connect_error(&client, KeyPair::from_seed([0x62; 32]).public_key).await;
        assert!(matches!(err, HyperDhtError::PeerNotFound), "{err:?}");
    })
    .await;
}

#[tokio::test]
async fn connect_to_a_holder_that_refuses_is_not_peer_not_found() {
    within(async {
        let (_nodes, bootstrap) = cluster(2).await;
        let key = KeyPair::from_seed([0x63; 32]);
        let _served = serve_key(&bootstrap, &key, true).await;
        let client = spawn_node(bootstrap, true).await;
        let err = connect_error(&client, key.public_key).await;
        assert!(matches!(err, HyperDhtError::NoRelayNodes), "{err:?}");
    })
    .await;
}
