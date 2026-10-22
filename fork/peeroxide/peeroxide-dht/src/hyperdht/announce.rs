//! `HyperDhtHandle::announce` and `unannounce` (LOOKUP walk) here; servers withdraw via
//! `unannounce_relays` in `unannounce.rs`. Neighbours: `lookup` finds peers; `server` calls `announce_server`.

use crate::crypto::{NS_ANNOUNCE, ann_signable, hash, sign_detached};
use crate::hyperdht_messages::{
    ANNOUNCE, AnnounceMessage, HyperPeer, LOOKUP, RelayInfo, UNANNOUNCE, encode_announce_to_bytes,
    encode_hyper_peer_to_bytes,
};
use crate::messages::Ipv4Peer;
use crate::rpc::{UserQueryParams, UserRequestParams};

use super::handle::HyperDhtHandle;
use super::keypair::KeyPair;
use super::types::{AnnounceResult, HyperDhtError};

/// A server's announcement goes to this many closest repliers (JS `announcer.js` `pickBest`).
const SERVER_RELAYS: usize = 3;

/// Who gets the ANNOUNCE.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fanout {
    /// Every replier with a token, after the query's commit phase (topics, JS `HyperDHT.announce`).
    All,
    /// The `SERVER_RELAYS` closest repliers with a token, no commit phase; only those that accepted
    /// (`error == 0`) are relays (a server's own key, JS `Announcer._update` and `_commit`).
    Server,
}

impl HyperDhtHandle {
    // ── ANNOUNCE ─────────────────────────────────────────────────────────────

    /// Announce this peer under the given target.
    pub async fn announce(
        &self,
        target: [u8; 32],
        key_pair: &KeyPair,
        relay_addresses: &[Ipv4Peer],
    ) -> Result<AnnounceResult, HyperDhtError> {
        self.announce_with(target, key_pair, relay_addresses, Fanout::All)
            .await
    }

    /// Announce of a listening server under `target` = `hash(public_key)`, as JS `Announcer._update`:
    /// ANNOUNCE only to the 3 closest repliers; the relays are those that accepted.
    pub(crate) async fn announce_server(
        &self,
        target: [u8; 32],
        key_pair: &KeyPair,
    ) -> Result<AnnounceResult, HyperDhtError> {
        self.announce_with(target, key_pair, &[], Fanout::Server)
            .await
    }

    async fn announce_with(
        &self,
        target: [u8; 32],
        key_pair: &KeyPair,
        relay_addresses: &[Ipv4Peer],
        fanout: Fanout,
    ) -> Result<AnnounceResult, HyperDhtError> {
        let replies = self
            .dht
            .query(UserQueryParams {
                target,
                command: LOOKUP,
                value: None,
                commit: fanout == Fanout::All,
                concurrency: None,
            })
            .await?;

        let mut closest_nodes = Vec::new();
        let mut accepted_relays: Vec<Ipv4Peer> = Vec::new();
        let mut relays: Vec<RelayInfo> = Vec::new();
        let mut sent = 0usize;

        for reply in &replies {
            closest_nodes.push(reply.from.clone());
            if fanout == Fanout::Server && sent == SERVER_RELAYS {
                continue;
            }

            let token = match &reply.token {
                Some(t) => *t,
                None => continue,
            };
            let node_id = match &reply.from_id {
                Some(id) => *id,
                None => continue,
            };

            // Stored record uses caller-provided relays verbatim, matching
            // Node `_requestAnnounce`: `ann.peer.relayAddresses = relayAddresses || []`.
            // Closest-ack accumulation is kept only in handle-state
            // (`current_relay_addresses` below) for *subsequent* announces
            // to consume — it is NEVER written into a stored peer record.
            // Do not "fix" this by re-adding per-record accumulation; the
            // TOPIC-close nodes don't hold FE for `hash(pk)`, so advertising
            // them as PEER_HANDSHAKE relays yields "empty reply".
            let record_relays: Vec<Ipv4Peer> = relay_addresses.iter().take(3).cloned().collect();

            let peer = HyperPeer {
                public_key: key_pair.public_key,
                relay_addresses: record_relays,
            };

            let peer_encoded = encode_hyper_peer_to_bytes(&peer)?;
            let signable =
                ann_signable(&target, &token, &node_id, &peer_encoded, &[], &NS_ANNOUNCE);
            let signature = sign_detached(&signable, &key_pair.secret_key);

            let ann = AnnounceMessage {
                peer: Some(peer),
                refresh: None,
                signature: Some(signature),
                bump: 0,
            };
            let ann_bytes = encode_announce_to_bytes(&ann)?;

            sent += 1;
            let res = self
                .dht
                .request(
                    UserRequestParams {
                        token: Some(token),
                        command: ANNOUNCE,
                        target: Some(target),
                        value: Some(ann_bytes),
                        timeout_ms: None,
                        retries: None,
                    },
                    &reply.from.host,
                    reply.from.port,
                )
                .await;
            let accepted = matches!(&res, Ok(r) if r.error == 0);
            let to = format!("{}:{}", reply.from.host, reply.from.port);
            tracing::debug!(to = %to, accepted, "announce: ANNOUNCE sent");
            if fanout == Fanout::Server && !accepted {
                continue;
            }

            // Accumulate this acker into handle-state relay list (Node
            // `Announcer._commit`: `if (relayAddresses.length < 3)
            // relayAddresses.push({ host: msg.from.host, port: msg.from.port })`).
            // We push only once per accepted reply and cap at 3. Dedup is
            // implicit — each `reply.from` is a distinct DHT node.
            if accepted_relays.len() < 3 {
                accepted_relays.push(reply.from.clone());
            }

            if let Some(to) = &reply.to {
                relays.push(RelayInfo {
                    relay_address: reply.from.clone(),
                    peer_address: to.clone(),
                });
            }
        }

        // Only update handle-state `current_relay_addresses` when announcing
        // on `hash(public_key)` (self-announce). Topic-announces also produce
        // a `closest_nodes` set, but those are topic-close nodes (NOT FE-holders
        // for PEER_HANDSHAKE forwarding). Mixing them into `current_relay_addresses`
        // — when parallel announces race on the shared Mutex — would corrupt
        // the relay list the topic-announce reads on a subsequent refresh.
        // See peer_discovery::do_refresh for the parallel-join order.
        if target == hash(&key_pair.public_key) {
            if let Ok(mut guard) = self.current_relay_addresses.lock() {
                *guard = accepted_relays;
            }
        }

        Ok(AnnounceResult {
            closest_nodes,
            relays,
        })
    }

    // ── UNANNOUNCE ────────────────────────────────────────────────────────────

    /// Remove a previously announced peer record.
    pub async fn unannounce(
        &self,
        target: [u8; 32],
        key_pair: &KeyPair,
    ) -> Result<(), HyperDhtError> {
        let replies = self
            .dht
            .query(UserQueryParams {
                target,
                command: LOOKUP,
                value: None,
                commit: false,
                concurrency: None,
            })
            .await?;

        for reply in &replies {
            let token = match &reply.token {
                Some(t) => *t,
                None => continue,
            };
            let node_id = match &reply.from_id {
                Some(id) => *id,
                None => continue,
            };

            let ann_bytes =
                super::unannounce::unannounce_value(&target, &token, &node_id, key_pair)?;

            let _ = self
                .dht
                .request(
                    UserRequestParams {
                        token: Some(token),
                        command: UNANNOUNCE,
                        target: Some(target),
                        value: Some(ann_bytes),
                        timeout_ms: None,
                        retries: None,
                    },
                    &reply.from.host,
                    reply.from.port,
                )
                .await;
        }

        Ok(())
    }
}
