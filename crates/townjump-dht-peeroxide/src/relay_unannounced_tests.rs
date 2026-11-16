// A relay that is not announced is known to the server only by address: `PeeroxideServer::set_relay_addresses`
// replaces the list the server matches against the relay key a client names. Native network of the fork in this process.
use crate::native_testnet_tests::{
    DEFAULT_SIZE, NativeTestnet, assert_echo, echo_first, native_testnet, poll_stats, relay_only,
};
use crate::{PeeroxideRelay, RelayOpts};
use std::time::Duration;
use townjump_dht::{ConnectOpts, DhtServer, RelayAddress, ServerOpts};
use townjump_testkit::key_pair;

const RELAY: u8 = 0xe1;
const SERVER: u8 = 0xe2;
const CLIENT: u8 = 0xe3;
const POLL_STEP: Duration = Duration::from_millis(200);
const POLL_TRIES: usize = 50;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_reaches_unannounced_relay_after_set() {
    let net = native_testnet(DEFAULT_SIZE).await;
    let (_relay_net, relay) = unannounced_relay().await;
    let address = relay_address(&relay).await;
    let (_node, server, incoming) = net.listening(SERVER, ServerOpts::default()).await;
    let server_relayed = echo_first(incoming);

    server.set_relay_addresses(vec![address]);
    let (_client_node, result) = net
        .connect_from(CLIENT, key_pair(SERVER).public_key, through(&address))
        .await;

    let conn = result.expect("connect through the unannounced relay");
    assert!(conn.is_relayed(), "client end");
    assert_echo(conn).await;
    assert!(server_relayed.await.expect("server accepted"), "server end");
    let stats = poll_stats(&relay, |s| s.pairings_matched >= 1).await;
    assert!(stats.pairings_matched >= 1, "{stats:?}");
    server.close().await.expect("close");
    server.set_relay_addresses(vec![address]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_relay_addresses_replaces_the_list() {
    let net = native_testnet(DEFAULT_SIZE).await;
    let (_relay_net, relay) = unannounced_relay().await;
    let address = relay_address(&relay).await;
    let opts = ServerOpts {
        relay_addresses: vec![address],
        ..Default::default()
    };
    let (_node, server, incoming) = net.listening(SERVER, opts).await;
    echo_first(incoming);

    server.set_relay_addresses(vec![]);
    let (_client_node, result) = net
        .connect_from(CLIENT, key_pair(SERVER).public_key, through(&address))
        .await;

    assert!(result.is_err(), "the emptied list must not reach the relay");
}

fn through(address: &RelayAddress) -> ConnectOpts {
    let mut opts = ConnectOpts {
        relay_through: Some(address.public_key.into()),
        relay_addresses: vec![*address],
        ..Default::default()
    };
    relay_only(&mut opts);
    opts
}

/// The relay lives in a network of its own, so a search by key through the network of the peers cannot find it:
/// only its address reaches it.
async fn unannounced_relay() -> (NativeTestnet, PeeroxideRelay) {
    let island = native_testnet(1).await;
    let relay = island
        .relay_with(RelayOpts {
            identity: Some(key_pair(RELAY)),
            announce: false,
            ..Default::default()
        })
        .await;
    (island, relay)
}

async fn relay_address(relay: &PeeroxideRelay) -> RelayAddress {
    for _ in 0..POLL_TRIES {
        if let Ok(Some(address)) = relay.relay_address().await {
            return address;
        }
        tokio::time::sleep(POLL_STEP).await;
    }
    panic!("the relay never learned its address");
}
