// JS: dht-rpc/lib/query.js `_endAfterCommit` rejects with "Too few nodes responded" when no node replied.
//! A put on a node with no bootstrap and no seeds has nobody to ask: it fails instead of reporting success.

mod common;

use common::{spawn_config, within};
use peeroxide_dht::hyperdht::{HyperDhtError, KeyPair};
use peeroxide_dht::rpc::DhtConfig;

fn lone_config() -> DhtConfig {
    let mut dht = DhtConfig::default();
    dht.host = "127.0.0.1".to_string();
    dht.firewalled = false;
    dht.bootstrap = vec![];
    dht.nodes = vec![];
    dht
}

#[tokio::test]
async fn mutable_put_without_responders_fails() {
    within(async {
        let node = spawn_config(lone_config()).await;
        let key = KeyPair::from_seed([0x5a; 32]);
        let outcome = node.handle.mutable_put(&key, b"nobody", 1).await;
        assert!(
            matches!(outcome, Err(HyperDhtError::NoNodesResponded)),
            "got {outcome:?}"
        );
    })
    .await;
}

#[tokio::test]
async fn immutable_put_without_responders_fails() {
    within(async {
        let node = spawn_config(lone_config()).await;
        let outcome = node.handle.immutable_put(b"nobody").await;
        assert!(
            matches!(outcome, Err(HyperDhtError::NoNodesResponded)),
            "got {outcome:?}"
        );
    })
    .await;
}
