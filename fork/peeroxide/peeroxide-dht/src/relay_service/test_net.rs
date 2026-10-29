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
use crate::hyperdht::{HyperDhtConfig, HyperDhtError, HyperDhtHandle, KeyPair, ServerEvent, spawn};
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

async fn node(bootstrap: Vec<String>, ephemeral: bool) -> RelayPeer {
    let rt = UdxRuntime::new().expect("udx runtime");
    let dht = DhtConfig {
        bootstrap,
        host: "127.0.0.1".to_string(),
        firewalled: false,
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
    let announce = config.announce;
    let boot = node(vec![], false).await;
    let port = boot.handle.local_port().await.expect("port");
    let bootstrap = vec![format!("127.0.0.1:{port}")];
    let mut holders = vec![boot];
    for _ in 0..3 {
        holders.push(node(bootstrap.clone(), false).await);
    }
    let mut relay_node = node(bootstrap.clone(), false).await;
    let events = relay_node.events.take().expect("events");
    let relay_port = relay_node.handle.local_port().await.expect("relay port");
    let (relay, service) = run_relay_server(
        relay_node.rt.handle(),
        relay_node.handle.clone(),
        relay_node.key_pair.clone(),
        events,
        config,
    );
    let a = node(bootstrap.clone(), true).await;
    let b = node(bootstrap, true).await;
    let relay_pk = relay_node.key_pair.public_key;
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
