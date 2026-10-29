//! Local cluster helpers for the `hyperdht_connect_*` tests: nodes on 127.0.0.1 only.
#![allow(dead_code)]
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use libudx::UdxRuntime;
use peeroxide_dht::blind_relay::BlindRelayServer;
use peeroxide_dht::hyperdht::{
    HyperDhtConfig, HyperDhtError, HyperDhtHandle, KeyPair, ServerEvent, spawn,
};
use peeroxide_dht::relay_service::{RelayServiceConfig, RelayServiceHandle, run_relay_server};
use peeroxide_dht::rpc::DhtConfig;
use peeroxide_dht::server::{Firewall, Server, ServerOpts};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;

pub struct Node {
    pub rt: UdxRuntime,
    pub handle: HyperDhtHandle,
    pub events: UnboundedReceiver<ServerEvent>,
    _task: JoinHandle<Result<(), HyperDhtError>>,
}

pub async fn spawn_node(bootstrap: Vec<String>, ephemeral: bool) -> Node {
    spawn_node_fw(bootstrap, ephemeral, false).await
}

/// Like `spawn_node`; `firewalled` makes the node behave as one behind a NAT (it must be punched to).
pub async fn spawn_node_fw(bootstrap: Vec<String>, ephemeral: bool, firewalled: bool) -> Node {
    spawn_node_on(bootstrap, "127.0.0.1", ephemeral, firewalled).await
}

/// The first non-loopback IPv4 address of this machine; panics when there is none.
pub fn lan_host() -> String {
    peeroxide_dht::holepuncher::Holepuncher::local_addresses(0)
        .into_iter()
        .map(|a| a.host)
        .find(|host| !host.starts_with("127."))
        .expect("the test needs a non-loopback IPv4 interface")
}

/// A node bound to `host` (a relay must be reachable under the name clients see it by).
pub async fn spawn_node_on(
    bootstrap: Vec<String>,
    host: &str,
    ephemeral: bool,
    firewalled: bool,
) -> Node {
    let mut dht = DhtConfig::default();
    dht.bootstrap = bootstrap;
    dht.host = host.to_string();
    dht.firewalled = firewalled;
    dht.ephemeral = Some(ephemeral);
    spawn_config(dht).await
}

/// A node on 127.0.0.1 that decides its own mode (`ephemeral: None`, adaptive on): it starts
/// ephemeral and promotes itself after `stable_ticks` quiet ticks.
pub async fn spawn_adaptive(bootstrap: Vec<String>, firewalled: bool, stable_ticks: u64) -> Node {
    let mut dht = DhtConfig::default();
    dht.bootstrap = bootstrap;
    dht.host = "127.0.0.1".to_string();
    dht.firewalled = firewalled;
    dht.ephemeral = None;
    dht.adaptive = true;
    dht.stable_ticks = stable_ticks;
    spawn_config(dht).await
}

/// Spawns a node with exactly this DHT config and waits until it is bootstrapped.
pub async fn spawn_config(dht: DhtConfig) -> Node {
    let rt = UdxRuntime::new().expect("udx runtime");
    let mut cfg = HyperDhtConfig::default();
    cfg.dht = dht;
    let (task, handle, events) = spawn(&rt, cfg).await.expect("spawn node");
    handle.bootstrapped().await.expect("bootstrapped");
    Node {
        rt,
        handle,
        events,
        _task: task,
    }
}

/// `size` routable nodes (each keeps the announced record, so each one is a holder); the first
/// is the bootstrap of the others.
pub async fn cluster(size: usize) -> (Vec<Node>, Vec<String>) {
    let first = spawn_node(vec![], false).await;
    let port = first.handle.local_port().await.expect("port");
    let bootstrap = vec![format!("127.0.0.1:{port}")];
    let mut nodes = vec![first];
    for _ in 1..size {
        nodes.push(spawn_node(bootstrap.clone(), false).await);
    }
    (nodes, bootstrap)
}

pub async fn within<F: Future>(test: F) -> F::Output {
    let limit = Duration::from_secs(60);
    tokio::time::timeout(limit, test)
        .await
        .expect("test timed out after 60 s")
}

/// An ephemeral node that serves `key`; `refuse` makes the server drop every handshake
/// (`firewall` callback returns true), as `firewall: () => true` in JS. Keep the result alive.
pub struct Served {
    pub server: Server,
    pub node: Node,
    pub incoming: tokio::sync::mpsc::Receiver<peeroxide_dht::hyperdht::PeerConnection>,
}

pub async fn serve_key(bootstrap: &[String], key: &KeyPair, refuse: bool) -> Served {
    serve_key_fw(bootstrap, key, refuse, false).await
}

/// `serve_key` on a firewalled node: clients have to hole punch (no direct dial).
pub async fn serve_key_fw(
    bootstrap: &[String],
    key: &KeyPair,
    refuse: bool,
    firewalled: bool,
) -> Served {
    let firewall: Firewall = Arc::new(move |_input| refuse);
    let opts = ServerOpts::default().with_firewall(firewall);
    serve_key_opts(bootstrap, key, firewalled, opts).await
}

/// `serve_key_fw` with caller-made server options (e.g. `relay_through`).
pub async fn serve_key_opts(
    bootstrap: &[String],
    key: &KeyPair,
    firewalled: bool,
    opts: ServerOpts,
) -> Served {
    let mut node = spawn_node_fw(bootstrap.to_vec(), true, firewalled).await;
    let rt = UdxRuntime::shared(node.rt.handle());
    let events = std::mem::replace(&mut node.events, tokio::sync::mpsc::unbounded_channel().1);
    let (server, incoming) = Server::new(node.handle.clone(), events, rt, opts);
    server.listen(key.clone()).await.expect("listen");
    Served {
        server,
        node,
        incoming,
    }
}

/// A routable node running `run_relay_server`; `server` reads the relay counters. The record
/// of `public_key` is announced by the time the node answers, other nodes may need a moment.
pub struct Relay {
    pub node: Node,
    pub server: BlindRelayServer,
    pub public_key: [u8; 32],
    pub service: RelayServiceHandle,
}

pub async fn spawn_relay(bootstrap: &[String]) -> Relay {
    let mut node = spawn_node_on(bootstrap.to_vec(), &lan_host(), false, false).await;
    let key_pair = KeyPair::from_seed(rand::random::<[u8; 32]>());
    let events = std::mem::replace(&mut node.events, tokio::sync::mpsc::unbounded_channel().1);
    let (server, service) = run_relay_server(
        node.rt.handle(),
        node.handle.clone(),
        key_pair.clone(),
        events,
        RelayServiceConfig::default(),
    );
    Relay {
        node,
        server,
        public_key: key_pair.public_key,
        service,
    }
}
