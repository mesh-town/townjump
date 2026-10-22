// Common adapter test helpers: the JS testnet with a factory on its bootstrap, and a node without peers.
// Included via `#[path]` from dht_tests, server_tests and stream_tests; each needs its own part.
#![allow(dead_code)]
use crate::dht::PeeroxideDht;
use crate::factory::PeeroxideFactory;
use std::time::Duration;
use townjump_dht::DhtFactory;
use townjump_testkit::JsTestnet;
use townjump_testkit::conformance::wait::within;

/// JS network nodes beyond bootstrap (default of `interop/js/testnet.mjs`).
const TESTNET_NODES: usize = 4;
/// A port nobody listens on: bootstrap of an isolated node.
const CLOSED_BOOTSTRAP: &str = "127.0.0.1:1";
const CREATE_WAIT: Duration = Duration::from_secs(30);

/// A live JS DHT network and an adapter factory pointed at it; the network goes down with the value.
pub struct Testnet {
    pub factory: PeeroxideFactory,
    _net: JsTestnet,
}

/// Brings up a JS network on 127.0.0.1 (`node` and `.tmp/npm-holesail`); without a testnet it panics with the reason.
pub fn start_testnet() -> Testnet {
    let net = JsTestnet::start(TESTNET_NODES)
        .expect("JS testnet did not come up: node and .tmp/npm-holesail are required (interop/js/testnet.mjs)");
    let factory = PeeroxideFactory {
        bootstrap: Some(net.bootstrap()),
        ..Default::default()
    };
    Testnet { factory, _net: net }
}

/// A node without peers: bootstrap points at a closed loopback port, no network needed.
pub async fn isolated_node() -> PeeroxideDht {
    let factory = PeeroxideFactory {
        bootstrap: Some(vec![CLOSED_BOOTSTRAP.to_string()]),
        ..Default::default()
    };
    let created = within(
        CREATE_WAIT,
        "create of an isolated node",
        factory.create(None),
    )
    .await;
    created.expect("create of an isolated node: a node without peers needs no network")
}
