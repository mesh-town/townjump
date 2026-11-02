// Tests of `PeeroxideServer::listen` timing and reach: it waits for the node being alive only (JS: `dht.bind()`),
// the announce itself tops the query up from the bootstrap nodes (server.js `_listen`, dht-rpc query.js `_open`).
use crate::native_testnet_tests::native_testnet;
use std::time::{Duration, Instant};
use townjump_dht::{Dht, DhtFactory, DhtServer, ServerOpts};
use townjump_testkit::conformance::wait::within;
use townjump_testkit::key_pair;

#[allow(clippy::duplicate_mod)]
#[path = "support_tests.rs"]
mod support;

const WAIT: Duration = Duration::from_secs(30);
/// One dead bootstrap wave is about 4 s; two sequential ones are about 8 s.
const ONE_WAVE_LIMIT: Duration = Duration::from_secs(6);
const NETWORK_SIZE: usize = 20;
const KEYS: u8 = 6;
/// `SERVER_RELAYS` of the fork: a server announces to at most this many nodes.
const MAX_HOLDERS: usize = 3;

// JS: `Server.listen` awaits `dht.bind()` and starts the announcer; it never awaits the bootstrap walk.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn listen_returns_without_waiting_for_the_bootstrap_walk() {
    let node = support::isolated_node().await;
    let (server, _incoming) = node
        .create_server(ServerOpts::default())
        .expect("create_server");
    let started = Instant::now();
    let listened = within(WAIT, "listen", server.listen(key_pair(0xb2))).await;
    let took = started.elapsed();
    eprintln!("listen took {took:?}");
    listened.expect("listen on a node without peers");
    assert!(took < ONE_WAVE_LIMIT, "listen took {took:?}");
}

// The announce must still land on live nodes when `listen` does not wait for the bootstrap walk.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn listen_announce_reaches_the_closest_nodes() {
    let testnet = native_testnet(NETWORK_SIZE).await;
    let mut before = testnet.router_counts().await;
    // Servers stay up: a destroyed server withdraws its announcement and would blur the per-key delta.
    let mut alive = Vec::new();
    eprintln!("key | holders (node indexes)");
    for i in 0..KEYS {
        let node = testnet.factory.create(None).await.expect("create");
        let (server, incoming) = node
            .create_server(ServerOpts::default())
            .expect("create_server");
        within(WAIT, "listen", server.listen(key_pair(0xc0 + i)))
            .await
            .expect("listen");
        let after = testnet.router_counts().await;
        let holders: Vec<usize> = (0..after.len()).filter(|&n| after[n] > before[n]).collect();
        eprintln!("{i} | {} | {holders:?}", holders.len());
        assert!(
            (1..=MAX_HOLDERS).contains(&holders.len()),
            "key {i}: holders {holders:?}"
        );
        before = after;
        alive.push((node, server, incoming));
    }
}
