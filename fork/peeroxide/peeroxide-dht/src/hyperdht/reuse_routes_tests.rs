// Route table of the held-socket store; helpers are in `reuse_tests.rs`.
// JS: hyperdht/lib/socket-pool.js

use super::*;

mod added_twice_for_one_key {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn first_route_is_kept() {
        // given
        let fx = setup();
        let (s1, s2) = (fx.socket().await, fx.socket().await);
        fx.reuse.add_route(KEY, &s1, peer(4001));
        // when
        fx.reuse.add_route(KEY, &s2, peer(4002));
        // then
        let route = fx.reuse.route(&KEY).expect("route");
        assert!(
            route.socket.ptr_eq(&s1),
            "route must stay on the first socket"
        );
        assert!(
            !route.socket.ptr_eq(&s2),
            "route must not move to the second socket"
        );
        assert_eq!(route.addr, peer(4001));
    }
}

mod drop_route {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn by_another_socket_keeps_it() {
        // given
        let fx = setup();
        let (s1, s2) = (fx.socket().await, fx.socket().await);
        fx.reuse.add_route(KEY, &s1, peer(4001));
        // when
        fx.reuse.drop_route(&KEY, &s2);
        // then
        assert!(
            fx.reuse.route(&KEY).is_some(),
            "route of another socket stays"
        );
        assert_eq!(fx.reuse.drops(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn by_its_socket_removes_it() {
        // given
        let fx = setup();
        let s1 = fx.socket().await;
        fx.reuse.add_route(KEY, &s1, peer(4001));
        // when
        fx.reuse.drop_route(&KEY, &s1);
        // then
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
