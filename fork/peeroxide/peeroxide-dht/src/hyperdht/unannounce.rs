// JS: hyperdht/lib/announcer.js (`_unannounceAll`, `_unannounce`)
//! A server withdraws its record from its relays: for each relay a FIND_PEER (to get the token),
//! then an UNANNOUNCE, all relays in parallel under one deadline. No LOOKUP walk.

use std::future::Future;
use std::time::Duration;

use tokio::task::JoinSet;
use tokio::time::Instant;

use crate::crypto::{NS_UNANNOUNCE, ann_signable, sign_detached};
use crate::hyperdht_messages::{
    AnnounceMessage, FIND_PEER, HyperPeer, UNANNOUNCE, encode_announce_to_bytes,
    encode_hyper_peer_to_bytes,
};
use crate::messages::Ipv4Peer;
use crate::rpc::UserRequestParams;

use super::handle::HyperDhtHandle;
use super::keypair::KeyPair;
use super::types::HyperDhtError;

/// The whole withdrawal (all relays at once) gives up after this long.
pub(crate) const UNANNOUNCE_DEADLINE: Duration = Duration::from_millis(1500);

impl HyperDhtHandle {
    /// Withdraws the record `target` from every relay in parallel, at most `UNANNOUNCE_DEADLINE`;
    /// how many relays confirmed. No relays: nothing is sent.
    pub(crate) async fn unannounce_relays(
        &self,
        target: [u8; 32],
        key_pair: &KeyPair,
        relays: &[Ipv4Peer],
    ) -> usize {
        let jobs = relays
            .iter()
            .map(|relay| unannounce_at(self.clone(), target, key_pair.clone(), relay.clone()))
            .collect();
        settle_within(UNANNOUNCE_DEADLINE, jobs).await
    }
}

/// FIND_PEER then UNANNOUNCE at one relay; `true` when the relay accepted the withdrawal.
async fn unannounce_at(
    dht: HyperDhtHandle,
    target: [u8; 32],
    key_pair: KeyPair,
    relay: Ipv4Peer,
) -> bool {
    let Some((token, node_id)) = find_token(&dht, target, &relay).await else {
        return false;
    };
    let Ok(value) = unannounce_value(&target, &token, &node_id, &key_pair) else {
        return false;
    };
    let params = request(UNANNOUNCE, target, Some(token), Some(value));
    let reply = dht.dht().request(params, &relay.host, relay.port).await;
    matches!(reply, Ok(r) if r.error == 0)
}

/// The relay's token and node id when it holds a record under `target` (JS announcer.js:225).
async fn find_token(
    dht: &HyperDhtHandle,
    target: [u8; 32],
    relay: &Ipv4Peer,
) -> Option<([u8; 32], [u8; 32])> {
    let params = request(FIND_PEER, target, None, None);
    let reply = dht
        .dht()
        .request(params, &relay.host, relay.port)
        .await
        .ok()?;
    reply.value?;
    Some((reply.token?, reply.id?))
}

fn request(
    command: u64,
    target: [u8; 32],
    token: Option<[u8; 32]>,
    value: Option<Vec<u8>>,
) -> UserRequestParams {
    UserRequestParams {
        token,
        command,
        target: Some(target),
        value,
        timeout_ms: None,
        retries: None,
    }
}

/// The signed UNANNOUNCE body (a peer record without relay addresses).
pub(super) fn unannounce_value(
    target: &[u8; 32],
    token: &[u8; 32],
    node_id: &[u8; 32],
    key_pair: &KeyPair,
) -> Result<Vec<u8>, HyperDhtError> {
    let peer = HyperPeer {
        public_key: key_pair.public_key,
        relay_addresses: vec![],
    };
    let peer_encoded = encode_hyper_peer_to_bytes(&peer)?;
    let signable = ann_signable(target, token, node_id, &peer_encoded, &[], &NS_UNANNOUNCE);
    let signature = sign_detached(&signable, &key_pair.secret_key);
    let ann = AnnounceMessage {
        peer: Some(peer),
        refresh: None,
        signature: Some(signature),
        bump: 0,
    };
    Ok(encode_announce_to_bytes(&ann)?)
}

/// Runs all `jobs` at once; how many returned `true` before `deadline`. Jobs still running at the
/// deadline are aborted and dropped (their requests are cancelled).
async fn settle_within<F>(deadline: Duration, jobs: Vec<F>) -> usize
where
    F: Future<Output = bool> + Send + 'static,
{
    let end = Instant::now() + deadline;
    let mut set = JoinSet::new();
    jobs.into_iter().for_each(|job| {
        set.spawn(job);
    });
    let mut confirmed = 0;
    while let Ok(Some(done)) = tokio::time::timeout_at(end, set.join_next()).await {
        confirmed += usize::from(matches!(done, Ok(true)));
    }
    set.shutdown().await;
    confirmed
}

#[cfg(test)]
#[path = "unannounce_tests.rs"]
mod tests;
