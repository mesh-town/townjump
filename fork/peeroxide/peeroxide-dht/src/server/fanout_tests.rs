//! Who gets the ANNOUNCE, on a local cluster of 8 nodes: a server's announcement
//! (`announce_server`, `listen`, `resume`) goes to the 3 closest repliers, a topic's
//! (`announce`) still goes to every replier. Holders are read back with `lookup`.

use crate::crypto::hash;
use crate::hyperdht::HyperDhtHandle;
use crate::hyperdht_messages::RelayInfo;
use crate::server::ServerOpts;
use crate::server::test_net::{cluster, key, node, serve, within};

mod server_announce {
    use super::*;

    /// More than 3 nodes answer the query, yet only the 3 closest get the ANNOUNCE: they are the relays.
    #[tokio::test]
    async fn reaches_three_closest() {
        within(async {
            // given
            let cl = cluster(8).await;
            let announcer = node(&cl, false).await;
            let kp = key(1);
            let target = hash(&kp.public_key);
            // when
            let announced = announcer.handle.announce_server(target, &kp).await;
            // then
            let announced = announced.expect("announce_server");
            assert_eq!(announced.relays.len(), 3, "relays: {:?}", announced.relays);
            assert!(
                announced.closest_nodes.len() > 3,
                "only {} repliers, the test needs more than 3",
                announced.closest_nodes.len()
            );
            assert_eq!(
                holders(&announcer.handle, target).await,
                relay_addrs(&announced.relays),
                "the holders of the record must be exactly the relays"
            );
        })
        .await
    }

    /// `Server::listen` announces the server with the server fan-out: 3 holders.
    #[tokio::test]
    async fn listen_announces_to_three() {
        within(async {
            // given
            let cl = cluster(8).await;
            let kp = key(3);
            let served = serve(node(&cl, false).await, &kp, ServerOpts::default()).await;
            // when
            let found = holders(&served.handle, hash(&kp.public_key)).await;
            // then
            assert_eq!(found.len(), 3, "holders after listen: {found:?}");
        })
        .await
    }

    /// `resume` re-announces the server with the server fan-out, not to every replier.
    #[tokio::test]
    async fn resume_announces_to_three() {
        within(async {
            // given
            let cl = cluster(8).await;
            let kp = key(4);
            let served = serve(node(&cl, false).await, &kp, ServerOpts::default()).await;
            // when
            served.handle.suspend().await.expect("suspend");
            served.handle.resume().await.expect("resume");
            // then
            let found = holders(&served.handle, hash(&kp.public_key)).await;
            assert!(
                (1..=3).contains(&found.len()),
                "holders after resume: {found:?}"
            );
        })
        .await
    }
}

mod topic_announce {
    use super::*;

    /// A topic announce (what hyperswarm does) is not narrowed: every replier holds the record.
    #[tokio::test]
    async fn still_reaches_every_replier() {
        within(async {
            // given
            let cl = cluster(8).await;
            let announcer = node(&cl, false).await;
            let kp = key(2);
            let target = hash(&kp.public_key);
            // when
            let announced = announcer.handle.announce(target, &kp, &[]).await;
            // then
            announced.expect("announce");
            let found = holders(&announcer.handle, target).await;
            assert!(found.len() > 3, "holders of a topic: {found:?}");
        })
        .await
    }
}

/// `(host, port)` of every node that holds a record under `target`, sorted.
async fn holders(handle: &HyperDhtHandle, target: [u8; 32]) -> Vec<(String, u16)> {
    let found = handle.lookup(target).await.expect("lookup");
    let mut addrs: Vec<_> = found
        .into_iter()
        .map(|result| (result.from.host, result.from.port))
        .collect();
    addrs.sort();
    addrs
}

/// `(host, port)` of the relay nodes, sorted.
fn relay_addrs(relays: &[RelayInfo]) -> Vec<(String, u16)> {
    let mut addrs: Vec<_> = relays
        .iter()
        .map(|r| (r.relay_address.host.clone(), r.relay_address.port))
        .collect();
    addrs.sort();
    addrs
}
