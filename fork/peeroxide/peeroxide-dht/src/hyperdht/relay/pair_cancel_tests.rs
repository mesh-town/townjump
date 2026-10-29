//! `pair_via_relay` on a local cluster: what a cancelled pairing leaves behind, and pairing
//! through a relay that is reachable only by its address (no announce in the DHT).
// JS: hyperdht/lib/connect.js:793-838

use std::time::{Duration, Instant};

use libudx::UdxRuntime;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;

use super::{pair_via_relay, registry};
use crate::hyperdht::{HyperDhtConfig, HyperDhtError, HyperDhtHandle, KeyPair, ServerEvent, spawn};
use crate::messages::Ipv4Peer;
use crate::relay_service::{RelayServiceConfig, run_relay_server};
use crate::rpc::DhtConfig;
use crate::server::{Server, ServerOpts};

const KEEP_ALIVE: Duration = Duration::from_secs(5);

mod cancelled_pairing {
    use super::*;

    mod relay_that_never_answers_the_blind_relay_protocol {
        use super::*;

        #[tokio::test(flavor = "multi_thread")]
        async fn mux_task_ends_within_a_second_of_dropping_the_future() {
            // given: an ordinary server as the "relay", nobody reads its incoming connections
            let w = silent_relay().await;
            let id = 0x7101;
            // when: the pairing is dropped after 2 s, having started its mux task
            let started = w.pair_dropped_after(id, Duration::from_secs(2)).await;
            // then: the mux task is gone from the registry within 1 s
            assert!(
                started,
                "the pairing never started a mux task, nothing was tested"
            );
            assert!(
                until(|| !registry::is_live(id), Duration::from_secs(1)).await,
                "mux task of stream {id:#x} outlived the dropped pairing"
            );
        }
    }
}

mod relay_addresses {
    use super::*;

    mod relay_without_announce {
        use super::*;

        #[tokio::test(flavor = "multi_thread")]
        async fn peers_pair_through_the_given_address() {
            // given: a relay nobody can find in the DHT, and its address
            let w = quiet_relay().await;
            assert!(!w.found_in_dht().await, "the relay must not be announced");
            // when: both peers pair with that address
            let (a, b) = w.pair_both(0x7201, 0x7202).await;
            // then: the relay matched them
            assert!(
                a.is_ok() && b.is_ok(),
                "pairing failed: {:?} {:?}",
                a.err(),
                b.err()
            );
            assert!(w.matched() >= 1, "the relay made no pairing");
        }
    }
}

/// One DHT node with its own UDX runtime.
struct Peer {
    rt: UdxRuntime,
    handle: HyperDhtHandle,
    key_pair: KeyPair,
    events: Option<UnboundedReceiver<ServerEvent>>,
    _task: JoinHandle<Result<(), HyperDhtError>>,
}

/// A "relay" that is an ordinary server: it answers the handshake, then says nothing.
struct Silent {
    client: Peer,
    relay_pk: [u8; 32],
    _server: Server,
    _incoming: tokio::sync::mpsc::Receiver<crate::hyperdht::PeerConnection>,
    _rt: UdxRuntime,
    _holders: Vec<Peer>,
}

/// A real relay service without self-announce, and two clients.
struct Quiet {
    a: Peer,
    b: Peer,
    relay_pk: [u8; 32],
    relay_addr: Ipv4Peer,
    stats: crate::blind_relay::BlindRelayServer,
    _service: Box<dyn std::any::Any + Send>,
    _relay_node: Peer,
    _holders: Vec<Peer>,
}

impl Silent {
    /// Runs the pairing for `limit`, then drops it; whether its mux task was seen alive meanwhile.
    async fn pair_dropped_after(&self, id: u32, limit: Duration) -> bool {
        let c = &self.client;
        let pairing = pair_via_relay(
            &c.handle,
            &c.key_pair,
            &c.rt,
            self.relay_pk,
            &[],
            rand::random(),
            true,
            id,
            KEEP_ALIVE,
        );
        let watch =
            async { until(|| registry::is_live(id), limit - Duration::from_millis(200)).await };
        let (res, started) = tokio::join!(tokio::time::timeout(limit, pairing), watch);
        assert!(
            res.is_err(),
            "the pairing must still be waiting at {limit:?}"
        );
        started
    }
}

impl Quiet {
    async fn found_in_dht(&self) -> bool {
        let target = crate::crypto::hash(&self.relay_pk);
        matches!(self.a.handle.find_peer(target).await, Ok(Some(_)))
    }

    async fn pair_both(
        &self,
        ida: u32,
        idb: u32,
    ) -> (
        Result<super::RelayPaired, HyperDhtError>,
        Result<super::RelayPaired, HyperDhtError>,
    ) {
        let token = rand::random::<[u8; 32]>();
        let addrs = [self.relay_addr.clone()];
        async fn pair(
            p: &Peer,
            relay_pk: [u8; 32],
            addrs: &[Ipv4Peer],
            token: [u8; 32],
            initiator: bool,
            id: u32,
        ) -> Result<super::RelayPaired, HyperDhtError> {
            pair_via_relay(
                &p.handle,
                &p.key_pair,
                &p.rt,
                relay_pk,
                addrs,
                token,
                initiator,
                id,
                KEEP_ALIVE,
            )
            .await
        }
        let pk = self.relay_pk;
        tokio::join!(
            pair(&self.a, pk, &addrs, token, true, ida),
            pair(&self.b, pk, &addrs, token, false, idb)
        )
    }

    fn matched(&self) -> u64 {
        self.stats.stats().pairings_matched
    }
}

async fn node(bootstrap: Vec<String>, ephemeral: bool) -> Peer {
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
    Peer {
        rt,
        handle,
        key_pair: KeyPair::from_seed(rand::random::<[u8; 32]>()),
        events: Some(events),
        _task: task,
    }
}

/// Four routable nodes (they keep announced records); returns the bootstrap list and the nodes.
async fn cluster() -> (Vec<String>, Vec<Peer>) {
    let boot = node(vec![], false).await;
    let port = boot.handle.local_port().await.expect("port");
    let bootstrap = vec![format!("127.0.0.1:{port}")];
    let mut holders = vec![boot];
    for _ in 0..3 {
        holders.push(node(bootstrap.clone(), false).await);
    }
    (bootstrap, holders)
}

async fn silent_relay() -> Silent {
    let (bootstrap, holders) = cluster().await;
    let mut server_node = node(bootstrap.clone(), false).await;
    let events = server_node.events.take().expect("events");
    let rt = UdxRuntime::shared(server_node.rt.handle());
    let (server, incoming) = Server::new(
        server_node.handle.clone(),
        events,
        rt,
        ServerOpts::default(),
    );
    let relay_kp = KeyPair::from_seed([0x51; 32]);
    server.listen(relay_kp.clone()).await.expect("listen");
    Silent {
        client: node(bootstrap, true).await,
        relay_pk: relay_kp.public_key,
        _server: server,
        _incoming: incoming,
        _rt: server_node.rt,
        _holders: holders,
    }
}

async fn quiet_relay() -> Quiet {
    let (bootstrap, holders) = cluster().await;
    let mut relay_node = node(bootstrap.clone(), false).await;
    let events = relay_node.events.take().expect("events");
    let config = RelayServiceConfig {
        announce: false,
        ..Default::default()
    };
    let (stats, service) = run_relay_server(
        relay_node.rt.handle(),
        relay_node.handle.clone(),
        relay_node.key_pair.clone(),
        events,
        config,
    );
    let port = relay_node.handle.local_port().await.expect("port");
    let relay_addr = Ipv4Peer {
        host: "127.0.0.1".into(),
        port,
    };
    let (a, b) = (
        node(bootstrap.clone(), true).await,
        node(bootstrap, true).await,
    );
    Quiet {
        a,
        b,
        relay_pk: relay_node.key_pair.public_key,
        relay_addr,
        stats,
        _service: Box::new(service),
        _relay_node: relay_node,
        _holders: holders,
    }
}

/// Polls `cond` every 20 ms for up to `limit`; whether it became true.
async fn until(cond: impl Fn() -> bool, limit: Duration) -> bool {
    let end = Instant::now() + limit;
    while !cond() {
        if Instant::now() > end {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    true
}
