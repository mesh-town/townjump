// Live test of seed nodes against the JS testnet: a node with a dead bootstrap, seeded with JS nodes, finds a record.
// JS: dht-rpc `opts.nodes` (nodes inserted into the routing table without a ping); hyperdht 6.34.0 stores the record.
use crate::factory::PeeroxideFactory;
use std::net::SocketAddrV4;
use townjump_dht::{Dht, DhtFactory};
use townjump_testkit::conformance::wait::{ACCEPT_WAIT, within};
use townjump_testkit::key_pair;

// One helper file is included in several test files: a separate copy in each would be pointless.
#[allow(clippy::duplicate_mod)]
#[path = "support_tests.rs"]
mod support;
use support::start_testnet;

const DEAD_BOOTSTRAP: &str = "127.0.0.1:1";

fn reader_factory(seeds: Vec<SocketAddrV4>) -> PeeroxideFactory {
    let mut factory = PeeroxideFactory {
        bootstrap: Some(vec![DEAD_BOOTSTRAP.to_string()]),
        ..Default::default()
    };
    factory.set_nodes(seeds);
    factory
}

/// Addresses of the testnet bootstrap node.
fn testnet_seeds(testnet: &support::Testnet) -> Vec<SocketAddrV4> {
    let list = testnet.factory.bootstrap.as_ref();
    let list = list.expect("testnet bootstrap");
    list.iter()
        .map(|a| a.parse().expect("bootstrap address"))
        .collect()
}

/// Reads the record with a fresh node of `factory`; `Err` and `Ok(None)` both come back as `None`.
async fn read_with(
    factory: &PeeroxideFactory,
    pk: [u8; 32],
) -> Option<townjump_dht::MutableRecord> {
    let node = within(ACCEPT_WAIT, "create reader", factory.create(None)).await;
    let node = node.expect("create reader");
    within(ACCEPT_WAIT, "get", node.mutable_get(pk, None))
        .await
        .ok()
        .flatten()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs the JS testnet: node and .tmp/npm-holesail"]
async fn seeded_node_finds_record_through_js_nodes() {
    let testnet = start_testnet();
    let pair = key_pair(0xb1);
    let writer = testnet.factory.create(Some(pair.clone()));
    let writer = within(ACCEPT_WAIT, "create writer", writer).await;
    let writer = writer.expect("create writer");
    let put = writer.mutable_put(&pair, b"saved-nodes", Some(1));
    within(ACCEPT_WAIT, "put", put).await.expect("put");

    let lost = read_with(&reader_factory(vec![]), pair.public_key).await;
    assert!(lost.is_none(), "unseeded node found: {lost:?}");
    let seeded = reader_factory(testnet_seeds(&testnet));
    let got = read_with(&seeded, pair.public_key).await;
    let got = got.expect("record found through seed nodes");
    assert_eq!((got.seq, got.value.as_slice()), (1, &b"saved-nodes"[..]));
}
