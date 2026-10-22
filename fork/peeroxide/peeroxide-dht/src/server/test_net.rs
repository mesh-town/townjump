// JS: hyperdht/lib/server.js
//! Test network (`cfg(test)`): a local DHT cluster on 127.0.0.1, fixed keys, a raw
//! client handshake and an echo peer. No internet, no JS.

use std::future::Future;
use std::time::Duration;

use libudx::UdxRuntime;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::{Server, ServerOpts};
use crate::crypto::hash;
use crate::holepuncher::Holepuncher;
use crate::hyperdht::{
    self, HyperDhtConfig, HyperDhtError, HyperDhtHandle, KeyPair, PeerConnection, ServerEvent,
};
use crate::hyperdht_messages::{
    NoisePayload, PEER_HANDSHAKE, RelayThroughInfo, SecretStreamInfo, UdxInfo,
};
use crate::messages::Ipv4Peer;
use crate::noise_wrap::NoiseWrap;
use crate::router::{HandshakeResult, Router};
use crate::rpc::{DhtConfig, UserRequestParams};

/// One DHT node with its own UDX runtime; `events` feeds a `Server`.
pub(super) struct Node {
    pub(super) rt: UdxRuntime,
    pub(super) handle: HyperDhtHandle,
    pub(super) events: mpsc::UnboundedReceiver<ServerEvent>,
    _task: JoinHandle<Result<(), HyperDhtError>>,
}

/// Nodes that are neither firewalled nor ephemeral (they join each other's routing
/// tables), so an ANNOUNCE reaches every one of them and each keeps a relay record.
pub(super) struct Cluster {
    /// The cluster nodes; `nodes[i]` listens at `addrs[i]`.
    pub(super) nodes: Vec<Node>,
    /// Addresses of the cluster nodes (`127.0.0.1:port`).
    pub(super) addrs: Vec<Ipv4Peer>,
}

/// A started `Server` together with the node it runs on.
pub(super) struct Served {
    pub(super) server: Server,
    pub(super) incoming: mpsc::Receiver<PeerConnection>,
    pub(super) handle: HyperDhtHandle,
    _rt: UdxRuntime,
    _task: JoinHandle<Result<(), HyperDhtError>>,
}

fn peer(port: u16) -> Ipv4Peer {
    Ipv4Peer {
        host: "127.0.0.1".into(),
        port,
    }
}

async fn spawn_node(bootstrap: Vec<String>, firewalled: bool, ephemeral: bool) -> Node {
    spawn_node_on(bootstrap, "127.0.0.1", firewalled, ephemeral).await
}

/// As `spawn_node`, bound to `host` (the node's source address as the relays see it).
async fn spawn_node_on(
    bootstrap: Vec<String>,
    host: &str,
    firewalled: bool,
    ephemeral: bool,
) -> Node {
    let rt = UdxRuntime::new().expect("udx runtime");
    let dht = DhtConfig {
        bootstrap,
        host: host.into(),
        port: 0,
        firewalled,
        ephemeral: Some(ephemeral),
        ..DhtConfig::default()
    };
    let cfg = HyperDhtConfig {
        dht,
        ..HyperDhtConfig::default()
    };
    let (task, handle, events) = hyperdht::spawn(&rt, cfg).await.expect("spawn node");
    handle.bootstrapped().await.expect("bootstrapped");
    Node {
        rt,
        handle,
        events,
        _task: task,
    }
}

/// A cluster of `n >= 2` nodes; the first one is the bootstrap of the rest.
pub(super) async fn cluster(n: usize) -> Cluster {
    assert!(n >= 2, "a cluster needs at least 2 nodes");
    let first = spawn_node(vec![], false, false).await;
    let first_port = first.handle.local_port().await.expect("port");
    let bootstrap = vec![format!("127.0.0.1:{first_port}")];
    let mut addrs = vec![peer(first_port)];
    let mut nodes = vec![first];
    for _ in 1..n {
        let node = spawn_node(bootstrap.clone(), false, false).await;
        addrs.push(peer(node.handle.local_port().await.expect("port")));
        nodes.push(node);
    }
    Cluster { nodes, addrs }
}

/// A routable node that joins the cluster, bound to `host` (a relay node: it must keep its
/// announcement, and clients on this machine reach it by a name other than loopback).
pub(super) async fn member_on(cluster: &Cluster, host: &str) -> Node {
    let port = cluster.addrs[0].port;
    spawn_node_on(vec![format!("127.0.0.1:{port}")], host, false, false).await
}

/// A node outside the cluster list that bootstraps through the cluster.
pub(super) async fn node(cluster: &Cluster, firewalled: bool) -> Node {
    let port = cluster.addrs[0].port;
    spawn_node(vec![format!("127.0.0.1:{port}")], firewalled, true).await
}

/// As [`node`], bound to `host` instead of `127.0.0.1` (a LAN address of this machine, see
/// [`lan_host`]): the cluster sees the node coming from another host than its own.
pub(super) async fn node_on(cluster: &Cluster, host: &str, firewalled: bool) -> Node {
    let port = cluster.addrs[0].port;
    spawn_node_on(vec![format!("127.0.0.1:{port}")], host, firewalled, true).await
}

/// The first non-loopback IPv4 address of this machine; panics when there is none.
pub(super) fn lan_host() -> String {
    Holepuncher::local_addresses(0)
        .into_iter()
        .map(|a| a.host)
        .find(|host| !host.starts_with("127."))
        .expect("the test needs a non-loopback IPv4 interface")
}

/// Deterministic key pair: seed `[tag; 32]`.
pub(super) fn key(tag: u8) -> KeyPair {
    KeyPair::from_seed([tag; 32])
}

/// Starts a `Server` on `node` (no `listen` yet).
pub(super) fn start(node: Node, opts: ServerOpts) -> Served {
    let rt = UdxRuntime::shared(node.rt.handle());
    let (server, incoming) = Server::new(node.handle.clone(), node.events, rt, opts);
    Served {
        server,
        incoming,
        handle: node.handle,
        _rt: node.rt,
        _task: node._task,
    }
}

/// Starts a `Server` on `node` and listens on `key`.
pub(super) async fn serve(node: Node, key: &KeyPair, opts: ServerOpts) -> Served {
    let served = start(node, opts);
    served.server.listen(key.clone()).await.expect("listen");
    served
}

/// The client's first Noise message (initiator side) and the state to read the reply.
pub(super) fn hello(
    client: &KeyPair,
    server_pk: [u8; 32],
    firewall: u64,
    udx_id: Option<u64>,
) -> (NoiseWrap, Vec<u8>) {
    hello_with(client, server_pk, firewall, udx_id, vec![])
}

/// As [`hello`], announcing `addresses4` (what a real client puts into its first message).
pub(super) fn hello_with(
    client: &KeyPair,
    server_pk: [u8; 32],
    firewall: u64,
    udx_id: Option<u64>,
    addresses4: Vec<Ipv4Peer>,
) -> (NoiseWrap, Vec<u8>) {
    hello_relay(client, server_pk, firewall, udx_id, addresses4, None)
}

/// As [`hello_with`], with the relay the client asks for in its payload.
pub(super) fn hello_relay(
    client: &KeyPair,
    server_pk: [u8; 32],
    firewall: u64,
    udx_id: Option<u64>,
    addresses4: Vec<Ipv4Peer>,
    relay_through: Option<RelayThroughInfo>,
) -> (NoiseWrap, Vec<u8>) {
    let mut nw = NoiseWrap::new_initiator(client.to_noise_keypair(), server_pk);
    let payload = NoisePayload {
        version: 1,
        error: 0,
        firewall,
        holepunch: None,
        addresses4,
        addresses6: vec![],
        udx: udx_id.map(|id| UdxInfo {
            version: 1,
            reusable_socket: false,
            id,
            seq: 0,
        }),
        secret_stream: Some(SecretStreamInfo { version: 1 }),
        relay_through,
        relay_addresses: None,
    };
    let noise = nw.send(&payload).expect("noise send");
    (nw, noise)
}

/// Sends PEER_HANDSHAKE for `hash(server_pk)` to `to` (a cluster node = via relay,
/// the server's own port = direct). `None` = error, timeout or an empty reply.
pub(super) async fn raw_handshake(
    client: &Node,
    server_pk: [u8; 32],
    noise: &[u8],
    to: &Ipv4Peer,
) -> Option<HandshakeResult> {
    raw_handshake_via(client, server_pk, noise, to, None).await
}

/// As [`raw_handshake`], with `relay_address` in the message: `to` forwards it to that
/// node (the second relay), which hands it to the server as FROM_SECOND_RELAY.
pub(super) async fn raw_handshake_via(
    client: &Node,
    server_pk: [u8; 32],
    noise: &[u8],
    to: &Ipv4Peer,
    relay_address: Option<Ipv4Peer>,
) -> Option<HandshakeResult> {
    let value = Router::encode_client_handshake(noise.to_vec(), None, relay_address).ok()?;
    let params = UserRequestParams {
        command: PEER_HANDSHAKE,
        target: Some(hash(&server_pk)),
        value: Some(value),
        timeout_ms: Some(2000),
        retries: Some(0),
        ..UserRequestParams::default()
    };
    let resp = client.handle.dht().request(params, &to.host, to.port).await;
    let resp = resp.ok()?;
    let router = client.handle.router().lock().ok()?;
    router
        .validate_handshake_reply(&resp.value?, to, &resp.from)
        .ok()
}

/// Echo server: writes back everything it reads until the peer shuts down.
pub(super) async fn echo(mut conn: PeerConnection) {
    while let Ok(Some(data)) = conn.stream.read().await {
        if conn.stream.write(&data).await.is_err() {
            return;
        }
    }
    let _ = conn.stream.shutdown().await;
}

/// Sends `bytes` in 64 KiB pieces, shuts down the write side, reads to the end
/// and returns the sha256 of everything received.
pub(super) async fn roundtrip(conn: PeerConnection, bytes: Vec<u8>) -> [u8; 32] {
    let (mut reader, mut writer) = conn.stream.into_split();
    let sender = tokio::spawn(async move {
        for chunk in bytes.chunks(64 * 1024) {
            writer.write(chunk).await.expect("client write");
        }
        writer.shutdown().await.expect("client shutdown");
    });
    let mut digest = Sha256::new();
    while let Some(data) = reader.read().await.expect("client read") {
        digest.update(&data);
    }
    sender.await.expect("sender task");
    digest.finalize().into()
}

/// sha256 of `bytes`.
pub(super) fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// Port of the socket the node's DHT requests leave from (`server_socket`, the
/// address that relays see for this node).
pub(super) async fn primary_port(handle: &HyperDhtHandle) -> u16 {
    let socket = handle.server_socket().await.expect("server_socket");
    let socket = socket.expect("primary socket exists");
    socket.local_addr().await.expect("local_addr").port()
}

/// The node's own address as the relays see it (`remote_address()`), once the sample has settled
/// (polled every 50 ms for 5 s); panics otherwise.
pub(super) async fn settled_remote_address(handle: &HyperDhtHandle) -> Ipv4Peer {
    for _ in 0..100 {
        if let Ok(Some(address)) = handle.remote_address().await {
            return address;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("remote address did not settle in 5 s");
}

/// Runs a cluster test under the 60 s limit; panics on timeout.
pub(super) async fn within<F: Future>(test: F) -> F::Output {
    match tokio::time::timeout(Duration::from_secs(60), test).await {
        Ok(out) => out,
        Err(_) => panic!("cluster test timed out after 60 s"),
    }
}
