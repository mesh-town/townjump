//! How long `Server::close` and `HyperDhtHandle::suspend` take to withdraw the record when
//! some nodes of the cluster never answer. A "silent" node is a suspended cluster node (same
//! port, no replies), so the wait is the request timeout, not a refused port.

use std::time::{Duration, Instant};

use crate::messages::Ipv4Peer;
use crate::server::ServerOpts;
use crate::server::test_net::{Cluster, Served, cluster, key, node, serve, within};

/// Suspends every cluster node whose address is in `victims`.
async fn silence(cl: &Cluster, victims: &[Ipv4Peer]) {
    for (i, addr) in cl.addrs.iter().enumerate() {
        if victims.contains(addr) {
            cl.nodes[i].handle.suspend().await.expect("suspend");
        }
    }
}

/// A server on a fresh node and its 3 relays.
async fn served_with_relays(cl: &Cluster, tag: u8) -> (Served, Vec<Ipv4Peer>) {
    let served = serve(node(cl, false).await, &key(tag), ServerOpts::default()).await;
    let relays = served.handle.current_relay_addresses();
    assert_eq!(relays.len(), 3, "relays: {relays:?}");
    (served, relays)
}

mod server_close {
    use super::*;

    /// Silent nodes outside the relay set must not delay `close`.
    #[tokio::test]
    async fn close_asks_only_the_relays() {
        within(async {
            // given
            let cl = cluster(6).await;
            let (served, relays) = served_with_relays(&cl, 21).await;
            let others: Vec<_> = cl
                .addrs
                .iter()
                .filter(|a| !relays.contains(a))
                .cloned()
                .collect();
            silence(&cl, &others).await;
            // when
            let start = Instant::now();
            let closed = served.server.close().await;
            let took = start.elapsed();
            eprintln!("took {took:?}");
            // then
            assert!(closed.is_ok(), "close: {closed:?}");
            assert!(took < Duration::from_millis(1000), "close took {took:?}");
        })
        .await
    }

    /// Silent relays: `close` gives up at the deadline.
    #[tokio::test]
    async fn close_stops_waiting_at_the_deadline() {
        within(async {
            // given
            let cl = cluster(4).await;
            let (served, relays) = served_with_relays(&cl, 24).await;
            silence(&cl, &relays).await;
            // when
            let start = Instant::now();
            let closed = served.server.close().await;
            let took = start.elapsed();
            eprintln!("took {took:?}");
            // then
            assert!(closed.is_ok(), "close: {closed:?}");
            assert!(took < Duration::from_millis(2500), "close took {took:?}");
        })
        .await
    }
}

mod server_suspend {
    use super::*;

    /// Silent relays: `suspend` gives up at the deadline.
    #[tokio::test]
    async fn suspend_stops_waiting_at_the_deadline() {
        within(async {
            // given
            let cl = cluster(4).await;
            let (served, relays) = served_with_relays(&cl, 25).await;
            silence(&cl, &relays).await;
            // when
            let start = Instant::now();
            let suspended = served.handle.suspend().await;
            let took = start.elapsed();
            eprintln!("took {took:?}");
            // then
            assert!(suspended.is_ok(), "suspend: {suspended:?}");
            assert!(took < Duration::from_millis(2500), "suspend took {took:?}");
        })
        .await
    }
}
