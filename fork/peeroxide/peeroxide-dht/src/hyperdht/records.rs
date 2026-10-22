//! `HyperDhtHandle::immutable_*` / `mutable_*` and the helpers `encode_compact_uint`, `pick_latest_mutable`.
//! Neighbours: `types` holds the result records.

use crate::crypto::{NS_MUTABLE_PUT, hash, mutable_signable, sign_detached, verify_detached};
use crate::hyperdht_messages::{
    IMMUTABLE_GET, IMMUTABLE_PUT, MUTABLE_GET, MUTABLE_PUT, MutablePutRequest,
    decode_mutable_get_response_from_bytes, encode_mutable_put_request_to_bytes,
};
use crate::query::QueryReply;
use crate::rpc::{DhtError, UserQueryParams, UserRequestParams};

use super::handle::HyperDhtHandle;
use super::keypair::KeyPair;
use super::types::{HyperDhtError, ImmutablePutResult, MutableGetResult, MutablePutResult};

impl HyperDhtHandle {
    // ── IMMUTABLE_PUT ────────────────────────────────────────────────────────

    /// Store immutable content under its content hash.
    pub async fn immutable_put(&self, value: &[u8]) -> Result<ImmutablePutResult, HyperDhtError> {
        let target = hash(value);

        let replies = self
            .dht
            .query(UserQueryParams {
                target,
                command: IMMUTABLE_GET,
                value: None,
                commit: true,
                concurrency: None,
            })
            .await?;

        let mut closest_nodes = Vec::new();

        for reply in &replies {
            closest_nodes.push(reply.from.clone());

            let token = match &reply.token {
                Some(t) => *t,
                None => continue,
            };

            let _ = self
                .dht
                .request(
                    UserRequestParams {
                        token: Some(token),
                        command: IMMUTABLE_PUT,
                        target: Some(target),
                        value: Some(value.to_vec()),
                        timeout_ms: None,
                        retries: None,
                    },
                    &reply.from.host,
                    reply.from.port,
                )
                .await;
        }

        Ok(ImmutablePutResult {
            hash: target,
            closest_nodes,
        })
    }

    // ── IMMUTABLE_GET ────────────────────────────────────────────────────────

    /// Fetch immutable content by content hash.
    pub async fn immutable_get(&self, target: [u8; 32]) -> Result<Option<Vec<u8>>, HyperDhtError> {
        let replies = self
            .dht
            .query(UserQueryParams {
                target,
                command: IMMUTABLE_GET,
                value: None,
                commit: false,
                concurrency: None,
            })
            .await?;

        for reply in replies {
            if let Some(value) = reply.value {
                if hash(&value) == target {
                    return Ok(Some(value));
                }
            }
        }
        Ok(None)
    }

    // ── MUTABLE_PUT ───────────────────────────────────────────────────────────

    /// Store a signed mutable record for the given key pair.
    pub async fn mutable_put(
        &self,
        key_pair: &KeyPair,
        value: &[u8],
        seq: u64,
    ) -> Result<MutablePutResult, HyperDhtError> {
        let target = hash(&key_pair.public_key);
        let signable = mutable_signable(&NS_MUTABLE_PUT, seq, value);
        let signature = sign_detached(&signable, &key_pair.secret_key);

        let put = MutablePutRequest {
            public_key: key_pair.public_key,
            seq,
            value: value.to_vec(),
            signature,
        };
        let put_bytes = encode_mutable_put_request_to_bytes(&put)?;

        let seq_bytes = encode_compact_uint(seq);

        let replies = self
            .dht
            .query(UserQueryParams {
                target,
                command: MUTABLE_GET,
                value: Some(seq_bytes),
                commit: true,
                concurrency: None,
            })
            .await?;

        let mut closest_nodes = Vec::new();
        let mut commit_timeouts: u32 = 0;

        for reply in &replies {
            closest_nodes.push(reply.from.clone());

            let token = match &reply.token {
                Some(t) => *t,
                None => continue,
            };

            if let Err(DhtError::RequestFailed(_)) = self
                .dht
                .request(
                    UserRequestParams {
                        token: Some(token),
                        command: MUTABLE_PUT,
                        target: Some(target),
                        value: Some(put_bytes.clone()),
                        timeout_ms: None,
                        retries: None,
                    },
                    &reply.from.host,
                    reply.from.port,
                )
                .await
            {
                commit_timeouts += 1;
            }
        }

        Ok(MutablePutResult {
            public_key: key_pair.public_key,
            closest_nodes,
            seq,
            signature,
            commit_timeouts,
        })
    }

    // ── MUTABLE_GET ───────────────────────────────────────────────────────────

    /// Fetch and verify a mutable record for the given public key.
    pub async fn mutable_get(
        &self,
        public_key: &[u8; 32],
        seq: u64,
    ) -> Result<Option<MutableGetResult>, HyperDhtError> {
        let target = hash(public_key);
        let seq_bytes = encode_compact_uint(seq);

        let replies = self
            .dht
            .query(UserQueryParams {
                target,
                command: MUTABLE_GET,
                value: Some(seq_bytes),
                commit: false,
                concurrency: None,
            })
            .await?;

        Ok(pick_latest_mutable(replies, public_key, seq))
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

pub(super) fn encode_compact_uint(v: u64) -> Vec<u8> {
    let mut state = crate::compact_encoding::State::new();
    crate::compact_encoding::preencode_uint(&mut state, v);
    state.alloc();
    crate::compact_encoding::encode_uint(&mut state, v);
    state.buffer
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Among verified replies with `seq >= min_seq`, picks the one with the highest `seq`
/// (JS hyperdht `get` with `latest`); ties keep the first reply.
pub(super) fn pick_latest_mutable(
    replies: Vec<QueryReply>,
    public_key: &[u8; 32],
    min_seq: u64,
) -> Option<MutableGetResult> {
    let mut best: Option<MutableGetResult> = None;
    for reply in replies {
        let Some(value) = &reply.value else { continue };
        let Ok(resp) = decode_mutable_get_response_from_bytes(value) else {
            continue;
        };
        let newer = best.as_ref().is_none_or(|b| resp.seq > b.seq);
        let signable = mutable_signable(&NS_MUTABLE_PUT, resp.seq, &resp.value);
        if resp.seq < min_seq || !newer || !verify_detached(&resp.signature, &signable, public_key)
        {
            continue;
        }
        best = Some(MutableGetResult {
            value: resp.value,
            seq: resp.seq,
            signature: resp.signature,
            from: reply.from,
        });
    }
    best
}
