// JS: hyperdht/lib/server.js `firewall(remotePublicKey, …)`: `true` is a rejection; a foreign key is rejected, the own one accepted.
// Related: setup.rs, wait.rs (`connect_refused`, `assert_empty`). Keys: 0x41 server, 0x42 own, 0x43 foreign.
use super::keys::key_pair;
use super::setup::{connect_pair, listen_on, node};
use super::wait::{assert_empty, connect_refused};
use std::sync::Arc;
use townjump_dht::{DhtFactory, Firewall};

/// Firewall as in JS: `true` = reject; lets in only `allowed`.
fn only(allowed: [u8; 32]) -> Firewall {
    Arc::new(move |pk| *pk != allowed)
}

/// A foreign key gets an error and nothing reaches the incoming queue; the own key passes.
pub async fn firewall_foreign_key<F: DhtFactory>(factory: &F) {
    let own = key_pair(0x42).public_key;
    let mut server = listen_on(factory, key_pair(0x41), Some(only(own))).await;
    let stranger = node(factory, 0x43).await;
    connect_refused(&stranger, server.pk, "firewall: foreign key").await;
    assert_empty(&mut server.incoming, "firewall: foreign key").await;
    let friend = node(factory, 0x42).await;
    connect_pair(&friend, own, &mut server, "firewall: own key").await;
}
