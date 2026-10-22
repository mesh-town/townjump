// JS: hyperdht/lib/connect.js (for await of c.query), hyperdht/index.js mapFindPeer
//! Tests of `HolderFeed`: holders go out as the replies arrive, relays from records at the end.

use std::future::poll_fn;
use std::time::Duration;

use super::*;
use crate::hyperdht_messages::{HyperPeer, encode_hyper_peer_to_bytes};
use crate::rpc::DhtError;

fn peer(port: u16) -> Ipv4Peer {
    Ipv4Peer {
        host: "10.0.0.1".into(),
        port,
    }
}

fn reply(port: u16, value: Option<Vec<u8>>) -> QueryReply {
    QueryReply {
        from: peer(port),
        to: None,
        from_id: None,
        token: None,
        closer_nodes: Vec::new(),
        error: 0,
        value,
        rtt: Duration::ZERO,
    }
}

/// A reply of the holder `port` whose record lists `relays`.
fn holder(port: u16, relays: &[Ipv4Peer]) -> QueryReply {
    let record = HyperPeer {
        public_key: [7; 32],
        relay_addresses: relays.to_vec(),
    };
    reply(
        port,
        Some(encode_hyper_peer_to_bytes(&record).expect("encode")),
    )
}

fn nodes(batch: Option<Vec<Candidate>>) -> Vec<Ipv4Peer> {
    batch
        .expect("a batch")
        .into_iter()
        .map(|c| match c {
            Candidate::Node(p) => p,
            Candidate::Chain(_) => panic!("chain in the feed"),
        })
        .collect()
}

async fn next(feed: &mut HolderFeed) -> Option<Vec<Candidate>> {
    // A batch that must come before the end of the query: an implementation that gathers the
    // replies until the end fails here instead of hanging.
    tokio::time::timeout(Duration::from_secs(1), poll_fn(|cx| feed.poll_more(cx)))
        .await
        .expect("batch before the query end")
}

#[tokio::test]
async fn the_first_holder_is_handed_out_before_the_query_ends() {
    let (stream, values, _done) = QueryStream::for_test();
    let mut feed = HolderFeed::new(stream, &[]);
    values.send(holder(1, &[])).expect("send");
    assert_eq!(nodes(next(&mut feed).await), vec![peer(1)]);
    assert!(feed.closest().is_empty());
}

#[tokio::test]
async fn a_reply_whose_value_is_not_a_record_is_skipped() {
    let (stream, values, _done) = QueryStream::for_test();
    let mut feed = HolderFeed::new(stream, &[]);
    values.send(reply(1, Some(vec![1]))).expect("send");
    assert!(nodes(next(&mut feed).await).is_empty());
    assert!(feed.tried().is_empty());
}

#[tokio::test]
async fn pre_and_repeated_holders_are_not_handed_out() {
    let (stream, values, _done) = QueryStream::for_test();
    let mut feed = HolderFeed::new(stream, &[peer(9)]);
    for port in [9, 1, 1] {
        values.send(holder(port, &[])).expect("send");
    }
    assert!(nodes(next(&mut feed).await).is_empty());
    assert_eq!(nodes(next(&mut feed).await), vec![peer(1)]);
    assert!(nodes(next(&mut feed).await).is_empty());
}

#[tokio::test]
async fn the_end_hands_out_record_relays_and_keeps_closest() {
    let (stream, values, done) = QueryStream::for_test();
    let mut feed = HolderFeed::new(stream, &[peer(9)]);
    values.send(holder(1, &[])).expect("send");
    assert_eq!(nodes(next(&mut feed).await), vec![peer(1)]);
    drop(values);
    done.send(Ok(vec![holder(1, &[peer(2)])])).expect("done");
    assert_eq!(nodes(next(&mut feed).await), vec![peer(2)]);
    assert_eq!(feed.closest().len(), 1);
    assert_eq!(feed.tried(), &[peer(9), peer(1), peer(2)]);
    assert!(next(&mut feed).await.is_none());
}

#[tokio::test]
async fn a_failed_query_ends_with_nothing() {
    let (stream, values, done) = QueryStream::for_test();
    let mut feed = HolderFeed::new(stream, &[]);
    drop(values);
    done.send(Err(DhtError::ChannelClosed)).expect("done");
    assert!(nodes(next(&mut feed).await).is_empty());
    assert!(feed.closest().is_empty());
    assert!(next(&mut feed).await.is_none());
}
