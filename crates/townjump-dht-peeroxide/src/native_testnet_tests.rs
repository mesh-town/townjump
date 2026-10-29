// Helpers for end-to-end adapter tests on a native DHT network of the fork, in this process: no node, no `#[ignore]`.
// The nodes sit on 127.0.0.1 and are open, so a client goes straight to an open server and a relay never pairs:
// clients and servers of relay tests come from `firewalled_dht`, and the client is limited to the relay by `relay_only`.
// Relays bind the LAN address (`relay_try`): peers on one host reach a relay by its advertised LAN address.
// Used by relay_admit_tests, relay_restart_tests, relay_reach_tests (and the relay path tests of the next module).
#![allow(dead_code)]
use crate::bootstrap::BootstrapPlan;
use crate::dht::PeeroxideDht;
use crate::factory::PeeroxideFactory;
use crate::node::Node;
use crate::relay::start_relay;
use crate::server::PeeroxideServer;
use crate::{PeeroxideRelay, RelayOpts, RelayStats};
use peeroxide_dht::crypto::hash;
use peeroxide_dht::holepuncher::Holepuncher;
use peeroxide_dht::hyperdht::HyperDhtConfig;
use peeroxide_dht::rpc::DhtConfig;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use townjump_dht::{ConnectOpts, Dht, DhtConnection, DhtError, DhtServer, Incoming, ServerOpts};
use townjump_keys::KeyPair;
use townjump_testkit::conformance::wait::{ACCEPT_WAIT, REFUSE_WAIT, STREAM_WAIT, within};
use townjump_testkit::key_pair;

/// Default network size of relay tests: a relay asks the table nodes and bootstrap, three answers are enough.
pub(super) const DEFAULT_SIZE: usize = 4;
const ANNOUNCE_TRIES: usize = 50;
const POLL_TRIES: usize = 50;
const POLL_STEP: Duration = Duration::from_millis(200);
const PAYLOAD: usize = 4096;

/// A live native network on 127.0.0.1 and a factory pointed at its first node; the nodes go down with the value.
pub(super) struct NativeTestnet {
    pub factory: PeeroxideFactory,
    nodes: Vec<Arc<Node>>,
}

/// Brings up `size` open persistent nodes on 127.0.0.1: the first is the bootstrap, the rest join through it.
pub(super) async fn native_testnet(size: usize) -> NativeTestnet {
    assert!(size >= 1, "a testnet needs at least the bootstrap node");
    let first = start_node(Vec::new()).await;
    let port = first.handle.local_port().await.expect("bootstrap port");
    let bootstrap = vec![format!("127.0.0.1:{port}")];
    let mut nodes = vec![first];
    for _ in 1..size {
        nodes.push(start_node(bootstrap.clone()).await);
    }
    let factory = PeeroxideFactory {
        bootstrap: Some(bootstrap),
        adaptive: false,
        nodes: Vec::new(),
    };
    NativeTestnet { factory, nodes }
}

async fn start_node(bootstrap: Vec<String>) -> Arc<Node> {
    let mut dht = DhtConfig::default();
    dht.bootstrap = bootstrap;
    dht.host = "127.0.0.1".to_string();
    dht.firewalled = false;
    dht.ephemeral = Some(false);
    let mut config = HyperDhtConfig::default();
    config.dht = dht;
    let node = Node::start(config, None).await.expect("testnet node");
    within(ACCEPT_WAIT, "testnet node bootstrap", node.ready())
        .await
        .expect("testnet node ready");
    node
}

/// Limits a client to the relay: no holepunch at all and no LAN shortcut.
pub(super) fn relay_only(opts: &mut ConnectOpts) {
    opts.holepunch = Some(Arc::new(|_| false));
    opts.local_connection = false;
}

impl Drop for NativeTestnet {
    fn drop(&mut self) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        for node in self.nodes.drain(..) {
            runtime.spawn(async move {
                let _ = node.destroy().await;
            });
        }
    }
}

impl NativeTestnet {
    /// Like `PeeroxideFactory::create`, but the node is behind a NAT (`firewalled = true`): it must be punched to
    /// or reached through a relay.
    pub(super) async fn firewalled_dht(&self, identity: Option<KeyPair>) -> PeeroxideDht {
        let plan = BootstrapPlan::parse(self.factory.bootstrap.as_deref()).expect("bootstrap plan");
        let mut config = self.factory.node_config(&plan);
        config.dht.firewalled = true;
        let node = Node::start(config, identity)
            .await
            .expect("firewalled node");
        PeeroxideDht::new(node)
    }

    /// `create_relay_with` under a time limit, on the LAN address (see `relay_try`).
    pub(super) async fn relay_with(&self, opts: RelayOpts) -> PeeroxideRelay {
        let created = self.relay_try(opts);
        let relay = within(ACCEPT_WAIT, "create_relay_with", created).await;
        relay.expect("create_relay_with")
    }

    /// `create_relay_with`, but the relay node binds the first LAN IPv4 address instead of 127.0.0.1.
    /// Peers on one host take the same-host LAN shortcut to the relay (`local_connection` is on for the relay
    /// connection, as in JS) and dial the relay's advertised LAN address: a relay bound to 127.0.0.1 is not there.
    /// In production the relay binds 0.0.0.0 and listens on every address, so only this loopback testnet needs it.
    pub(super) async fn relay_try(&self, opts: RelayOpts) -> Result<PeeroxideRelay, DhtError> {
        let plan = BootstrapPlan::parse(self.factory.bootstrap.as_deref())?;
        let mut config = self.factory.node_config(&plan);
        config.dht.host = lan_host();
        start_relay(config, opts).await
    }

    /// Whether a node of the network (not the relay itself) finds an announcement under the key.
    pub(super) async fn is_announced(&self, key: &[u8; 32]) -> bool {
        let found = self.nodes[0].handle.find_peer(hash(key)).await;
        matches!(found, Ok(Some(_)))
    }

    /// Waits until the network finds the relay by its key; panics after ~10 s.
    pub(super) async fn wait_announced(&self, key: &[u8; 32]) {
        for _ in 0..ANNOUNCE_TRIES {
            if self.is_announced(key).await {
                return;
            }
            tokio::time::sleep(POLL_STEP).await;
        }
        panic!("the relay was never announced");
    }

    /// A firewalled node on the key of `tag` with a server on the same key (the relay sees the node key).
    pub(super) async fn listening(
        &self,
        tag: u8,
        opts: ServerOpts,
    ) -> (PeeroxideDht, PeeroxideServer, Incoming) {
        let node = self.firewalled_dht(Some(key_pair(tag))).await;
        let (server, incoming) = node.create_server(opts).expect("create_server");
        let listen = within(ACCEPT_WAIT, "listen", server.listen(key_pair(tag)));
        listen.await.expect("listen");
        (node, server, incoming)
    }

    /// A firewalled client on the key of `tag` connects to `remote`; the node must outlive the connection.
    pub(super) async fn connect_from(
        &self,
        tag: u8,
        remote: [u8; 32],
        opts: ConnectOpts,
    ) -> (PeeroxideDht, Result<DhtConnection, DhtError>) {
        let node = self.firewalled_dht(Some(key_pair(tag))).await;
        let result = within(REFUSE_WAIT, "connect", node.connect(remote, opts)).await;
        (node, result)
    }

    /// One whole exchange: a server on `server_tag` echoes, a client on `client_tag` goes to it with `client`
    /// options (the caller sets `relay_through`; the client is limited to the relay) and reads its payload back.
    pub(super) async fn echo_via(
        &self,
        server_tag: u8,
        client_tag: u8,
        server: ServerOpts,
        mut client: ConnectOpts,
    ) {
        let (_node, _server, incoming) = self.listening(server_tag, server).await;
        echo_first(incoming);
        relay_only(&mut client);
        let remote = key_pair(server_tag).public_key;
        let (_client_node, conn) = self.connect_from(client_tag, remote, client).await;
        assert_echo(conn.expect("connect through the relay")).await;
    }
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

/// Writes a payload and reads the same bytes back.
pub(super) async fn assert_echo(mut conn: DhtConnection) {
    let sent: Vec<u8> = (0..PAYLOAD).map(|i| (i % 251) as u8).collect();
    conn.stream.write_all(&sent).await.expect("write");
    let mut echoed = vec![0u8; PAYLOAD];
    let read = within(STREAM_WAIT, "echo", conn.stream.read_exact(&mut echoed));
    read.await.expect("read of the echo");
    assert_eq!(sent, echoed, "echo differs from what was written");
}

/// Polls the relay counters until `done` holds or ~10 s pass; returns the last snapshot.
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

/// The first non-loopback IPv4 address of this machine; panics when there is none.
pub(super) fn lan_host() -> String {
    Holepuncher::local_addresses(0)
        .into_iter()
        .map(|a| a.host)
        .find(|host| !host.starts_with("127."))
        .expect("relay tests need a non-loopback IPv4 interface")
}

/// A UDP port that is free right now on the LAN address of the relays (taken, then released).
pub(super) fn free_udp_port() -> u16 {
    let socket = std::net::UdpSocket::bind((lan_host().as_str(), 0)).expect("bind a free port");
    socket.local_addr().expect("local address").port()
}
