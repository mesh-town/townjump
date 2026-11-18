// JS: hyperdht/lib/server.js (hs.onsocket: routes.add on the accepting side)
//! Live tests: a server that accepted a direct connection keeps a route to the client, so it can
//! dial the client back over the same socket and address. Both ends listen (A on `key(2)`, B on
//! `key(1)`); A connects to B, B is the acceptor. Fixture is in `test_net.rs`.

use std::time::Duration;

use super::ServerOpts;
use super::test_net::{Listening, both_listen, echo, roundtrip, sha256, within};
use crate::hyperdht::{ConnectOpts, HyperDhtError, LINGER, PeerConnection};

fn reusable(on: bool) -> ServerOpts {
    ServerOpts {
        reusable_socket: on,
        ..ServerOpts::default()
    }
}

fn connect_opts(firewalled: bool, reusable: bool) -> ConnectOpts {
    ConnectOpts {
        local_connection: !firewalled,
        reusable_socket: reusable,
        ..ConnectOpts::default()
    }
}

/// A connects to B; returns A's end and the end B accepted.
async fn a_dials_b(
    l: &mut Listening,
    firewalled: bool,
    client_on: bool,
) -> (PeerConnection, PeerConnection) {
    let opts = connect_opts(firewalled, client_on);
    let client = l.a.connect(&l.a_key, l.b_key.public_key, opts).await;
    let client = client.expect("A connects to B");
    let next = tokio::time::timeout(Duration::from_secs(10), l.b.incoming.recv()).await;
    let accepted = next
        .expect("B accepted nothing in 10 s")
        .expect("channel closed");
    (client, accepted)
}

async fn echoes(client: PeerConnection, server: PeerConnection) -> bool {
    let bytes: Vec<u8> = (0..150_000usize).map(|i| (i % 251) as u8).collect();
    let expected = sha256(&bytes);
    let server_end = tokio::spawn(echo(server));
    let got = roundtrip(client, bytes).await;
    let _ = server_end.await;
    got == expected
}

mod accepted_direct_connection {
    use super::*;

    #[tokio::test]
    async fn accepted_direct_connection_adds_route_to_the_client() {
        within(async {
            // given: both nodes open, both reusable
            let mut l = both_listen(false, reusable(true), reusable(true)).await;
            // when: A connects to B
            let (_client, accepted) = a_dials_b(&mut l, false, true).await;
            // then: B keeps a route to A on the accepted socket
            let routes = l.b.handle.reuse.route_sockets(&l.a_key.public_key);
            assert!(
                routes.iter().any(|s| s.ptr_eq(&accepted.socket)),
                "B has no route to A on the accepted socket ({} records)",
                routes.len()
            );
        })
        .await;
    }
}

mod accepted_punched_connection {
    use super::*;

    #[tokio::test]
    async fn accepted_punched_connection_adds_route_on_the_held_socket() {
        within(async {
            // given: both nodes firewalled (punch), both reusable
            let mut l = both_listen(true, reusable(true), reusable(true)).await;
            // when: A connects to B
            let (_client, accepted) = a_dials_b(&mut l, true, true).await;
            // then: the socket B holds carries the route to A
            assert!(
                l.b.handle.reuse.is_held(&accepted.socket),
                "B does not hold the socket"
            );
            let routes = l.b.handle.reuse.route_sockets(&l.a_key.public_key);
            assert!(
                routes.iter().any(|s| s.ptr_eq(&accepted.socket)),
                "B has no route to A on the held socket ({} records)",
                routes.len()
            );
        })
        .await;
    }
}

mod punched_without_a_lease {
    use super::*;

    #[tokio::test]
    async fn punched_connection_without_a_lease_adds_no_route() {
        within(async {
            // given: both nodes firewalled; B's next hold fails (no lease)
            let mut l = both_listen(true, reusable(true), reusable(true)).await;
            l.b.handle.reuse.fail_next_hold();
            // when: A connects to B
            let (_client, accepted) = a_dials_b(&mut l, true, true).await;
            // then: B holds nothing and keeps no route (a record would have no reader)
            assert!(
                !l.b.handle.reuse.is_held(&accepted.socket),
                "B holds the socket"
            );
            let n = l.b.handle.reuse.route_sockets(&l.a_key.public_key).len();
            assert_eq!(n, 0, "route added on a socket nobody holds");
        })
        .await;
    }
}

mod either_side_not_reusable {
    use super::*;

    #[tokio::test]
    async fn no_route_when_either_side_is_not_reusable() {
        within(async {
            // given/when: the client does not announce reusable_socket
            let mut l = both_listen(true, reusable(true), reusable(true)).await;
            let (_c, _s) = a_dials_b(&mut l, true, false).await;
            // then: B keeps nothing
            let n = l.b.handle.reuse.route_sockets(&l.a_key.public_key).len();
            assert_eq!(n, 0, "route added although the client is not reusable");

            // given/when: the server has the option off
            let mut l = both_listen(true, reusable(true), reusable(false)).await;
            let (_c, _s) = a_dials_b(&mut l, true, true).await;
            // then: B keeps nothing
            let n = l.b.handle.reuse.route_sockets(&l.a_key.public_key).len();
            assert_eq!(n, 0, "route added although the server is not reusable");
        })
        .await;
    }
}

mod dial_back {
    use super::*;

    /// A -> B, then B dials A: the route is hit, the connection is direct, A took it as FROM_CLIENT.
    async fn dials_back(firewalled: bool) {
        within(async {
            let mut l = both_listen(firewalled, reusable(true), reusable(true)).await;
            let (_client, _accepted) = a_dials_b(&mut l, firewalled, true).await;
            let hits = l.b.handle.reuse.hits();
            let log = capture::start();
            let opts = connect_opts(firewalled, true);
            let back = l.b.connect(&l.b_key, l.a_key.public_key, opts).await;
            let back = back.expect("B dials A back");
            let incoming = tokio::time::timeout(Duration::from_secs(10), l.a.incoming.recv()).await;
            let at_a = incoming
                .expect("A accepted nothing in 10 s")
                .expect("channel closed");
            let lines = log.lines();
            assert_eq!(
                l.b.handle.reuse.hits(),
                hits + 1,
                "B did not take the route"
            );
            assert!(
                back.relay_path().is_none(),
                "the dial-back went through a relay"
            );
            assert!(
                lines
                    .iter()
                    .any(|l| l.contains("HANDLE_LOCALLY") && l.contains("inbound_mode=0")),
                "A did not take the handshake as mode 0: {lines:?}"
            );
            assert!(echoes(back, at_a).await, "echo over the dial-back failed");
        })
        .await;
    }

    #[tokio::test]
    async fn acceptor_dials_back_over_the_route() {
        dials_back(false).await;
    }

    #[tokio::test]
    async fn acceptor_dials_back_over_the_punched_route() {
        dials_back(true).await;
    }
}

mod capture {
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);

    impl Write for Sink {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("sink").extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Debug log of this thread (the test runtime is single-threaded) until dropped.
    pub(super) struct Log {
        sink: Sink,
        _guard: tracing::subscriber::DefaultGuard,
    }

    pub(super) fn start() -> Log {
        let sink = Sink::default();
        let writer = sink.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        Log {
            sink,
            _guard: tracing::subscriber::set_default(subscriber),
        }
    }

    impl Log {
        pub(super) fn lines(&self) -> Vec<String> {
            let bytes = self.sink.0.lock().expect("sink").clone();
            String::from_utf8_lossy(&bytes)
                .lines()
                .map(str::to_owned)
                .collect()
        }
    }
}

mod listener_is_gone {
    use super::*;

    #[tokio::test]
    async fn dial_back_to_a_peer_that_does_not_listen_falls_through() {
        within(async {
            // given: A does not listen (a plain client node); it connected to B once
            let mut l = both_listen(false, reusable(true), reusable(true)).await;
            let (client, _accepted) = a_dials_b(&mut l, false, true).await;
            l.a.server.close().await.ok();
            drop(client);
            // when: B dials A (the route leads to a socket nobody answers on)
            let hits = l.b.handle.reuse.hits();
            let started = std::time::Instant::now();
            let opts = connect_opts(false, true);
            let res = l.b.connect(&l.b_key, l.a_key.public_key, opts).await;
            // then: an ordinary "not found" error, soon
            assert!(
                matches!(res, Err(HyperDhtError::PeerNotFound)),
                "unexpected result: {:?}",
                res.map(|_| ())
            );
            assert_eq!(l.b.handle.reuse.hits(), hits + 1, "B did not try the route");
            assert!(
                started.elapsed() < Duration::from_secs(15),
                "dial-back hung"
            );
        })
        .await;
    }
}

mod linger {
    use super::*;

    #[tokio::test]
    async fn route_goes_when_the_incoming_connection_and_linger_end() {
        within(async {
            // given: a punched connection; B keeps a route on the held socket
            let mut l = both_listen(true, reusable(true), reusable(true)).await;
            let (client, accepted) = a_dials_b(&mut l, true, true).await;
            drop((client, accepted));
            // when: both ends closed and LINGER passed
            tokio::time::sleep(LINGER + Duration::from_secs(1)).await;
            // then: no record is left
            let n = l.b.handle.reuse.route_sockets(&l.a_key.public_key).len();
            assert_eq!(n, 0, "route outlived its socket");
        })
        .await;
    }
}
