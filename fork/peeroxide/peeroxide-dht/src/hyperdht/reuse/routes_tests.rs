// Route table of the held-socket store; helpers are in `tests.rs`.
// JS: hyperdht/lib/socket-pool.js

use super::*;

mod added_twice_for_one_key {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_new_address_on_the_same_socket_replaces_the_old() {
        // given: a route on a socket that is not held (the node's), failed once
        let fx = setup();
        let io = fx.socket().await;
        fx.reuse.add_route(KEY, &io, peer(4001));
        fx.reuse.route_failed(&KEY, &io, false);
        // when: the same socket comes back with a new address
        fx.reuse.add_route(KEY, &io, peer(4002));
        // then: the address is the new one, one record
        assert_eq!(fx.reuse.route(&KEY).expect("route").addr, peer(4002));
        fx.reuse.route_failed(&KEY, &io, false);
        fx.reuse.route_failed(&KEY, &io, false);
        assert!(fx.reuse.route(&KEY).is_none());
        assert_eq!(fx.reuse.drops(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn the_same_socket_is_not_duplicated() {
        // given
        let fx = setup();
        let (s1, _lease) = fx.held().await;
        fx.reuse.add_route(KEY, &s1, peer(4001));
        // when: one failure (the lease is alive), then the same socket added again
        fx.reuse.route_failed(&KEY, &s1, false);
        fx.reuse.add_route(KEY, &s1, peer(4001));
        // then: one record with the count reset: two more failures are needed to drop it
        fx.reuse.route_failed(&KEY, &s1, false);
        assert!(fx.reuse.route(&KEY).is_some(), "the count was not reset");
        fx.reuse.route_failed(&KEY, &s1, false);
        assert!(
            fx.reuse.route(&KEY).is_none(),
            "a duplicate record remained"
        );
        assert_eq!(fx.reuse.drops(), 1);
    }
}

mod two_sockets_for_one_key {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_tie_goes_to_the_last_added() {
        // given: both sockets live, no failures
        let fx = setup();
        let (s1, _lease1) = fx.held().await;
        let (s2, _lease2) = fx.held().await;
        fx.reuse.add_route(KEY, &s1, peer(4001));
        fx.reuse.add_route(KEY, &s2, peer(4002));
        // when / then
        let route = fx.reuse.route(&KEY).expect("route");
        assert!(route.socket.ptr_eq(&s2), "the first added won a tie");
        assert_eq!(route.addr, peer(4002));
    }

    #[tokio::test(start_paused = true)]
    async fn expiry_of_one_keeps_the_other() {
        // given: S2 (no live lease) is added first, S1 (live lease) second
        let fx = setup();
        let (s2, lease2) = fx.held().await;
        fx.reuse.add_route(KEY, &s2, peer(4002));
        let (s1, _lease1) = fx.held().await;
        fx.reuse.add_route(KEY, &s1, peer(4001));
        drop(lease2);
        // when: S2 expires
        sleep(LINGER + ONE_SEC).await;
        // then: the route is S1
        let route = fx.reuse.route(&KEY).expect("route on the live socket");
        assert!(route.socket.ptr_eq(&s1));
        assert_eq!(route.addr, peer(4001));
    }

    #[tokio::test(start_paused = true)]
    async fn route_prefers_fewer_failures() {
        // given: S1 added first and failed once (kept: its lease is alive), S2 added second
        let fx = setup();
        let (s1, _lease1) = fx.held().await;
        fx.reuse.add_route(KEY, &s1, peer(4001));
        let (s2, _lease2) = fx.held().await;
        fx.reuse.add_route(KEY, &s2, peer(4002));
        fx.reuse.route_failed(&KEY, &s1, false);
        // when / then
        let route = fx.reuse.route(&KEY).expect("route");
        assert!(route.socket.ptr_eq(&s2), "the record without failures wins");
    }
}

mod route_failed {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn once_with_a_live_lease_keeps_the_route() {
        // given
        let fx = setup();
        let (s1, _lease) = fx.hold_with_route(KEY).await;
        // when
        fx.reuse.route_failed(&KEY, &s1, false);
        // then
        assert_eq!(fx.reuse.drops(), 0);
        let route = fx.reuse.route(&KEY).expect("route kept");
        assert!(route.socket.ptr_eq(&s1));
    }

    #[tokio::test(start_paused = true)]
    async fn twice_in_a_row_drops_the_route() {
        // given
        let fx = setup();
        let (s1, _lease) = fx.hold_with_route(KEY).await;
        // when
        fx.reuse.route_failed(&KEY, &s1, false);
        assert!(fx.reuse.route(&KEY).is_some(), "kept after the first");
        fx.reuse.route_failed(&KEY, &s1, false);
        // then
        assert_eq!(fx.reuse.drops(), 1);
        assert!(fx.reuse.route(&KEY).is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn without_another_lease_drops_the_route() {
        // given: the hold lease is gone; the only lease is the attempt's own
        let fx = setup();
        let (s1, lease) = fx.hold_with_route(KEY).await;
        let attempt = fx.reuse.route(&KEY).expect("route");
        drop(lease);
        // when
        fx.reuse.route_failed(&KEY, &s1, attempt.lease.is_some());
        // then
        assert_eq!(fx.reuse.drops(), 1);
        drop(attempt);
        assert!(fx.reuse.route(&KEY).is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn success_in_between_resets_the_count() {
        // given
        let fx = setup();
        let (s1, _lease) = fx.hold_with_route(KEY).await;
        // when: failure, success, failure
        fx.reuse.route_failed(&KEY, &s1, false);
        fx.reuse.route_succeeded(&KEY, &s1);
        fx.reuse.route_failed(&KEY, &s1, false);
        // then
        assert_eq!(fx.reuse.drops(), 0);
        assert!(fx.reuse.route(&KEY).is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn on_another_socket_changes_nothing() {
        // given
        let fx = setup();
        let (s1, _lease) = fx.hold_with_route(KEY).await;
        let s2 = fx.socket().await;
        // when: two failures reported for a socket without a record
        fx.reuse.route_failed(&KEY, &s2, false);
        fx.reuse.route_failed(&KEY, &s2, false);
        // then
        assert_eq!(fx.reuse.drops(), 0);
        assert!(fx.reuse.route(&KEY).expect("route").socket.ptr_eq(&s1));
    }

    #[tokio::test(start_paused = true)]
    async fn unheld_socket_survives_one_failure() {
        // given: a socket like the IO socket has no lease at all
        let fx = setup();
        let io = fx.socket().await;
        fx.reuse.add_route(KEY, &io, peer(4001));
        // when / then: dropped only by the second failure
        fx.reuse.route_failed(&KEY, &io, false);
        assert!(fx.reuse.route(&KEY).is_some());
        fx.reuse.route_failed(&KEY, &io, false);
        assert!(fx.reuse.route(&KEY).is_none());
        assert_eq!(fx.reuse.drops(), 1);
    }
}

mod on_unheld_socket {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn lives_without_lease() {
        // given: a socket like the IO socket is not held
        let fx = setup();
        let io_socket = fx.socket().await;
        fx.reuse.add_route(KEY, &io_socket, peer(4001));
        // when
        sleep(10 * ONE_SEC).await;
        // then
        let route = fx
            .reuse
            .route(&KEY)
            .expect("route of an unheld socket has no timer");
        assert!(route.lease.is_none(), "an unheld socket gives no lease");
    }
}

mod on_held_socket {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn carries_a_lease() {
        // given
        let fx = setup();
        let (_s, _lease) = fx.hold_with_route(KEY).await;
        // when
        let route = fx.reuse.route(&KEY).expect("route");
        // then
        assert!(route.lease.is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn lease_keeps_socket_until_it_is_dropped() {
        // given: the hold lease is released, the route lease is alive
        let fx = setup();
        let (s, lease) = fx.hold_with_route(KEY).await;
        let route = fx.reuse.route(&KEY).expect("route");
        drop(lease);
        sleep(LINGER + ONE_SEC).await;
        assert!(fx.reuse.is_held(&s), "the route's own lease must count");
        // when
        drop(route);
        sleep(LINGER + ONE_MS).await;
        // then
        assert!(!fx.reuse.is_held(&s));
    }
}

mod clear {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn forgets_routes_and_held_sockets() {
        // given
        let fx = setup();
        let (s, lease) = fx.hold_with_route(KEY).await;
        // when
        fx.reuse.clear();
        // then
        assert!(fx.reuse.route(&KEY).is_none());
        assert!(fx.reuse.lease(&s).is_none());
        assert!(!fx.reuse.is_held(&s));
        drop(lease); // a dangling lease finds nothing
    }

    #[tokio::test(start_paused = true)]
    async fn is_terminal() {
        // given
        let fx = setup();
        let (s, _lease) = fx.hold_with_route(KEY).await;
        fx.reuse.clear();
        // when
        let again = fx.reuse.hold(&s);
        fx.reuse.add_route(OTHER_KEY, &s, peer(4001));
        // then
        assert!(again.is_none(), "hold after clear must refuse");
        assert!(
            fx.reuse.route(&OTHER_KEY).is_none(),
            "add_route after clear does nothing"
        );
    }
}

mod counters {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn hits_count_found_routes_only() {
        // given
        let fx = setup();
        let (_s, _lease) = fx.hold_with_route(KEY).await;
        // when
        let found = fx.reuse.route(&KEY);
        let missing = fx.reuse.route(&OTHER_KEY);
        // then
        assert!(found.is_some() && missing.is_none());
        assert_eq!(fx.reuse.hits(), 1);
    }
}
