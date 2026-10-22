//! connect tries the holders of a record like JS `findAndConnect`: two handshakes at a time, a
//! silent holder gives up after ~4 s (dht-rpc: 1 s x 3 retries), the first success wins. JS
//! timings (local testnet, `testvectors/probes-js/fork-connect-parallel-probe-parallel.cjs`): every holder silent,
//! 3 holders -> 8 s (two waves of 4 s); first silent + second alive -> instant. 127.0.0.1 only.
mod common;

use std::time::{Duration, Instant};

use common::{cluster, serve_key, spawn_node, within};
use peeroxide_dht::crypto::hash;
use peeroxide_dht::hyperdht::{HyperDhtError, KeyPair};
use peeroxide_dht::messages::Ipv4Peer;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(4);

#[tokio::test]
async fn silent_server_costs_waves_of_two_not_the_sum() {
    within(async {
        let (_nodes, bootstrap) = cluster(4).await;
        let key = KeyPair::from_seed([0x71; 32]);
        let _served = serve_key(&bootstrap, &key, true).await;
        let client = spawn_node(bootstrap, true).await;
        let replies = client
            .handle
            .query_find_peer(hash(&key.public_key))
            .await
            .expect("find peer");
        let holders = replies.iter().filter(|r| r.value.is_some()).count() as u32;
        assert!(holders >= 3, "need >= 3 holders, got {holders}");
        let me = KeyPair::from_seed([0x72; 32]);

        let started = Instant::now();
        let err = client.handle.connect(&me, key.public_key, &client.rt).await;
        let elapsed = started.elapsed();

        assert!(matches!(err, Err(HyperDhtError::NoRelayNodes)), "{err:?}");
        let waves = holders.div_ceil(2);
        assert!(
            elapsed >= HANDSHAKE_TIMEOUT * waves - Duration::from_secs(1),
            "{elapsed:?}"
        );
        assert!(
            elapsed < HANDSHAKE_TIMEOUT * waves + Duration::from_secs(3),
            "{elapsed:?}"
        );
    })
    .await;
}

#[tokio::test]
async fn silent_pre_connect_relay_does_not_delay_the_holders() {
    within(async {
        let (_nodes, bootstrap) = cluster(3).await;
        let key = KeyPair::from_seed([0x73; 32]);
        let _served = serve_key(&bootstrap, &key, false).await;
        let client = spawn_node(bootstrap, true).await;
        let black_hole = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind");
        let relay = Ipv4Peer {
            host: "127.0.0.1".to_string(),
            port: black_hole.local_addr().expect("addr").port(),
        };
        let me = KeyPair::from_seed([0x74; 32]);

        let started = Instant::now();
        let result = client
            .handle
            .connect_with_nodes(&me, key.public_key, &[relay], &client.rt)
            .await;
        let elapsed = started.elapsed();

        assert!(result.is_ok(), "{:?}", result.err());
        assert!(elapsed < Duration::from_secs(3), "{elapsed:?}");
    })
    .await;
}

/// JS `findAndConnect` starts a handshake on the first FIND_PEER holder without waiting for the
/// query to end. A node that went silent holds the query for its `on_timeout` (~4 s), the
/// handshake must not.
#[tokio::test]
async fn a_silent_node_in_the_search_does_not_delay_the_handshake() {
    within(async {
        let (nodes, bootstrap) = cluster(4).await;
        let key = KeyPair::from_seed([0x75; 32]);
        let _served = serve_key(&bootstrap, &key, false).await;
        let client = spawn_node(bootstrap, true).await;
        nodes[1].handle.suspend().await.expect("suspend");
        let me = KeyPair::from_seed([0x76; 32]);

        let timed_connect = async {
            let started = Instant::now();
            let result = client.handle.connect(&me, key.public_key, &client.rt).await;
            (result, started.elapsed())
        };
        let timed_query = async {
            let started = Instant::now();
            let result = client.handle.query_find_peer(hash(&key.public_key)).await;
            (result, started.elapsed())
        };
        let ((connected, connect), (queried, query)) = tokio::join!(timed_connect, timed_query);

        eprintln!("T0 connect={connect:?} query={query:?}");
        assert!(connected.is_ok(), "{:?}", connected.err());
        assert!(queried.is_ok(), "{:?}", queried.err());
        assert!(
            query >= Duration::from_secs(3),
            "query does not wait for the silent node: {query:?}"
        );
        assert!(
            connect + Duration::from_secs(1) < query,
            "connect {connect:?} waits for the search end {query:?}"
        );
    })
    .await;
}
