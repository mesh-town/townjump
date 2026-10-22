// JS: hyperdht/lib/connect.js:findAndConnect (for await of c.query), hyperdht/index.js:mapFindPeer
//! Candidates of the holder search: the FIND_PEER replies that carry a record are handed to the
//! race one by one as they arrive, the relay addresses stored in the records follow at the end of
//! the query. Neighbours: `connect_flow.rs` (the race), `connect_race.rs` (`Feed`),
//! `rpc/query_stream.rs` (the stream of replies).

use std::task::{Context, Poll};

use super::connect_race::Feed;
use crate::hyperdht_messages::decode_hyper_peer_from_bytes;
use crate::messages::Ipv4Peer;
use crate::query::QueryReply;
use crate::rpc::{DhtError, QueryStream};

/// One slot of the race: a single node, or the pre-connect addresses tried one after another.
pub(super) enum Candidate {
    Node(Ipv4Peer),
    Chain(Vec<Ipv4Peer>),
}

/// Peers in order, without repeats and without those in `skip`.
pub(super) fn unique(peers: impl Iterator<Item = Ipv4Peer>, skip: &[Ipv4Peer]) -> Vec<Ipv4Peer> {
    let mut out: Vec<Ipv4Peer> = Vec::new();
    for peer in peers {
        if !skip.contains(&peer) && !out.contains(&peer) {
            out.push(peer);
        }
    }
    out
}

/// Holders of the record (`reply.from`), then the relay addresses stored in their records.
/// Filters by `value.is_some()`, not by decoding the record: the tail after the end of the query
/// may hand out a holder with an unreadable record, while the stream (`poll_more`) skips such a
/// one, as js `mapFindPeer` does; the upstream behaviour is kept on purpose.
fn holder_relays(replies: &[QueryReply], skip: &[Ipv4Peer]) -> Vec<Ipv4Peer> {
    let holders = || replies.iter().filter(|r| r.value.is_some());
    let froms = holders().map(|r| r.from.clone());
    let records = holders().filter_map(|r| decode_hyper_peer_from_bytes(r.value.as_deref()?).ok());
    unique(froms.chain(records.flat_map(|p| p.relay_addresses)), skip)
}

pub(super) fn count_holders(replies: &[QueryReply]) -> usize {
    replies.iter().filter(|r| r.value.is_some()).count()
}

/// The FIND_PEER query as a feed of candidates. `tried` holds `pre` and everything handed out.
pub(super) struct HolderFeed {
    query: QueryStream,
    tried: Vec<Ipv4Peer>,
    closest: Option<Vec<QueryReply>>,
}

impl HolderFeed {
    pub(super) fn new(query: QueryStream, pre: &[Ipv4Peer]) -> Self {
        Self {
            query,
            tried: pre.to_vec(),
            closest: None,
        }
    }

    /// The closest replies of the finished query; empty until it ends.
    pub(super) fn closest(&self) -> &[QueryReply] {
        self.closest.as_deref().unwrap_or_default()
    }

    pub(super) fn tried(&self) -> &[Ipv4Peer] {
        &self.tried
    }

    /// The race is over: the query runs to its end on its own.
    pub(super) fn detach(self) {
        self.query.detach(move |r| log_query_end(&r, true));
    }

    /// A reply with a value: a holder if the value is a record (JS `mapFindPeer`), once per node.
    fn holder_batch(&mut self, reply: QueryReply) -> Vec<Candidate> {
        let is_record = reply
            .value
            .as_deref()
            .is_some_and(|v| decode_hyper_peer_from_bytes(v).is_ok());
        if !is_record || self.tried.contains(&reply.from) {
            return Vec::new();
        }
        let from = reply.from;
        tracing::debug!(
            relay = %format!("{}:{}", from.host, from.port),
            "find_and_connect: FIND_PEER holder"
        );
        self.tried.push(from.clone());
        vec![Candidate::Node(from)]
    }

    /// The query ended: remember `closest`, hand out the relay addresses from the records.
    fn end_batch(&mut self, outcome: Result<Vec<QueryReply>, DhtError>) -> Vec<Candidate> {
        log_query_end(&outcome, false);
        let replies = outcome.unwrap_or_default();
        let relays = holder_relays(&replies, &self.tried);
        self.tried.extend(relays.iter().cloned());
        self.closest = Some(replies);
        relays.into_iter().map(Candidate::Node).collect()
    }
}

/// The end of the FIND_PEER query: at its natural end (`detached = false`) or after the race
/// let it go (`detached = true`).
fn log_query_end(result: &Result<Vec<QueryReply>, DhtError>, detached: bool) {
    match result {
        Ok(replies) => tracing::debug!(
            reply_count = replies.len(),
            detached,
            "find_and_connect: FIND_PEER completed"
        ),
        Err(e) => tracing::debug!(err = %e, detached, "find_and_connect: FIND_PEER query failed"),
    }
}

impl Feed<Candidate> for HolderFeed {
    fn poll_more(&mut self, cx: &mut Context<'_>) -> Poll<Option<Vec<Candidate>>> {
        if self.closest.is_some() {
            return Poll::Ready(None);
        }
        match self.query.poll_value(cx) {
            Poll::Ready(Some(reply)) => Poll::Ready(Some(self.holder_batch(reply))),
            // The result never comes before the end of the values: `finish_internal` drops the
            // value listener first (query.rs).
            Poll::Ready(None) => {
                let outcome = std::task::ready!(self.query.poll_done(cx));
                Poll::Ready(Some(self.end_batch(outcome)))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(test)]
#[path = "connect_holders_tests.rs"]
mod tests;
