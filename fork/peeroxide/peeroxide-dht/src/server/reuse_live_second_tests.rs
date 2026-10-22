// JS: hyperdht/lib/connect.js (reusableSocket)
//! Live tests, part 2: a second connection over the route of the first: with the first alive, in the
//! `LINGER` delay, after it, and without the option at the client or the server. Fixture and helpers are in the
//! parent (`reuse_live_tests.rs`), included from there.

use super::*;

mod first_alive {
    use super::*;

    #[tokio::test]
    async fn goes_over_the_route_on_the_same_sockets() {
        within(async {
            // given: the first connection is alive
            let mut fx = setup(Opts::default()).await;
            let first = fx.when_connect().await;
            let was = first.sockets();
            // when: a second connection
            let second = fx.when_connect().await;
            // then: the same sockets at the client and the server, the route is taken once and not dropped
            assert!(
                second.client.socket.ptr_eq(&was.client),
                "new client socket"
            );
            assert!(
                second.server.socket.ptr_eq(&was.server),
                "new server socket"
            );
            assert_eq!(fx.hits(), 1, "route not taken");
            assert_eq!(fx.drops(), 0, "route dropped");
            assert!(
                echoes(second).await,
                "data over the second connection did not arrive"
            );
            drop(first);
        })
        .await;
    }
}

mod first_dropped_within_linger {
    use super::*;

    #[tokio::test]
    async fn goes_over_the_route_on_the_same_sockets() {
        within(async {
            // given: the first connection is closed on both sides
            let mut fx = setup(Opts::default()).await;
            let first = fx.when_connect().await;
            let was = first.sockets();
            drop(first);
            // when: after 1 s (less than LINGER) a second connection
            tokio::time::sleep(Duration::from_secs(1)).await;
            let second = fx.when_connect().await;
            // then: the sockets were held and are now the same; the route is taken
            assert!(
                second.client.socket.ptr_eq(&was.client),
                "new client socket"
            );
            assert!(
                second.server.socket.ptr_eq(&was.server),
                "new server socket"
            );
            assert_eq!(fx.hits(), 1, "route not taken");
            assert_eq!(fx.drops(), 0, "route dropped");
            assert!(
                echoes(second).await,
                "data over the second connection did not arrive"
            );
        })
        .await;
    }
}

mod first_dropped_past_linger {
    use super::*;

    #[tokio::test]
    async fn punches_a_new_socket_and_does_not_touch_the_route() {
        within(async {
            // given: the first connection is closed on both sides
            let mut fx = setup(Opts::default()).await;
            let first = fx.when_connect().await;
            let was = first.sockets();
            drop(first);
            // when: after LINGER + 1 s a second connection
            tokio::time::sleep(LINGER + Duration::from_secs(1)).await;
            let second = fx.when_connect().await;
            // then: the route expired with the socket: holepunch, new sockets, the route was not taken
            assert!(
                !second.client.socket.ptr_eq(&was.client),
                "client socket is the old one"
            );
            assert!(
                !second.server.socket.ptr_eq(&was.server),
                "server socket is the old one"
            );
            assert_eq!(fx.hits(), 0, "expired route taken");
            assert!(
                echoes(second).await,
                "data over the second connection did not arrive"
            );
        })
        .await;
    }
}

mod client_without_option {
    use super::*;

    #[tokio::test]
    async fn punches_a_new_socket_and_does_not_touch_the_route() {
        within(async {
            // given: the first connection is alive, the client has reusable_socket: false
            let mut fx = setup(Opts::client_off()).await;
            let first = fx.when_connect().await;
            let was = first.sockets();
            // when: a second connection
            let second = fx.when_connect().await;
            // then: no route, new sockets
            assert_eq!(fx.hits(), 0, "route taken without the client option");
            assert!(
                !second.client.socket.ptr_eq(&was.client),
                "client socket is the old one"
            );
            assert!(
                !second.server.socket.ptr_eq(&was.server),
                "server socket is the old one"
            );
            drop(first);
        })
        .await;
    }
}

mod server_without_option {
    use super::*;

    #[tokio::test]
    async fn punches_a_new_socket_and_does_not_touch_the_route() {
        within(async {
            // given: the first connection is alive, the server has reusable_socket: false
            let mut fx = setup(Opts::server_off()).await;
            let first = fx.when_connect().await;
            let was = first.sockets();
            // when: a second connection
            let second = fx.when_connect().await;
            // then: no route, new sockets
            assert_eq!(fx.hits(), 0, "route taken without the server option");
            assert!(
                !second.client.socket.ptr_eq(&was.client),
                "client socket is the old one"
            );
            assert!(
                !second.server.socket.ptr_eq(&was.server),
                "server socket is the old one"
            );
            drop(first);
        })
        .await;
    }
}
