// JS: hyperdht/lib/connect.js (retry_route; JS drops the route on the first failure, we keep it)
//! Live tests, part 4: a route survives a failed retry while a punched socket of the pair lives.
//! The failure is injected (`Reuse::fail_next_route`); the fixture is in the parent
//! (`reuse_live_tests.rs`), included from there.

use super::*;

mod one_handshake_lost {
    use super::*;

    #[tokio::test]
    async fn route_is_kept() {
        within(async {
            // given: a live connection L holds the punch socket
            let mut fx = setup(Opts::default()).await;
            let long = fx.when_connect().await;
            // when: the next retry over its route fails
            fx.client.handle.reuse.fail_next_route();
            let second = fx.when_connect().await;
            // then: a connection is made by the fallback, and the route was not dropped
            assert_eq!(fx.hits(), 1, "route was not tried");
            assert_eq!(fx.drops(), 0, "one lost handshake dropped the route");
            assert!(
                echoes(second).await,
                "data over the fallback connection did not arrive"
            );
            drop(long);
        })
        .await;
    }
}

mod short_connection_expired {
    use super::*;

    #[tokio::test]
    async fn next_goes_over_the_live_connection_route() {
        within(async {
            // given: L (socket S1) is alive; a failed retry sends the second connection T to a new socket S2
            let mut fx = setup(Opts::default()).await;
            let long = fx.when_connect().await;
            let s1 = long.client.socket.clone();
            fx.client.handle.reuse.fail_next_route();
            let short = fx.when_connect().await;
            assert!(
                !short.client.socket.ptr_eq(&s1),
                "T did not leave S1: the injected failure had no effect"
            );
            // when: T is closed and its socket S2 expires
            drop(short);
            tokio::time::sleep(LINGER + Duration::from_secs(1)).await;
            let hits = fx.hits();
            let third = fx.when_connect().await;
            // then: the third goes over the route of L on S1, nothing was dropped, data flows
            assert!(fx.hits() > hits, "route was not tried");
            assert!(third.client.socket.ptr_eq(&s1), "third is not on S1");
            assert_eq!(fx.drops(), 0, "a route was dropped");
            assert!(
                echoes(third).await,
                "data over the third connection did not arrive"
            );
            drop(long);
        })
        .await;
    }
}
