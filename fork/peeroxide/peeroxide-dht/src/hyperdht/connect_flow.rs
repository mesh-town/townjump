// JS: hyperdht/lib/connect.js:findAndConnect, connectThroughNodes
//! The attempts of `connect_with_options` (one per relay key of a `RelayThrough::List`) and the
//! holder search of one attempt: the pre-connect chain and the FIND_PEER holders race in
//! parallel, FIND_NODE candidates are the fallback. Neighbours: `connect.rs` (the entry point and
//! one handshake: `handshake_through_node`), `connect_finish.rs` (`finish_attempt`),
//! `connect_race.rs` (parallelism), `connect_holders.rs` (the FIND_PEER feed of candidates),
//! `connect_error.rs` (final error).

use std::future::ready;

use libudx::UdxRuntime;

use super::connect_error::{connect_failure, is_terminal};
use super::connect_finish::AttemptError;
use super::connect_holders::{Candidate, HolderFeed, count_holders, unique};
use super::connect_race::{Attempt, Raced, first_success, first_success_fed};
use super::connect_route::Via;
use super::connect_trace::trace;
use super::relay::{RelayAsk, RelayThrough, resolve};
use super::{ConnectOpts, HyperDhtError, HyperDhtHandle, KeyPair, PeerConnection};
use crate::crypto::hash;
use crate::hyperdht_messages::NoisePayload;
use crate::messages::Ipv4Peer;
use crate::noise_wrap::NoiseWrapResult;
use crate::query::QueryReply;
use crate::router::HandshakeResult;
use crate::rpc::Session;

/// A PEER_HANDSHAKE reply that passed Noise: what `finish_attempt` continues from.
pub(super) struct Handshaken {
    pub(super) relay: Ipv4Peer,
    pub(super) nw_result: NoiseWrapResult,
    pub(super) remote_payload: NoisePayload,
    pub(super) hs_result: HandshakeResult,
    pub(super) local_stream_id: u32,
    /// The relay and token this handshake announced (own token per handshake).
    pub(super) relay_ask: Option<RelayAsk>,
    /// Socket and stream the PEER_HANDSHAKE went over (a retry by route); `None` — the node's socket.
    pub(super) via: Option<Via>,
}

/// Index in a `RelayThrough::List` where this connect starts: random, fixed by tests.
#[cfg(test)]
fn relay_start(opts: &ConnectOpts) -> usize {
    opts.relay_start
        .unwrap_or_else(|| rand::random::<u32>() as usize)
}

#[cfg(not(test))]
fn relay_start(_opts: &ConnectOpts) -> usize {
    rand::random::<u32>() as usize
}

impl HyperDhtHandle {
    /// JS `connect.js:498-536`: the relay and token are chosen once per attempt. With several relay
    /// keys (`RelayThrough::List`) the next key gets a whole new attempt (handshake, token, pairing)
    /// only after our own relay aborted; any other failure, and the abort of the server's relay,
    /// end the connect.
    pub(super) async fn connect_candidates(
        &self,
        key_pair: &KeyPair,
        remote_public_key: [u8; 32],
        relay_addresses: &[Ipv4Peer],
        runtime: &UdxRuntime,
        opts: ConnectOpts,
    ) -> Result<PeerConnection, HyperDhtError> {
        // No keys — one attempt without a relay.
        let mut keys: Vec<Option<[u8; 32]>> = opts
            .relay_through
            .as_ref()
            .map(|rt| rt.candidates(relay_start(&opts)))
            .unwrap_or_default()
            .into_iter()
            .map(Some)
            .collect();
        if keys.is_empty() {
            keys.push(None);
        }
        let n = keys.len();
        for (i, key) in keys.into_iter().enumerate() {
            if i > 0 {
                // The node closed in the middle of the walk: `Destroyed`, not one more attempt.
                self.ensure_running()?;
            }
            let opts = ConnectOpts {
                relay_ask: key.and_then(|k| resolve(Some(&RelayThrough::Key(k)))),
                ..opts.clone()
            };
            let pk = remote_public_key;
            let attempt =
                self.connect_attempt(key_pair, pk, relay_addresses, runtime, &opts, i == 0);
            match attempt.await {
                Ok(conn) => return Ok(conn),
                Err(AttemptError::OwnRelayAborted) if i + 1 < n => {
                    let relay = key
                        .map(|k| {
                            k[..8]
                                .iter()
                                .map(|b| format!("{b:02x}"))
                                .collect::<String>()
                        })
                        .unwrap_or_default();
                    tracing::debug!(
                        attempt = i + 1,
                        of = n,
                        relay,
                        "connect: own relay aborted, next key"
                    );
                }
                Err(e) => return Err(e.into()),
            }
        }
        Err(HyperDhtError::RelayAborted)
    }

    /// Copy of the handle whose DHT calls run in `session` (it cancels them when it ends).
    pub(super) fn in_session(&self, session: &Session) -> HyperDhtHandle {
        HyperDhtHandle {
            dht: session.dht().clone(),
            ..self.clone()
        }
    }

    /// The handshakes run in the session of the connect: dropping the future (or any exit)
    /// cancels the ones still running, as JS `connect.js` does with its session. FIND_PEER runs
    /// in a session of its own: after `Won`/`Stopped` `HolderFeed::detach` lets it run on, so late
    /// replies still reach the routing table. A deliberate departure
    /// from JS `connect.js:547`, which stops the query on the win.
    pub(super) async fn find_and_connect(
        &self,
        key_pair: &KeyPair,
        remote_public_key: [u8; 32],
        pre: &[Ipv4Peer],
        runtime: &UdxRuntime,
        opts: &ConnectOpts,
    ) -> Result<PeerConnection, AttemptError> {
        let session = self.dht.session();
        self.in_session(&session)
            .find_and_connect_in(&session, key_pair, remote_public_key, pre, runtime, opts)
            .await
    }

    async fn find_and_connect_in(
        &self,
        session: &Session,
        key_pair: &KeyPair,
        remote_public_key: [u8; 32],
        pre: &[Ipv4Peer],
        runtime: &UdxRuntime,
        opts: &ConnectOpts,
    ) -> Result<PeerConnection, AttemptError> {
        let target = hash(&remote_public_key);
        let chain = if pre.is_empty() {
            Vec::new()
        } else {
            vec![Candidate::Chain(pre.to_vec())]
        };
        let pk = &remote_public_key;
        let attempt = move |c| -> Attempt<'_, Handshaken> {
            Box::pin(self.handshake_candidate(key_pair, pk, c, opts))
        };
        let t0 = tokio::time::Instant::now();
        trace("FIND_PEER start", None, t0);
        let mut feed = HolderFeed::new(self.find_peer_stream(target), pre);
        let raced = first_success_fed(
            chain,
            &mut feed,
            if pre.is_empty() { 2 } else { 3 },
            attempt,
            HyperDhtError::NoRelayNodes,
        )
        .await;
        trace("FIND_PEER raced end", None, t0);
        let holders = count_holders(feed.closest());
        let Raced::Exhausted(last_err) = raced else {
            let tried = feed.tried().to_vec();
            feed.detach();
            return self
                .finish(session, raced, holders, &tried, key_pair, pk, runtime, opts)
                .await;
        };
        let raced = self
            .race_find_node(target, pre, feed.closest(), feed.tried(), attempt, last_err)
            .await?;
        self.finish(
            session,
            raced,
            holders,
            feed.tried(),
            key_pair,
            pk,
            runtime,
            opts,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn finish(
        &self,
        session: &Session,
        raced: Raced<Handshaken>,
        holders: usize,
        tried: &[Ipv4Peer],
        key_pair: &KeyPair,
        remote_public_key: &[u8; 32],
        runtime: &UdxRuntime,
        opts: &ConnectOpts,
    ) -> Result<PeerConnection, AttemptError> {
        match raced {
            Raced::Won(h) => {
                // The losing handshakes stop with the session of the connect (it stays usable
                // for the holepunch). FIND_PEER is not in it: `HolderFeed::detach` lets it finish
                // for the routing table, unlike JS `connect.js:547`, which stops the query.
                session.destroy();
                self.cache_relays(remote_public_key, &h.remote_payload, tried);
                self.finish_attempt(h, key_pair, remote_public_key, runtime, opts)
                    .await
            }
            Raced::Stopped(e) => Err(e.into()),
            Raced::Exhausted(e) => Err(connect_failure(holders, e).into()),
        }
    }

    /// Nodes the routing table reaches that did not return the record (e.g. mid-refresh).
    async fn race_find_node<'a>(
        &self,
        target: [u8; 32],
        pre: &[Ipv4Peer],
        replies: &[QueryReply],
        tried: &[Ipv4Peer],
        attempt: impl Fn(Candidate) -> Attempt<'a, Handshaken>,
        last_err: HyperDhtError,
    ) -> Result<Raced<Handshaken>, HyperDhtError> {
        let node_replies = self
            .dht
            .find_node(target)
            .await
            .map_err(HyperDhtError::Dht)?;
        if pre.is_empty() && replies.is_empty() && node_replies.is_empty() {
            return Err(HyperDhtError::PeerNotFound);
        }
        let nodes = node_replies
            .iter()
            .flat_map(|r| std::iter::once(r.from.clone()).chain(r.closer_nodes.iter().cloned()));
        let candidates: Vec<Candidate> = unique(nodes, tried)
            .into_iter()
            .map(Candidate::Node)
            .collect();
        tracing::debug!(
            candidate_count = candidates.len(),
            "find_and_connect: FIND_NODE candidates"
        );
        Ok(first_success(candidates, ready(Vec::new()), 2, attempt, last_err).await)
    }

    async fn handshake_candidate(
        &self,
        key_pair: &KeyPair,
        remote_public_key: &[u8; 32],
        candidate: Candidate,
        opts: &ConnectOpts,
    ) -> Result<Handshaken, HyperDhtError> {
        match candidate {
            Candidate::Node(relay) => {
                self.handshake_logged(key_pair, remote_public_key, &relay, opts)
                    .await
            }
            Candidate::Chain(relays) => {
                self.handshake_chain(key_pair, remote_public_key, &relays, opts)
                    .await
            }
        }
    }

    /// JS `connectThroughNodes`: addresses one after another, failures swallowed.
    async fn handshake_chain(
        &self,
        key_pair: &KeyPair,
        remote_public_key: &[u8; 32],
        relays: &[Ipv4Peer],
        opts: &ConnectOpts,
    ) -> Result<Handshaken, HyperDhtError> {
        let mut last = HyperDhtError::NoRelayNodes;
        for relay in relays {
            match self
                .handshake_logged(key_pair, remote_public_key, relay, opts)
                .await
            {
                Ok(handshaken) => return Ok(handshaken),
                Err(e) if is_terminal(&e) => return Err(e),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    async fn handshake_logged(
        &self,
        key_pair: &KeyPair,
        remote_public_key: &[u8; 32],
        relay: &Ipv4Peer,
        opts: &ConnectOpts,
    ) -> Result<Handshaken, HyperDhtError> {
        let result = self
            .handshake_through_node(key_pair, remote_public_key, relay, None, opts)
            .await;
        if let Err(e) = &result {
            tracing::debug!(relay = %format!("{}:{}", relay.host, relay.port), err = %e, "handshake attempt failed");
        }
        result
    }
}
