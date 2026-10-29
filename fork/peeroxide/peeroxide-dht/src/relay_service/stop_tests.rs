//! Ownership and stopping of the relay service's tasks: `TaskSet` rules, and `close` / `Drop` /
//! a dead node on a real relay node (announce withdrawn, every task gone, no announce needed).
// JS: blind-relay/index.js

use std::time::{Duration, Instant};

use libudx::UdxRuntime;
use tokio::sync::{mpsc, oneshot};

use super::*;
use crate::crypto::hash;
use crate::hyperdht::{
    HyperDhtConfig, HyperDhtError, KeyPair, RelayPaired, ServerEvent, pair_via_relay, spawn,
};
use crate::messages::Ipv4Peer;
use crate::relay_service::test_net::{RelayNet, relay_net};
use crate::relay_service::{RelayServiceConfig, run_relay_server};
use crate::rpc::DhtConfig;

mod task_set {
    use super::*;

    #[tokio::test]
    async fn closed_set_drops_a_spawned_future_unrun() {
        // given: a set that was taken (closed)
        let tasks = TaskSet::new();
        let _taken = tasks.take();
        let (ran, was_run) = oneshot::channel::<()>();

        // when: a future is spawned into it
        tasks.spawn(async move {
            let _ = ran.send(());
        });

        // then: the future is dropped without running, and nothing is left alive
        assert!(was_run.await.is_err(), "the future must not run");
        assert!(tasks.idle());
    }

    #[tokio::test]
    async fn nested_task_is_aborted_with_its_parent() {
        // given: a task that spawns a grandchild through the same set
        let tasks = TaskSet::new();
        let probe = tasks.probe();
        let inner = tasks.clone();
        tasks.spawn(async move {
            inner.spawn(std::future::pending());
            std::future::pending::<()>().await;
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!probe.is_finished(), "tasks are alive before the stop");

        // when: the set is aborted and the last handle goes away
        tasks.abort_now();
        drop(tasks);

        // then: both tasks are gone within a second
        assert!(becomes(Duration::from_secs(1), || probe.is_finished()).await);
    }

    #[tokio::test]
    async fn finished_entries_are_reaped_on_the_next_spawn() {
        // given: 100 tasks that end at once
        let tasks = TaskSet::new();
        for _ in 0..100 {
            tasks.spawn(async {});
        }
        tokio::time::sleep(Duration::from_millis(100)).await;

        // when: one more task is spawned
        tasks.spawn(async {});

        // then: the finished entries are gone
        assert!(tasks.len() <= 2, "entries left: {}", tasks.len());
    }
}

mod close {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn stops_every_task_even_with_a_live_session() {
        // given: an announcing service, a fast sweep and one live pairing
        let config = RelayServiceConfig {
            sweep_interval: Duration::from_millis(50),
            ..RelayServiceConfig::default()
        };
        let mut n = relay_net(config).await;
        let _paired = pair_both(&n, &[]).await;
        assert!(
            n.relay.stats().sessions_active > 0,
            "a session is open on the relay"
        );
        let service = n.service.take().expect("service handle");
        let probe = service.tasks.probe();

        // when: the service is closed
        let started = Instant::now();
        let closed = tokio::time::timeout(Duration::from_secs(7), service.close()).await;

        // then: close returned in time and no task of the service is left on the relay
        assert!(closed.is_ok(), "close took {:?}", started.elapsed());
        assert!(
            becomes(Duration::from_secs(1), || probe.is_finished()).await,
            "tasks outlive close"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn withdraws_the_announce() {
        // given: an announced relay that is found
        let mut n = relay_net(RelayServiceConfig::default()).await;
        assert!(found(&n).await, "the relay is announced before close");
        let service = n.service.take().expect("service handle");

        // when: it is closed
        service.close().await;

        // then: the record disappears
        let gone = becomes_async(Duration::from_secs(10), || async { !found(&n).await }).await;
        assert!(gone, "find_peer still returns the relay");
    }
}

mod drop_of_the_handle {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn aborts_every_task_without_close() {
        // given: a running service
        let mut n = relay_net(RelayServiceConfig::default()).await;
        let service = n.service.take().expect("service handle");
        let probe = service.tasks.probe();
        assert!(!service.is_finished());

        // when: the handle is dropped
        drop(service);

        // then: the tasks end within 2 s
        assert!(becomes(Duration::from_secs(2), || probe.is_finished()).await);
    }
}

mod dead_node {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn finishes_the_service() {
        // given: a service on a node whose event channel can be closed
        let (events, feed) = mpsc::unbounded_channel::<ServerEvent>();
        let (_node, service) = lone_service(feed).await;
        assert!(!service.is_finished());

        // when: the sender goes away
        drop(events);

        // then: the service reports itself finished
        assert!(becomes(Duration::from_secs(2), || service.is_finished()).await);
    }
}

mod without_announce {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn is_not_found_but_pairs_by_address() {
        // given: a relay that does not announce itself
        let config = RelayServiceConfig {
            announce: false,
            ..RelayServiceConfig::default()
        };
        let n = relay_net(config).await;

        // when: peers look it up for 3 s
        let start = Instant::now();
        let mut ever_found = false;
        while start.elapsed() < Duration::from_secs(3) {
            ever_found |= found(&n).await;
            tokio::time::sleep(Duration::from_millis(300)).await;
        }

        // then: it is never found, yet two peers pair through it by its address
        assert!(!ever_found, "a silent relay must not be discoverable");
        let paired = pair_both(&n, std::slice::from_ref(&n.relay_addr)).await;
        assert!(paired.is_some(), "pairing by address works");
        assert!(n.relay.stats().pairings_matched >= 1);
    }
}

/// Poll `cond` every 20 ms until it holds or `limit` passes.
async fn becomes(limit: Duration, cond: impl Fn() -> bool) -> bool {
    becomes_async(limit, || async { cond() }).await
}

async fn becomes_async<F, Fut>(limit: Duration, cond: F) -> bool
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let start = Instant::now();
    while !cond().await {
        if start.elapsed() > limit {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    true
}

/// Does peer A find the relay's record on the DHT?
async fn found(n: &RelayNet) -> bool {
    matches!(n.a.handle.find_peer(hash(&n.relay_pk)).await, Ok(Some(_)))
}

/// Pair A with B on a fresh token through the relay (by `addresses`, or via the DHT when empty).
async fn pair_both(n: &RelayNet, addresses: &[Ipv4Peer]) -> Option<(RelayPaired, RelayPaired)> {
    let token = rand::random::<[u8; 32]>();
    let (a, b) = tokio::join!(
        pair_one(n, true, addresses, token, 0x1001),
        pair_one(n, false, addresses, token, 0x2001)
    );
    Some((a.ok()?, b.ok()?))
}

async fn pair_one(
    n: &RelayNet,
    is_a: bool,
    addresses: &[Ipv4Peer],
    token: [u8; 32],
    id: u32,
) -> Result<RelayPaired, HyperDhtError> {
    let p = if is_a { &n.a } else { &n.b };
    let paired = pair_via_relay(
        &p.handle,
        &p.key_pair,
        &p.rt,
        n.relay_pk,
        addresses,
        token,
        is_a,
        id,
        Duration::from_secs(5),
    );
    tokio::time::timeout(Duration::from_secs(20), paired)
        .await
        .unwrap_or(Err(HyperDhtError::RelayAborted))
}

/// A relay service on a lone loopback node, fed by `feed` instead of the node's own events.
async fn lone_service(
    feed: mpsc::UnboundedReceiver<ServerEvent>,
) -> (UdxRuntime, RelayServiceHandle) {
    let rt = UdxRuntime::new().expect("udx runtime");
    let dht = DhtConfig {
        bootstrap: vec![],
        host: "127.0.0.1".to_string(),
        firewalled: false,
        ephemeral: Some(false),
        ..DhtConfig::default()
    };
    let cfg = HyperDhtConfig {
        dht,
        ..HyperDhtConfig::default()
    };
    let (_task, handle, _own_events) = spawn(&rt, cfg).await.expect("spawn node");
    let config = RelayServiceConfig {
        announce: false,
        ..RelayServiceConfig::default()
    };
    let (_relay, service) =
        run_relay_server(rt.handle(), handle, KeyPair::generate(), feed, config);
    (rt, service)
}
