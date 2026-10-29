// Live tests of the relay node on the JS testnet: the `relay_through` option of either side reaches the relay.
// On 127.0.0.1 the direct path wins and the relay pairs in the background (the fork test covers data through the relay).
use super::*;
use crate::server::PeeroxideServer;
use peeroxide_dht::crypto::hash;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use townjump_dht::{
    ConnectOpts, Dht, DhtConnection, DhtFactory, DhtServer, Incoming, RelayThrough, ServerOpts,
};
use townjump_testkit::conformance::wait::{ACCEPT_WAIT, STREAM_WAIT, within};
use townjump_testkit::key_pair;

// One helper file is included in several test files: a separate copy in each would be pointless.
#[allow(clippy::duplicate_mod)]
#[path = "support_tests.rs"]
pub(super) mod support;
use support::{Testnet, start_testnet};

const ANNOUNCE_TRIES: usize = 50;
const POLL_TRIES: usize = 50;
const POLL_STEP: Duration = Duration::from_millis(200);
pub(super) const PAYLOAD: usize = 4096;

/// A relay the whole cluster can find by its key.
pub(super) async fn announced_relay(testnet: &Testnet, tag: u8) -> PeeroxideRelay {
    let created = testnet.factory.create_relay(Some(key_pair(tag)));
    let relay = within(ACCEPT_WAIT, "create_relay", created).await;
    let relay = relay.expect("create_relay");
    let topic = hash(&relay.public_key());
    for _ in 0..ANNOUNCE_TRIES {
        if matches!(relay.node.handle.find_peer(topic).await, Ok(Some(_))) {
            return relay;
        }
        tokio::time::sleep(POLL_STEP).await;
    }
    panic!("relay was never announced");
}

/// A listening server on a key by label and its incoming queue; the node must outlive the test.
pub(super) async fn listening(
    testnet: &Testnet,
    tag: u8,
    opts: ServerOpts,
) -> (impl Dht, PeeroxideServer, Incoming) {
    let node = testnet.factory.create(Some(key_pair(tag)));
    let node = within(ACCEPT_WAIT, "server node", node)
        .await
        .expect("node");
    let (server, incoming) = node.create_server(opts).expect("create_server");
    let listen = within(ACCEPT_WAIT, "listen", server.listen(key_pair(tag)));
    listen.await.expect("listen");
    (node, server, incoming)
}

/// Echoes the first accepted connection until the client shuts down.
pub(super) fn echo_first(mut incoming: Incoming) {
    tokio::spawn(async move {
        let Some(mut conn) = incoming.accept().await else {
            return;
        };
        let mut buf = vec![0u8; 1024];
        while let Ok(n) = conn.stream.read(&mut buf).await {
            if n == 0 || conn.stream.write_all(&buf[..n]).await.is_err() {
                return;
            }
        }
    });
}

async fn client_connect(
    testnet: &Testnet,
    pk: [u8; 32],
    opts: ConnectOpts,
) -> (impl Dht, DhtConnection) {
    let node = testnet.factory.create(Some(key_pair(0xd3)));
    let node = within(ACCEPT_WAIT, "client node", node)
        .await
        .expect("node");
    let conn = within(ACCEPT_WAIT, "connect", node.connect(pk, opts)).await;
    (node, conn.expect("connect"))
}

async fn assert_echo(mut conn: DhtConnection) {
    let sent: Vec<u8> = (0..PAYLOAD).map(|i| (i % 251) as u8).collect();
    conn.stream.write_all(&sent).await.expect("write");
    let mut echoed = vec![0u8; PAYLOAD];
    let read = within(STREAM_WAIT, "echo", conn.stream.read_exact(&mut echoed));
    read.await.expect("read of the echo");
    assert_eq!(sent, echoed, "echo differs from what was written");
}

/// Polls the relay counter until `done` holds or ~10 s pass; returns the last snapshot.
pub(super) async fn poll_stats(
    relay: &PeeroxideRelay,
    done: impl Fn(&RelayStats) -> bool,
) -> RelayStats {
    let mut stats = relay.stats();
    for _ in 0..POLL_TRIES {
        stats = relay.stats();
        if done(&stats) {
            break;
        }
        tokio::time::sleep(POLL_STEP).await;
    }
    stats
}

// JS: hyperdht/test/relaying.js — `relayThrough` on the connecting side makes the client ask the relay.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs the JS testnet: node and .tmp/npm-holesail"]
async fn client_option_reaches_the_relay() {
    let testnet = start_testnet();
    let relay = announced_relay(&testnet, 0xd1).await;
    let (_srv_node, _server, incoming) = listening(&testnet, 0xd2, ServerOpts::default()).await;
    echo_first(incoming);
    let opts = ConnectOpts {
        relay_through: Some(RelayThrough::Key(relay.public_key())),
        ..Default::default()
    };
    let (_node, conn) = client_connect(&testnet, key_pair(0xd2).public_key, opts).await;
    assert_echo(conn).await;
    let stats = poll_stats(&relay, |s| s.pairings_requested >= 1).await;
    assert!(stats.pairings_requested >= 1, "{stats:?}");
    relay.close().await;
}

// JS: hyperdht/test/relaying.js — `relayThrough` on the server side makes the server ask the relay.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs the JS testnet: node and .tmp/npm-holesail"]
async fn server_option_reaches_the_relay() {
    let testnet = start_testnet();
    let relay = announced_relay(&testnet, 0xd1).await;
    let opts = ServerOpts {
        relay_through: Some(RelayThrough::Key(relay.public_key())),
        ..Default::default()
    };
    let (_srv_node, _server, incoming) = listening(&testnet, 0xd2, opts).await;
    echo_first(incoming);
    let (_node, conn) =
        client_connect(&testnet, key_pair(0xd2).public_key, ConnectOpts::default()).await;
    assert_echo(conn).await;
    let stats = poll_stats(&relay, |s| s.pairings_requested >= 1).await;
    assert!(stats.pairings_requested >= 1, "{stats:?}");
    relay.close().await;
}

// JS: hyperdht/test/relaying.js — without `relayThrough` nobody talks to the relay.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs the JS testnet: node and .tmp/npm-holesail"]
async fn no_relay_without_option() {
    let testnet = start_testnet();
    let relay = announced_relay(&testnet, 0xd1).await;
    let (_srv_node, _server, incoming) = listening(&testnet, 0xd2, ServerOpts::default()).await;
    echo_first(incoming);
    let (_node, conn) =
        client_connect(&testnet, key_pair(0xd2).public_key, ConnectOpts::default()).await;
    assert_echo(conn).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let stats = relay.stats();
    assert_eq!(stats.pairings_requested, 0, "{stats:?}");
    relay.close().await;
}

// A relay key nobody announced: the failed relay attempt does not change the result while the direct path works.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs the JS testnet: node and .tmp/npm-holesail"]
async fn unannounced_relay_key_still_connects() {
    let testnet = start_testnet();
    let (_srv_node, _server, incoming) = listening(&testnet, 0xd2, ServerOpts::default()).await;
    echo_first(incoming);
    let opts = ConnectOpts {
        relay_through: Some(RelayThrough::Key([0x86; 32])),
        ..Default::default()
    };
    let (_node, conn) = client_connect(&testnet, key_pair(0xd2).public_key, opts).await;
    assert_echo(conn).await;
}
