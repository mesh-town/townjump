//! `unannounce_relays` against a local cluster, and `close` of a suspended server.

use std::time::{Duration, Instant};

use crate::crypto::hash;
use crate::server::ServerOpts;
use crate::server::test_net::{cluster, key, node, serve, within};

mod unannounce_relays {
    use super::*;

    #[tokio::test]
    async fn removes_the_record_from_every_relay() {
        within(async {
            // given
            let cl = cluster(3).await;
            let kp = key(22);
            let target = hash(&kp.public_key);
            let served = serve(node(&cl, false).await, &kp, ServerOpts::default()).await;
            let relays = served.handle.current_relay_addresses();
            let client = node(&cl, false).await;
            // when
            let n = served.handle.unannounce_relays(target, &kp, &relays).await;
            // then
            assert_eq!(n, relays.len());
            let found = client.handle.find_peer(target).await.expect("find_peer");
            assert!(found.is_none(), "record still there: {found:?}");
        })
        .await
    }

    #[tokio::test]
    async fn skips_nodes_without_the_record() {
        within(async {
            // given
            let cl = cluster(3).await;
            let kp = key(23);
            let client = node(&cl, false).await;
            // when
            let n = client
                .handle
                .unannounce_relays(hash(&kp.public_key), &kp, &cl.addrs)
                .await;
            // then
            assert_eq!(n, 0);
        })
        .await
    }

    #[tokio::test]
    async fn does_nothing_for_no_relays() {
        within(async {
            // given
            let cl = cluster(2).await;
            let kp = key(26);
            let client = node(&cl, false).await;
            let start = Instant::now();
            // when
            let n = client
                .handle
                .unannounce_relays(hash(&kp.public_key), &kp, &[])
                .await;
            // then
            assert_eq!(n, 0);
            assert!(start.elapsed() < Duration::from_millis(100));
        })
        .await
    }
}

mod server_close {
    use super::*;

    #[tokio::test]
    async fn close_while_suspended_sends_nothing_and_returns() {
        within(async {
            // given
            let cl = cluster(3).await;
            let served = serve(node(&cl, false).await, &key(27), ServerOpts::default()).await;
            served.handle.suspend().await.expect("suspend");
            let start = Instant::now();
            // when
            let closed = served.server.close().await;
            // then
            assert!(closed.is_ok(), "close: {closed:?}");
            assert!(
                start.elapsed() < Duration::from_millis(500),
                "{:?}",
                start.elapsed()
            );
        })
        .await
    }
}
