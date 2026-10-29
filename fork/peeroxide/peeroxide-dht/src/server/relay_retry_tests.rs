//! Cluster tests of `RelayThrough::List` on the client: after its own relay aborted the connect
//! goes on with the next key of the list, but never for a relay the server chose.
//!
//! The direct path is closed on both sides, so the relay alone decides the outcome. A "dead"
//! relay is an ordinary server on its own key: it answers the handshake but speaks no
//! blind-relay, so the pairing runs into its 15 s limit (like `relay_never_pairs_aborts`) and the
//! elapsed time shows how many attempts were made.

use std::time::{Duration, Instant};

use crate::hyperdht::{ConnectOpts, HolepunchFilter, HyperDhtError, PeerConnection, RelayThrough};
use crate::server::ServerOpts;
use crate::server::relay_flow_tests::{Relay, relay};
use crate::server::test_net::{
    Cluster, Served, cluster, echo, key, node, roundtrip, serve, sha256, within,
};

mod list_with_dead_first_relay {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn moves_to_the_next_relay_and_pairs_there() {
        within(async {
            // given: the list [dead, live] tried from its first entry
            let w = setup(closed_punch()).await;
            let _dead = dead_relay(&w.cl, 9).await;
            let live = relay(&w.cl).await;
            let list = RelayThrough::List(vec![key(9).public_key, live.pk]);
            // when: the client connects
            let (conn, took) = w.connect(list).await;
            // then: the connection runs through the live relay, after the dead one was tried
            echo_64k(conn.expect("connected")).await;
            assert!(matched(&live) >= 1, "the live relay made no pairing");
            assert!(
                took >= Duration::from_secs(10),
                "dead relay not tried first: {took:?}"
            );
        })
        .await;
    }
}

mod list_of_dead_relays {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn aborts_after_one_attempt_per_relay() {
        // given: the list [dead1, dead2]
        let w = setup(closed_punch()).await;
        let (_d1, _d2) = (dead_relay(&w.cl, 9).await, dead_relay(&w.cl, 10).await);
        let list = RelayThrough::List(vec![key(9).public_key, key(10).public_key]);
        // when: the client connects (own limit, two attempts take ~30 s)
        let both = w.connect(list);
        let (conn, took) = tokio::time::timeout(Duration::from_secs(70), both)
            .await
            .expect("connect ends within 70 s");
        // then: it aborts, and the time shows two attempts, not one and not three
        assert!(
            matches!(conn, Err(HyperDhtError::RelayAborted)),
            "{:?}",
            conn.err()
        );
        assert!(
            took >= Duration::from_secs(25),
            "only one attempt? {took:?}"
        );
        assert!(
            took < Duration::from_secs(50),
            "more than two attempts? {took:?}"
        );
    }
}

mod server_with_its_own_dead_relay {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn abort_is_not_retried_with_the_clients_list() {
        within(async {
            // given: the server names a dead relay; the client offers two live ones
            let opts = ServerOpts {
                relay_through: Some(RelayThrough::Key(key(9).public_key)),
                ..closed_punch()
            };
            let w = setup(opts).await;
            let _dead = dead_relay(&w.cl, 9).await;
            let (a, b) = (relay(&w.cl).await, relay(&w.cl).await);
            let list = RelayThrough::List(vec![a.pk, b.pk]);
            // when: the client connects (the server's relay wins, and it is dead)
            let (conn, took) = w.connect(list).await;
            // then: one attempt only, and the client's relays were never asked
            assert!(
                matches!(conn, Err(HyperDhtError::RelayAborted)),
                "{:?}",
                conn.err()
            );
            assert!(took < Duration::from_secs(25), "retried? {took:?}");
            assert_eq!(requested(&a) + requested(&b), 0, "a client relay was used");
        })
        .await;
    }
}

mod list_with_live_first_relay {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn connects_in_one_attempt() {
        within(async {
            // given: the list [live, dead]
            let w = setup(closed_punch()).await;
            let live = relay(&w.cl).await;
            let _dead = dead_relay(&w.cl, 9).await;
            let list = RelayThrough::List(vec![live.pk, key(9).public_key]);
            // when: the client connects
            let (conn, took) = w.connect(list).await;
            // then: it is quick, so the dead relay was never tried
            echo_64k(conn.expect("connected")).await;
            assert!(
                took < Duration::from_secs(10),
                "dead relay was tried: {took:?}"
            );
            assert!(matched(&live) >= 1, "the live relay made no pairing");
        })
        .await;
    }
}

/// A running cluster with a firewalled echo server on `key(1)`.
struct World {
    cl: Cluster,
}

impl World {
    /// Connects a fresh firewalled client with the relay list, direct path closed, list tried
    /// from its first entry; returns the outcome and how long it took.
    async fn connect(
        &self,
        list: RelayThrough,
    ) -> (Result<PeerConnection, HyperDhtError>, Duration) {
        let client = node(&self.cl, true).await;
        let opts = ConnectOpts {
            local_connection: false,
            relay_through: Some(list),
            holepunch: Some(HolepunchFilter::new(|_| false)),
            relay_start: Some(0),
            ..ConnectOpts::default()
        };
        let started = Instant::now();
        let out = client
            .handle
            .connect_with_options(&key(2), key(1).public_key, &[], &client.rt, opts)
            .await;
        std::mem::forget(client);
        (out, started.elapsed())
    }
}

fn closed_punch() -> ServerOpts {
    ServerOpts {
        holepunch: Some(HolepunchFilter::new(|_| false)),
        ..ServerOpts::default()
    }
}

async fn setup(opts: ServerOpts) -> World {
    let cl = cluster(4).await;
    let served = serve(node(&cl, true).await, &key(1), opts).await;
    spawn_echo(served);
    World { cl }
}

/// Echoes every connection the server accepts; owns the server for the rest of the test.
fn spawn_echo(served: Served) {
    tokio::spawn(async move {
        let mut served = served;
        while let Some(conn) = served.incoming.recv().await {
            tokio::spawn(echo(conn));
        }
    });
}

/// A node serving on `key(tag)` without blind-relay: reachable by key, never pairs.
async fn dead_relay(cl: &Cluster, tag: u8) -> Served {
    serve(node(cl, false).await, &key(tag), ServerOpts::default()).await
}

fn matched(r: &Relay) -> u64 {
    r.stats.stats().pairings_matched
}

fn requested(r: &Relay) -> u64 {
    r.stats.stats().pairings_requested
}

async fn echo_64k(conn: PeerConnection) {
    let data: Vec<u8> = (0..64 * 1024).map(|i| (i % 251) as u8).collect();
    assert_eq!(roundtrip(conn, data.clone()).await, sha256(&data));
}
