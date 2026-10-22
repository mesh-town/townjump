// JS: hyperdht/lib/connect.js (retry_route, fallback to FIND_PEER)
//! Live tests, part 3: route failure (fallback to a new connection, `drop_route`), a route
//! on the node IO socket (direct branch, the socket is not held) and a client behind the server's own
//! public IP (the server answers from its node socket). Fixture and helpers are in the
//! parent (`reuse_live_tests.rs`), included from there.

use super::*;

mod route_fails {
    use super::*;

    mod server_dropped_its_side_and_socket_expired {
        use super::*;

        #[tokio::test]
        async fn falls_back_to_a_new_connection_and_drops_the_route() {
            within(async {
                // given: the client holds the first; the server closed its own, and its socket expired (LINGER + 1 s)
                let mut fx = setup(Opts::default()).await;
                let Pair { client, server } = fx.when_connect().await;
                let was = client.socket.clone();
                drop(server);
                tokio::time::sleep(LINGER + Duration::from_secs(1)).await;
                // when: a second connection (the client route leads nowhere)
                let second = fx.when_connect().await;
                // then: connected (fallback to lookup), route dropped, new socket
                assert_eq!(fx.hits(), 1, "route was not tried");
                assert_eq!(fx.drops(), 1, "failure did not drop the route");
                assert!(
                    !second.client.socket.ptr_eq(&was),
                    "client socket is the old one"
                );
                assert!(
                    echoes(second).await,
                    "data over the fallback connection did not arrive"
                );
                drop(client);
            })
            .await;
        }
    }
}

mod io_socket_route {
    use super::*;

    mod nodes_not_firewalled {
        use super::*;

        #[tokio::test]
        async fn second_connection_goes_over_the_route_and_works() {
            within(async {
                // given: nodes not firewalled, local_connection default; the first connection is alive
                let mut fx = setup(Opts::direct()).await;
                let first = fx.when_connect().await;
                // when: a second connection
                let second = fx.when_connect().await;
                // then: the route is taken and not dropped, data flows
                assert_eq!(fx.hits(), 1, "route on the IO socket not taken");
                assert_eq!(fx.drops(), 0, "route on the IO socket dropped");
                assert!(
                    echoes(second).await,
                    "data over the second connection did not arrive"
                );
                drop(first);
            })
            .await;
        }

        #[tokio::test]
        async fn second_connection_over_the_route_skips_rto_wait() {
            within(async {
                // given: nodes not firewalled; the first connection is alive
                let mut fx = setup(Opts::direct()).await;
                let first = fx.when_connect().await;
                // when: a second connection (under a clock)
                let started = std::time::Instant::now();
                let second = fx.when_connect().await;
                let took = started.elapsed();
                // then: the route is taken, and the first server packet is not lost (no RTO wait)
                assert_eq!(fx.hits(), 1, "route on the IO socket not taken");
                assert!(
                    took < Duration::from_millis(800),
                    "second connection took {took:?} (waited for RTO?)"
                );
                drop((first, second));
            })
            .await;
        }

        #[tokio::test]
        async fn node_socket_is_never_held() {
            within(async {
                // given: nodes not firewalled; the first connection is alive
                let mut fx = setup(Opts::direct()).await;
                let first = fx.when_connect().await;
                // when: a second connection
                let second = fx.when_connect().await;
                // then: the client connection goes over the node IO socket (otherwise the check is empty), and none
                // of the connection sockets is held: a hold on the IO socket would hijack the node receive
                let io = fx.client_io_socket().await;
                assert!(
                    first.client.socket.ptr_eq(&io),
                    "direct branch went out from another socket"
                );
                for pair in [&first, &second] {
                    assert!(
                        !fx.client_held(&pair.client.socket),
                        "client IO socket held"
                    );
                    assert!(
                        !fx.server_held(&pair.server.socket),
                        "server IO socket held"
                    );
                }
            })
            .await;
        }
    }
}

mod same_host {
    use super::*;

    /// Both nodes firewalled on one host (a loopback cluster), the client announces its LAN
    /// addresses (`Opts::lan()`), so the server takes the plan without a puncher.
    mod client_behind_our_public_ip {
        use super::*;

        #[tokio::test]
        async fn server_answers_on_its_node_socket() {
            within(async {
                // given: a firewalled server and a client on the same host that announces a LAN address
                let mut fx = setup(Opts::lan()).await;
                // when: the first connection
                let first = fx.when_connect().await;
                // then: the server side runs on its node IO socket (not on a puncher) and holds nothing
                let io = fx.server_io_socket().await;
                assert!(
                    first.server.socket.ptr_eq(&io),
                    "server answered from a puncher socket"
                );
                assert!(
                    !fx.server_held(&first.server.socket),
                    "server socket is held"
                );
            })
            .await;
        }

        #[tokio::test]
        async fn second_connection_past_linger_goes_over_the_route() {
            within(async {
                // given: the first connection is closed on both sides and LINGER + 1 s have passed
                let mut fx = setup(Opts::lan()).await;
                let first = fx.when_connect().await;
                drop(first);
                tokio::time::sleep(LINGER + Duration::from_secs(1)).await;
                // when: a second connection
                let second = fx.when_connect().await;
                // then: the route to the node socket is taken and survives (nothing expired), data flows
                assert_eq!(fx.hits(), 1, "route not taken");
                assert_eq!(fx.drops(), 0, "the answer came from a closed puncher");
                assert!(
                    echoes(second).await,
                    "data over the second connection did not arrive"
                );
            })
            .await;
        }
    }
}
