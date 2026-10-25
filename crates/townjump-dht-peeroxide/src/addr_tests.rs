// Tests of the address-aware part of `PeeroxideDht`: `local_port`, `connect_direct`, the `firewalled` flag of the node mode.
// Nodes without peers talk over 127.0.0.1 (no JS network); the live test (`#[ignore]`) checks `remote_addr` of a connect by key.
use super::*;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use townjump_dht::{DhtServer, Duplex};
use townjump_testkit::conformance::setup::{connect_pair, listen_on, node};
use townjump_testkit::conformance::wait::{accept_one, within};
use townjump_testkit::key_pair;

#[allow(clippy::duplicate_mod)]
#[path = "support_tests.rs"]
mod support;

use support::isolated_node;

/// A direct connect to a live listener, or a failure of it, ends well within this.
const DIAL_WAIT: Duration = Duration::from_secs(15);
const LISTEN_WAIT: Duration = Duration::from_secs(30);
const OFFLINE_WAIT: Duration = Duration::from_secs(30);
/// Bootstrap of an isolated node takes about 4 s; a direct connect must not wait for it.
const NO_BOOTSTRAP_WAIT: Duration = Duration::from_secs(2);
const SERVER_SEED: u8 = 0xd1;

mod local_port {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn of_a_live_node_is_nonzero() {
        let node = isolated_node().await;
        let port = within(OFFLINE_WAIT, "local_port", node.local_port()).await;
        let _ = node.destroy().await;
        assert!(matches!(port, Ok(p) if p != 0), "got {port:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn of_a_destroyed_node_fails_with_node_destroyed() {
        let node = destroyed_node().await;
        let port = within(OFFLINE_WAIT, "local_port", node.local_port()).await;
        assert!(matches!(port, Err(DhtError::Destroyed)), "got {port:?}");
    }
}

mod connect_direct {
    use super::*;

    mod to_a_listening_server {
        use super::*;

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn carries_data_both_ways() {
            let mut dialed = dial_a_listener().await;
            assert_eq!(dialed.client.remote_public_key, dialed.server_key);
            let (client, server) = (&mut dialed.client.stream, &mut dialed.accepted.stream);
            assert_eq!(carry(client, server, b"ping").await, b"ping");
            assert_eq!(carry(server, client, b"pong").await, b"pong");
            dialed.nodes.destroy().await;
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn reports_remote_addr_on_both_sides() {
            let dialed = dial_a_listener().await;
            let dialed_addr = dialed.client.remote_addr();
            let accepted_addr = dialed.accepted.remote_addr();
            let expected = Some(SocketAddr::V4(dialed.addr));
            dialed.nodes.destroy().await;
            assert_eq!(dialed_addr, expected, "client: the address it dialed");
            let from = accepted_addr.expect("server: the address the handshake came from");
            assert!(from.ip().is_loopback(), "server sees {from}");
            assert_ne!(from.port(), 0, "server sees {from}");
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn does_not_wait_for_the_node_bootstrap() {
            let nodes = Nodes::start().await;
            let (_server, mut incoming, addr, key) = listening(&nodes.server, SERVER_SEED).await;
            let fresh = isolated_node().await;
            let started = Instant::now();
            let dial = fresh.connect_direct(key.public_key, addr);
            let connected = within(DIAL_WAIT, "connect_direct", dial).await;
            let elapsed = started.elapsed();
            let _accepted = accept_one(&mut incoming, "accept").await;
            let _ = fresh.destroy().await;
            nodes.destroy().await;
            assert!(connected.is_ok(), "got {connected:?}");
            assert!(elapsed < NO_BOOTSTRAP_WAIT, "took {elapsed:?}");
        }
    }

    mod to_an_address_nobody_listens_on {
        use super::*;

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn fails_within_the_request_timeout() {
            let client = isolated_node().await;
            let dead = closed_udp_addr();
            let dial = client.connect_direct(key_pair(0xd4).public_key, dead);
            let refused = within(DIAL_WAIT, "connect_direct to a dead address", dial).await;
            let _ = client.destroy().await;
            assert!(refused.is_err(), "nobody answers at {dead}");
            assert!(
                !matches!(refused, Err(DhtError::Destroyed)),
                "got {refused:?}"
            );
        }
    }

    mod with_a_key_the_server_does_not_hold {
        use super::*;

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn fails_and_delivers_nothing() {
            let nodes = Nodes::start().await;
            let (_server, mut incoming, addr, _key) = listening(&nodes.server, 0xd2).await;
            let foreign = key_pair(0xd3).public_key;
            let dial = nodes.client.connect_direct(foreign, addr);
            let refused = within(DIAL_WAIT, "connect_direct with a foreign key", dial).await;
            let delivered = tokio::time::timeout(Duration::from_secs(1), incoming.accept()).await;
            nodes.destroy().await;
            assert!(refused.is_err(), "the server holds no such key");
            assert!(delivered.is_err(), "the server got a connection");
        }
    }

    mod on_a_destroyed_node {
        use super::*;

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn fails_with_node_destroyed() {
            let node = destroyed_node().await;
            let addr = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 9);
            let dial = node.connect_direct([7; 32], addr);
            let refused = within(DIAL_WAIT, "connect_direct", dial).await;
            assert!(
                matches!(refused, Err(DhtError::Destroyed)),
                "got {refused:?}"
            );
        }
    }

    mod on_a_suspended_node {
        use super::*;

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn fails_with_suspended() {
            let node = isolated_node().await;
            node.suspend().await.expect("suspend");
            let addr = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 9);
            let dial = node.connect_direct([7; 32], addr);
            let refused = within(DIAL_WAIT, "connect_direct", dial).await;
            let _ = node.destroy().await;
            match refused {
                Err(DhtError::Backend(text)) => assert_eq!(text, "SUSPENDED: Suspended"),
                other => panic!("expected a backend error, got {other:?}"),
            }
        }
    }
}

mod node_mode {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn isolated_node_is_not_firewalled() {
        let node = isolated_node().await;
        let firewalled = node.node_mode().firewalled;
        let _ = node.destroy().await;
        assert!(!firewalled, "a loopback-only node is reachable");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn mode_changed_resolves_on_a_firewalled_only_difference() {
        let node = isolated_node().await;
        let mode = node.node_mode();
        let last = NodeMode {
            firewalled: !mode.firewalled,
            ..mode
        };
        let change = node.mode_changed(last);
        let changed = within(Duration::from_secs(1), "mode_changed", change).await;
        let _ = node.destroy().await;
        assert_eq!(changed.expect("mode_changed"), mode);
    }
}

mod live {
    use super::support::start_testnet;
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "needs the JS testnet: node and .tmp/npm-holesail"]
    async fn connect_by_key_reports_remote_addr_on_both_sides() {
        let testnet = start_testnet();
        let mut server = listen_on(&testnet.factory, key_pair(0xd5), None).await;
        let client = node(&testnet.factory, 0xd6).await;
        let client_pk = key_pair(0xd6).public_key;
        let (dialed, accepted) = connect_pair(&client, client_pk, &mut server, "by key").await;
        assert!(dialed.remote_addr().is_some(), "client side has no address");
        assert!(
            accepted.remote_addr().is_some(),
            "server side has no address"
        );
        let (mut client_end, mut server_end) = (dialed, accepted);
        let sent = carry(&mut client_end.stream, &mut server_end.stream, b"ping").await;
        assert_eq!(sent, b"ping");
    }
}

/// Two nodes without peers: one to listen, one to dial.
struct Nodes {
    server: PeeroxideDht,
    client: PeeroxideDht,
}

impl Nodes {
    async fn start() -> Nodes {
        Nodes {
            server: isolated_node().await,
            client: isolated_node().await,
        }
    }

    async fn destroy(&self) {
        let _ = self.client.destroy().await;
        let _ = self.server.destroy().await;
    }
}

/// A client connected by `connect_direct` to a listener on the other node, and the server's end of that connection.
struct Dialed {
    nodes: Nodes,
    client: DhtConnection,
    accepted: DhtConnection,
    addr: SocketAddrV4,
    server_key: [u8; 32],
    _server: PeeroxideServer,
}

async fn dial_a_listener() -> Dialed {
    let nodes = Nodes::start().await;
    let (server, mut incoming, addr, key) = listening(&nodes.server, SERVER_SEED).await;
    let dial = nodes.client.connect_direct(key.public_key, addr);
    let dial = within(DIAL_WAIT, "connect_direct", dial);
    let (client, accepted) = tokio::join!(dial, accept_one(&mut incoming, "accept"));
    Dialed {
        nodes,
        client: client.expect("connect_direct to a listening server"),
        accepted,
        addr,
        server_key: key.public_key,
        _server: server,
    }
}

/// A server of `node` listening on the pair of `seed`, and the address a peer dials it at: loopback and the DHT socket port.
async fn listening(
    node: &PeeroxideDht,
    seed: u8,
) -> (PeeroxideServer, Incoming, SocketAddrV4, KeyPair) {
    let (server, incoming) = node
        .create_server(ServerOpts::default())
        .expect("create_server");
    let key = key_pair(seed);
    let listen = server.listen(key.clone());
    within(LISTEN_WAIT, "listen", listen).await.expect("listen");
    let port = node.local_port().await.expect("local_port");
    let addr = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
    (server, incoming, addr, key)
}

async fn destroyed_node() -> PeeroxideDht {
    let node = isolated_node().await;
    node.destroy().await.expect("destroy");
    node
}

/// A loopback UDP address that is closed now.
fn closed_udp_addr() -> SocketAddrV4 {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("bind");
    let port = socket.local_addr().expect("local_addr").port();
    SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)
}

/// `bytes` go out of `from` (write and flush) and are read at `to` for exactly their length.
async fn carry(from: &mut Box<dyn Duplex>, to: &mut Box<dyn Duplex>, bytes: &[u8]) -> Vec<u8> {
    let write = async {
        from.write_all(bytes).await.expect("write_all");
        from.flush().await.expect("flush");
    };
    let read = async {
        let mut got = vec![0; bytes.len()];
        to.read_exact(&mut got).await.expect("read_exact");
        got
    };
    within(DIAL_WAIT, "byte transfer", async {
        tokio::join!(write, read).1
    })
    .await
}
