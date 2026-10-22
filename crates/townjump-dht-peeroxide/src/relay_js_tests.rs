// Live test: a JS client (`js/peer.cjs dial` with `relayThrough`) connects to a Rust server through the Rust relay node.
// On 127.0.0.1 the direct path may win; what counts is that the JS client reached the Rust relay and the data came back.
use super::tests::support::start_testnet;
use super::tests::{PAYLOAD, announced_relay, echo_first, listening, poll_stats};
use crate::js_peer_support_tests::JsPeer;
use std::time::Duration;
use townjump_dht::ServerOpts;
use townjump_testkit::key_pair;

const OPEN_WAIT: Duration = Duration::from_secs(20);
const DATA_WAIT: Duration = Duration::from_secs(10);

/// Waits until the DATA events of `peer` add up to `want` bytes; returns everything received.
async fn received(peer: &JsPeer, want: usize) -> String {
    let mut got = String::new();
    while got.len() < want {
        match peer.wait_event("DATA", DATA_WAIT).await {
            Some(event) => got.push_str(&event.data),
            None => panic!(
                "echo stopped at {} of {want} bytes: {}",
                got.len(),
                peer.dump()
            ),
        }
    }
    got
}

// JS: hyperdht/test/relaying.js — a client with `relayThrough` asks the relay to pair it with the server.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs the JS testnet: node and .tmp/npm-holesail"]
async fn js_client_connects_through_the_wrapper_relay() {
    let testnet = start_testnet();
    let bootstrap = testnet.factory.bootstrap.clone().expect("bootstrap");
    let relay = announced_relay(&testnet, 0xd1).await;
    let (_srv_node, _server, incoming) = listening(&testnet, 0xd2, ServerOpts::default()).await;
    echo_first(incoming);
    let server_pk = key_pair(0xd2).public_key;
    let mut js = JsPeer::dial_via_relay(&bootstrap, server_pk, relay.public_key());
    let open = js.wait_event("OPEN", OPEN_WAIT).await;
    assert!(open.is_some(), "JS client did not open: {}", js.dump());
    let sent: String = (0..PAYLOAD)
        .map(|i| (b'a' + (i % 26) as u8) as char)
        .collect();
    js.send_line(&format!("send {sent}"));
    let echoed = received(&js, PAYLOAD).await;
    assert_eq!(sent, echoed, "echo differs from what the JS client wrote");
    let stats = poll_stats(&relay, |s| s.pairings_requested >= 1).await;
    assert!(stats.pairings_requested >= 1, "{stats:?}");
    relay.close().await;
}
