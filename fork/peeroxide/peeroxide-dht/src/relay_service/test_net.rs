//! Test cluster for the relay service: a bootstrap node with three routable holders, the relay's own node
//! running [`run_relay_server`] with a caller-chosen config, and two ephemeral peers `a` and `b`.
//!
//! A copy of the `node()` helper of `hyperdht::relay::test_support` (that module is private to
//! `hyperdht::relay`), with the service config as a parameter and the service handle exposed.
// JS: blind-relay/index.js

use std::time::{Duration, Instant};

use libudx::UdxRuntime;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;

use crate::blind_relay::BlindRelayServer;
use crate::hyperdht::{
    HyperDhtConfig, HyperDhtError, HyperDhtHandle, KeyPair, RelayPaired, ServerEvent,
    pair_via_relay, spawn,
};
use crate::hyperdht_messages::FIREWALL_CONSISTENT;
use crate::messages::Ipv4Peer;
use crate::relay_service::{RelayServiceConfig, RelayServiceHandle, run_relay_server};
use crate::rpc::DhtConfig;

/// One DHT node on 127.0.0.1 with its own UDX runtime.
pub(crate) struct RelayPeer {
    pub rt: UdxRuntime,
    pub handle: HyperDhtHandle,
    pub key_pair: KeyPair,
    pub events: Option<UnboundedReceiver<ServerEvent>>,
    _task: JoinHandle<Result<(), HyperDhtError>>,
}

/// Two peers that can reach the relay `relay_pk` (also by `relay_addr`); `relay` reads the relay's counters.
/// `service` is the relay's stop handle; a test may `take()` it to close or drop it.
pub(crate) struct RelayNet {
    pub a: RelayPeer,
    pub b: RelayPeer,
    pub relay_pk: [u8; 32],
    pub relay_addr: Ipv4Peer,
    pub relay: BlindRelayServer,
    pub service: Option<RelayServiceHandle>,
    _boot: Vec<RelayPeer>,
    _relay_node: RelayPeer,
}

/// Which nodes of the cluster sit behind a (consistent) NAT: the relay's node, the two peers, or neither.
#[derive(Clone, Copy, Default)]
pub(crate) struct NetShape {
    pub relay_firewalled: bool,
    pub peers_firewalled: bool,
}

async fn node(bootstrap: Vec<String>, ephemeral: bool) -> RelayPeer {
    node_with(bootstrap, ephemeral, false).await
}

async fn node_with(bootstrap: Vec<String>, ephemeral: bool, firewalled: bool) -> RelayPeer {
    let rt = UdxRuntime::new().expect("udx runtime");
    let dht = DhtConfig {
        bootstrap,
        host: "127.0.0.1".to_string(),
        firewalled,
        ephemeral: Some(ephemeral),
        ..DhtConfig::default()
    };
    let cfg = HyperDhtConfig {
        dht,
        ..HyperDhtConfig::default()
    };
    let (task, handle, events) = spawn(&rt, cfg).await.expect("spawn node");
    handle.bootstrapped().await.expect("bootstrapped");
    RelayPeer {
        rt,
        handle,
        key_pair: KeyPair::from_seed(rand::random::<[u8; 32]>()),
        events: Some(events),
        _task: task,
    }
}

/// Bring up the cluster (four routable nodes keep the announced record) with the relay running `config`.
/// When `config.announce` is set, wait until the relay's record is visible to peer `a`; otherwise return at once.
pub(crate) async fn relay_net(config: RelayServiceConfig) -> RelayNet {
    relay_net_shaped(config, NetShape::default()).await
}

/// As [`relay_net`], with the relay's node and/or the peers firewalled as `shape` says.
/// A firewalled relay node is waited for until it knows it is `FIREWALL_CONSISTENT` and has announced.
pub(crate) async fn relay_net_shaped(config: RelayServiceConfig, shape: NetShape) -> RelayNet {
    let announce = config.announce;
    let boot = node(vec![], false).await;
    let port = boot.handle.local_port().await.expect("port");
    let bootstrap = vec![format!("127.0.0.1:{port}")];
    let mut holders = vec![boot];
    for _ in 0..3 {
        holders.push(node(bootstrap.clone(), false).await);
    }
    let mut relay_node = node_with(bootstrap.clone(), false, shape.relay_firewalled).await;
    let events = relay_node.events.take().expect("events");
    let relay_port = relay_node.handle.local_port().await.expect("relay port");
    let (relay, service) = run_relay_server(
        relay_node.rt.handle(),
        relay_node.handle.clone(),
        relay_node.key_pair.clone(),
        events,
        config,
    );
    let a = node_with(bootstrap.clone(), true, shape.peers_firewalled).await;
    let b = node_with(bootstrap, true, shape.peers_firewalled).await;
    let relay_pk = relay_node.key_pair.public_key;
    if shape.relay_firewalled {
        wait_firewalled_and_announced(&relay_node.handle).await;
    }
    if announce {
        let start = Instant::now();
        while !matches!(
            a.handle.find_peer(crate::crypto::hash(&relay_pk)).await,
            Ok(Some(_))
        ) {
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "relay not announced"
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
    RelayNet {
        a,
        b,
        relay_pk,
        relay_addr: Ipv4Peer {
            host: "127.0.0.1".to_string(),
            port: relay_port,
        },
        relay,
        service: Some(service),
        _boot: holders,
        _relay_node: relay_node,
    }
}

/// Wait (10 s) until the node's firewall is consistent and (20 s) until it announced through some relays.
async fn wait_firewalled_and_announced(handle: &HyperDhtHandle) {
    let start = Instant::now();
    while handle.firewalled().await.ok() != Some(FIREWALL_CONSISTENT) {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "the relay node never became FIREWALL_CONSISTENT"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let start = Instant::now();
    while handle.current_relay_addresses().is_empty() {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "the firewalled relay node never got its announce accepted"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Who {
    A,
    B,
}

/// Pair one token through the relay; gives up after 20 s.
pub(crate) async fn pair(
    n: &RelayNet,
    who: Who,
    token: [u8; 32],
    initiator: bool,
    id: u32,
) -> Result<RelayPaired, HyperDhtError> {
    let p = match who {
        Who::A => &n.a,
        Who::B => &n.b,
    };
    let paired = pair_via_relay(
        &p.handle,
        &p.key_pair,
        &p.rt,
        n.relay_pk,
        &[],
        token,
        initiator,
        id,
        Duration::from_secs(5),
    );
    tokio::time::timeout(Duration::from_secs(20), paired)
        .await
        .unwrap_or(Err(HyperDhtError::RelayAborted))
}

/// Run a stream over each paired socket (`a_id`/`b_id` are the local ids given to `pair`), send bytes
/// A to B and B to A through the relay, and check they arrive intact; the relay must have bridged two streams.
pub(crate) async fn echo_over(
    n: &RelayNet,
    a: &RelayPaired,
    a_id: u32,
    b: &RelayPaired,
    b_id: u32,
) {
    let sa = n.a.rt.create_stream(a_id).await.expect("stream a");
    let sb = n.b.rt.create_stream(b_id).await.expect("stream b");
    let (ca, cb) = tokio::join!(
        sa.connect(&a.socket, a.remote_id, a.relay_addr),
        sb.connect(&b.socket, b.remote_id, b.relay_addr)
    );
    ca.expect("connect a");
    cb.expect("connect b");
    // The relay learns a peer's address from its first packet, so both sides speak first.
    let to_b: Vec<u8> = (0..16 * 1024u32).map(|i| (i % 251) as u8).collect();
    let to_a: Vec<u8> = (0..16 * 1024u32).map(|i| (i % 241) as u8).collect();
    let exchange = async {
        let (mut sa, mut sb) = (sa, sb);
        let (wa, wb) = tokio::join!(sa.write(&to_b), sb.write(&to_a));
        wa.expect("write a");
        wb.expect("write b");
        let (got_b, got_a) =
            tokio::join!(read_len(&mut sb, to_b.len()), read_len(&mut sa, to_a.len()));
        (got_b, got_a)
    };
    let (got_b, got_a) = tokio::time::timeout(Duration::from_secs(20), exchange)
        .await
        .expect("exchange timed out");
    assert_eq!(got_b, to_b, "bytes from A reach B");
    assert_eq!(got_a, to_a, "bytes from B reach A");
    assert!(
        n.relay.stats().streams_opened >= 2,
        "the relay bridged the pair"
    );
}

async fn read_len(stream: &mut libudx::UdxStream, len: usize) -> Vec<u8> {
    let mut got = Vec::new();
    while got.len() < len {
        got.extend(stream.read().await.expect("read").expect("stream open"));
    }
    got
}
