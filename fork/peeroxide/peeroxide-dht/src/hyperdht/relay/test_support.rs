//! Test cluster for the relay client: a bootstrap node, a blind-relay node and two ephemeral peers.
// JS: blind-relay/index.js

use std::time::{Duration, Instant};

use libudx::UdxRuntime;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;

use crate::blind_relay::BlindRelayServer;
use crate::hyperdht::{HyperDhtConfig, HyperDhtError, HyperDhtHandle, KeyPair, ServerEvent, spawn};
use crate::relay_service::{RelayServiceConfig, RelayServiceHandle, run_relay_server};
use crate::rpc::DhtConfig;

/// One DHT node on 127.0.0.1 with its own UDX runtime.
pub(super) struct TestPeer {
    pub rt: UdxRuntime,
    pub handle: HyperDhtHandle,
    pub key_pair: KeyPair,
    pub events: Option<UnboundedReceiver<ServerEvent>>,
    _task: JoinHandle<Result<(), HyperDhtError>>,
}

/// Two peers that can reach the relay `relay_pk`; `relay` reads the relay's counters.
pub(super) struct TestNet {
    pub a: TestPeer,
    pub b: TestPeer,
    pub relay_pk: [u8; 32],
    pub relay: BlindRelayServer,
    _boot: Vec<TestPeer>,
    _relay_node: TestPeer,
    _relay_service: RelayServiceHandle,
}

async fn node(bootstrap: Vec<String>, ephemeral: bool) -> TestPeer {
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
    TestPeer {
        rt,
        handle,
        key_pair: KeyPair::from_seed(rand::random::<[u8; 32]>()),
        events: Some(events),
        _task: task,
    }
}

/// Bring up the cluster (four routable nodes keep the announced record) and wait until the relay's record is visible to peer `a`.
pub(super) async fn net() -> TestNet {
    let boot = node(vec![], false).await;
    let port = boot.handle.local_port().await.expect("port");
    let bootstrap = vec![format!("127.0.0.1:{port}")];
    let mut holders = vec![boot];
    for _ in 0..3 {
        holders.push(node(bootstrap.clone(), false).await);
    }
    let mut relay_node = node(bootstrap.clone(), false).await;
    let events = relay_node.events.take().expect("events");
    let (relay, relay_service) = run_relay_server(
        relay_node.rt.handle(),
        relay_node.handle.clone(),
        relay_node.key_pair.clone(),
        events,
        RelayServiceConfig::default(),
    );
    let a = node(bootstrap.clone(), true).await;
    let b = node(bootstrap, true).await;
    let relay_pk = relay_node.key_pair.public_key;
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
    TestNet {
        a,
        b,
        relay_pk,
        relay,
        _boot: holders,
        _relay_node: relay_node,
        _relay_service: relay_service,
    }
}
