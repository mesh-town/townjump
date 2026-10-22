//! `HyperDhtHandle::lookup`, `find_peer`, `query_find_peer`: read-only DHT queries.
//! Neighbours: `announce` and `records` hold the other queries.

use crate::hyperdht_messages::{
    FIND_PEER, HyperPeer, LOOKUP, decode_hyper_peer_from_bytes, decode_lookup_raw_reply_from_bytes,
};
use crate::query::QueryReply;
use crate::rpc::{QueryStream, UserQueryParams};

use super::handle::HyperDhtHandle;
use super::types::{HyperDhtError, LookupResult};

/// The FIND_PEER query of the target, as `find_peer`, `query_find_peer` and the stream run it.
fn find_peer_params(target: [u8; 32]) -> UserQueryParams {
    UserQueryParams {
        target,
        command: FIND_PEER,
        value: None,
        commit: false,
        concurrency: None,
    }
}

impl HyperDhtHandle {
    // ── LOOKUP ────────────────────────────────────────────────────────────────

    /// Query the DHT for peers advertising the target.
    pub async fn lookup(&self, target: [u8; 32]) -> Result<Vec<LookupResult>, HyperDhtError> {
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

        let mut results = Vec::new();
        for reply in replies {
            if let Some(value) = &reply.value {
                if let Ok(raw) = decode_lookup_raw_reply_from_bytes(value) {
                    if !raw.peers.is_empty() {
                        results.push(LookupResult {
                            from: reply.from.clone(),
                            to: None,
                            peers: raw.peers,
                        });
                    }
                }
            }
        }
        Ok(results)
    }

    // ── FIND_PEER ─────────────────────────────────────────────────────────────

    /// Return the first peer record found for the target.
    pub async fn find_peer(&self, target: [u8; 32]) -> Result<Option<HyperPeer>, HyperDhtError> {
        let replies = self.dht.query(find_peer_params(target)).await?;

        for reply in replies {
            if let Some(value) = reply.value {
                if let Ok(peer) = decode_hyper_peer_from_bytes(&value) {
                    return Ok(Some(peer));
                }
            }
        }
        Ok(None)
    }

    /// FIND_PEER query as a stream of the replies that carry a record (js `findAndConnect`).
    pub(super) fn find_peer_stream(&self, target: [u8; 32]) -> QueryStream {
        self.dht.query_stream(find_peer_params(target))
    }

    /// Run a FIND_PEER query and return all raw replies.
    ///
    /// Unlike [`find_peer`](Self::find_peer), this returns every reply from
    /// the iterative query so callers can try connecting through each
    /// responding node's address.
    pub async fn query_find_peer(
        &self,
        target: [u8; 32],
    ) -> Result<Vec<QueryReply>, HyperDhtError> {
        Ok(self.dht.query(find_peer_params(target)).await?)
    }
}
