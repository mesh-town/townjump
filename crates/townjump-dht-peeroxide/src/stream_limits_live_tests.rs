// Live tests of the node-wide stream limits and of the stream id reported by a connection, on the in-process native testnet.
// Both sides are adapter nodes: the limits must reach the outgoing stream and the incoming one.
use crate::dht::PeeroxideDht;
use crate::factory::PeeroxideFactory;
use crate::server::PeeroxideServer;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use townjump_dht::{
    ConnectOpts, Dht, DhtConnection, DhtFactory, DhtServer, Incoming, ServerOpts, StreamLimits,
};
use townjump_testkit::conformance::wait::{ACCEPT_WAIT, STREAM_WAIT, within};
use townjump_testkit::key_pair;

use crate::native_testnet_tests::{NativeTestnet, native_testnet};

const NETWORK_SIZE: usize = 4;

/// libudx never goes below 64 KiB, so this is the smallest limit that can be observed.
const SMALL: u32 = 65_536;
/// The libudx default receive window (4 MiB).
const LIBUDX_DEFAULT: u32 = 4_194_304;

struct Pair {
    client: DhtConnection,
    server: DhtConnection,
    client_node: PeeroxideDht,
    server_node: PeeroxideDht,
    _listener: PeeroxideServer,
    _incoming: Incoming,
}

async fn node(testnet: &NativeTestnet, limits: StreamLimits, tag: u8) -> PeeroxideDht {
    let factory = PeeroxideFactory {
        stream_limits: limits,
        ..testnet.factory.clone()
    };
    let created = within(
        ACCEPT_WAIT,
        "node create",
        factory.create(Some(key_pair(tag))),
    );
    created.await.expect("node create")
}

/// One connection between two nodes with the given limits; both ends exchange a byte so the streams are live.
async fn connected(testnet: &NativeTestnet, limits: StreamLimits, tag: u8) -> Pair {
    let server_node = node(testnet, limits, tag).await;
    let (listener, mut incoming) = server_node
        .create_server(ServerOpts::default())
        .expect("create_server");
    let listen = listener.listen(key_pair(tag));
    within(ACCEPT_WAIT, "listen", listen).await.expect("listen");
    let client_node = node(testnet, limits, tag + 1).await;
    let dial = client_node.connect(key_pair(tag).public_key, ConnectOpts::default());
    let accept = within(ACCEPT_WAIT, "accept", incoming.accept());
    let (client, server) = tokio::join!(within(ACCEPT_WAIT, "connect", dial), accept);
    let (mut client, mut server) = (client.expect("connect"), server.expect("incoming"));
    client.stream.write_all(b"x").await.expect("write");
    client.stream.flush().await.expect("flush");
    let mut byte = [0u8; 1];
    let read = server.stream.read_exact(&mut byte);
    within(STREAM_WAIT, "read", read).await.expect("read");
    Pair {
        client,
        server,
        client_node,
        server_node,
        _listener: listener,
        _incoming: incoming,
    }
}

fn windows(pair: &Pair) -> (Vec<Option<u32>>, Vec<Option<u32>>) {
    let read = |node: &PeeroxideDht| {
        let live = node.node.tunnels.live();
        live.iter().map(|slot| slot.recv_window_max()).collect()
    };
    (read(&pair.client_node), read(&pair.server_node))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stream_limits_reach_both_sides() {
    let testnet = native_testnet(NETWORK_SIZE).await;
    let limits = StreamLimits {
        recv_window_max: Some(SMALL),
        ..StreamLimits::default()
    };

    let pair = connected(&testnet, limits, 0xd1).await;

    let (client, server) = windows(&pair);
    assert_eq!(client, vec![Some(SMALL)], "outgoing stream");
    assert_eq!(server, vec![Some(SMALL)], "incoming stream");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn default_limits_keep_libudx_defaults() {
    let testnet = native_testnet(NETWORK_SIZE).await;

    let pair = connected(&testnet, StreamLimits::default(), 0xd3).await;

    let (client, server) = windows(&pair);
    assert_eq!(client, vec![Some(LIBUDX_DEFAULT)]);
    assert_eq!(server, vec![Some(LIBUDX_DEFAULT)]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connection_reports_local_stream_id() {
    let testnet = native_testnet(NETWORK_SIZE).await;

    let pair = connected(&testnet, StreamLimits::default(), 0xd5).await;

    let (outgoing, incoming) = (pair.client.local_stream_id(), pair.server.local_stream_id());
    assert!(outgoing.is_some(), "outgoing connection has no stream id");
    assert!(incoming.is_some(), "incoming connection has no stream id");
    assert_ne!(outgoing, incoming);
}
