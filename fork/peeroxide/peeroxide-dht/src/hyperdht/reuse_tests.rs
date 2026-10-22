// Tests of the held-socket and route store: linger timer, route table, datagram forwarder.
// JS: hyperdht/lib/socket-pool.js
//
// Time tests run on paused tokio time (`sleep` jumps the clock); the forwarder tests need a real
// network wait, so they run unpaused and wait on the channel with a timeout.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use libudx::{UdxRuntime, UdxSocket};
use tokio::sync::mpsc::{self, UnboundedReceiver};
use tokio::time::{sleep, timeout};

use super::{Forward, LINGER, Lease, Reuse};
use crate::messages::Ipv4Peer;

const KEY: [u8; 32] = [1; 32];
const OTHER_KEY: [u8; 32] = [2; 32];
const ONE_MS: Duration = Duration::from_millis(1);
const ONE_SEC: Duration = Duration::from_secs(1);

mod linger {
    use super::*;

    mod last_lease_released {
        use super::*;

        #[tokio::test(start_paused = true)]
        async fn socket_and_route_stay_until_linger_ends() {
            // given
            let fx = setup();
            let (s, lease) = fx.hold_with_route(KEY).await;
            // when
            drop(lease);
            sleep(LINGER - ONE_MS).await;
            // then
            assert!(fx.reuse.is_held(&s), "socket must live until LINGER ends");
            assert!(
                fx.reuse.route(&KEY).is_some(),
                "route lives with its socket"
            );
        }

        #[tokio::test(start_paused = true)]
        async fn socket_and_route_go_when_linger_ends() {
            // given
            let fx = setup();
            let (s, lease) = fx.hold_with_route(KEY).await;
            // when
            drop(lease);
            sleep(LINGER + ONE_MS).await;
            // then
            assert!(!fx.reuse.is_held(&s), "socket must be closed after LINGER");
            assert!(
                fx.reuse.lease(&s).is_none(),
                "an expired socket cannot be leased"
            );
            assert!(
                fx.reuse.route(&KEY).is_none(),
                "routes go with their socket"
            );
        }
    }

    mod lease_taken_during_linger {
        use super::*;

        #[tokio::test(start_paused = true)]
        async fn cancels_expiry() {
            // given
            let fx = setup();
            let (s, lease) = fx.hold_with_route(KEY).await;
            drop(lease);
            sleep(ONE_SEC).await;
            // when
            let relet = fx
                .reuse
                .lease(&s)
                .expect("socket is still held during LINGER");
            sleep(5 * ONE_SEC).await;
            // then
            assert!(fx.reuse.is_held(&s), "a leased socket must not expire");
            assert!(
                fx.reuse.route(&KEY).is_some(),
                "route stays with the leased socket"
            );
            drop(relet);
        }
    }

    mod lease_taken_and_released_during_linger {
        use super::*;

        #[tokio::test(start_paused = true)]
        async fn first_timer_does_not_expire_the_socket() {
            // given: drop at 0 (timer A at 3 s), lease and its drop at 1 s (timer B at 4 s)
            let fx = setup();
            let (s, lease) = fx.hold_with_route(KEY).await;
            drop(lease);
            sleep(ONE_SEC).await;
            drop(
                fx.reuse
                    .lease(&s)
                    .expect("socket is still held during LINGER"),
            );
            // when: timer A fired at 3.0 s
            sleep(LINGER - ONE_SEC + ONE_MS).await;
            // then
            assert!(
                fx.reuse.is_held(&s),
                "stale timer of the first release must do nothing"
            );
            // and timer B removes the entry at 4.0 s
            sleep(ONE_SEC).await;
            assert!(
                !fx.reuse.is_held(&s),
                "the latest release must expire the socket"
            );
        }
    }

    mod two_leases_one_released {
        use super::*;

        #[tokio::test(start_paused = true)]
        async fn socket_stays() {
            // given
            let fx = setup();
            let (s, first) = fx.hold_with_route(KEY).await;
            let second = fx.reuse.lease(&s).expect("lease of held socket");
            // when
            drop(first);
            sleep(LINGER + ONE_SEC).await;
            // then
            assert!(
                fx.reuse.is_held(&s),
                "the second lease must keep the socket"
            );
            assert!(fx.reuse.route(&KEY).is_some());
            drop(second);
        }
    }

    mod hold_twice {
        use super::*;

        #[tokio::test(start_paused = true)]
        async fn counts_two_leases() {
            // given
            let fx = setup();
            let s = fx.socket().await;
            let first = fx.reuse.hold(&s).expect("first hold");
            let second = fx.reuse.hold(&s).expect("second hold of the same socket");
            // when
            drop(first);
            sleep(4 * ONE_SEC).await;
            // then
            assert!(fx.reuse.is_held(&s), "the second hold is a second lease");
            drop(second);
        }

        #[tokio::test(start_paused = true)]
        async fn expires_after_both_released() {
            // given
            let fx = setup();
            let s = fx.socket().await;
            let first = fx.reuse.hold(&s).expect("first hold");
            let second = fx.reuse.hold(&s).expect("second hold of the same socket");
            // when
            drop(first);
            drop(second);
            sleep(LINGER + ONE_MS).await;
            // then
            assert!(!fx.reuse.is_held(&s));
        }
    }

    mod two_sockets {
        use super::*;

        #[tokio::test(start_paused = true)]
        async fn expire_independently() {
            // given
            let fx = setup();
            let (s1, lease1) = fx.hold_with_route(KEY).await;
            let (s2, lease2) = fx.hold_with_route(OTHER_KEY).await;
            // when
            drop(lease1);
            sleep(LINGER + ONE_MS).await;
            // then
            assert!(!fx.reuse.is_held(&s1), "the released socket expires");
            assert!(fx.reuse.route(&KEY).is_none());
            assert!(fx.reuse.is_held(&s2), "the other socket is held separately");
            assert!(
                fx.reuse.route(&OTHER_KEY).is_some(),
                "its route must survive"
            );
            drop(lease2);
        }
    }

    mod released_outside_runtime {
        use super::*;

        #[test]
        fn expires_now() {
            // given: runtime made by hand, hold inside block_on
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            let fx = setup();
            let (s, lease) = rt.block_on(fx.hold_with_route(KEY));
            // when: drop outside the runtime: the timer has no runtime to run on
            drop(lease);
            // then
            assert!(
                !fx.reuse.is_held(&s),
                "no runtime for a timer: expire at once"
            );
            assert!(fx.reuse.route(&KEY).is_none());
        }
    }

    mod reuse_dropped {
        use super::*;

        #[tokio::test(start_paused = true)]
        async fn pending_timer_does_not_keep_the_store() {
            // given: lease released, the timer waits
            let fx = setup();
            let (_s, lease) = fx.hold_with_route(KEY).await;
            drop(lease);
            sleep(10 * ONE_MS).await;
            let store = Arc::downgrade(&fx.reuse.inner);
            // when
            drop(fx.reuse);
            // then
            assert!(store.upgrade().is_none(), "the timer must hold only a Weak");
        }

        #[tokio::test(start_paused = true)]
        async fn forwarder_does_not_keep_the_store() {
            // given: lease alive, the forwarder waits for datagrams
            let fx = setup();
            let (_s, lease) = fx.hold_with_route(KEY).await;
            sleep(10 * ONE_MS).await;
            let store = Arc::downgrade(&fx.reuse.inner);
            // when
            drop(fx.reuse);
            // then
            assert!(
                store.upgrade().is_none(),
                "the forwarder must hold only a Weak"
            );
            drop(lease);
        }
    }
}

mod unfit_sockets {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn lease_of_unheld_socket_is_none() {
        let fx = setup();
        let s = fx.socket().await;
        assert!(
            fx.reuse.lease(&s).is_none(),
            "only held sockets can be leased"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn hold_of_unbound_socket_is_none() {
        // given: without bind recv_start fails
        let fx = setup();
        let s = fx.runtime.create_socket().await.expect("create socket");
        // when
        let lease = fx.reuse.hold(&s);
        // then
        assert!(lease.is_none());
        assert!(!fx.reuse.is_held(&s));
    }
}

mod socket_identity {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn ptr_eq_tells_sockets_apart() {
        let fx = setup();
        let (s, other) = (fx.socket().await, fx.socket().await);
        assert!(s.clone().ptr_eq(&s), "a clone is the same socket");
        assert!(!s.ptr_eq(&other), "another socket is not");
    }
}

#[path = "reuse_forwarder_tests.rs"]
mod forwarder;
#[path = "reuse_routes_tests.rs"]
mod routes;

type Forwarded = (SocketAddr, Vec<u8>, UdxSocket);

struct Fixture {
    reuse: Reuse,
    runtime: UdxRuntime,
    forwarded: UnboundedReceiver<Forwarded>,
}

fn setup() -> Fixture {
    let (tx, forwarded) = mpsc::unbounded_channel();
    let forward: Forward = Arc::new(move |addr, data, socket| {
        let _ = tx.send((addr, data, socket));
    });
    Fixture {
        reuse: Reuse::new(forward),
        runtime: UdxRuntime::new().expect("udx runtime"),
        forwarded,
    }
}

fn peer(port: u16) -> Ipv4Peer {
    Ipv4Peer {
        host: "127.0.0.1".into(),
        port,
    }
}

impl Fixture {
    async fn socket(&self) -> UdxSocket {
        let socket = self.runtime.create_socket().await.expect("create socket");
        let addr = "127.0.0.1:0".parse().expect("loopback addr");
        socket.bind(addr).await.expect("bind");
        socket
    }

    /// A fresh bound socket, held; the lease must outlive the check.
    async fn held(&self) -> (UdxSocket, Lease) {
        let socket = self.socket().await;
        let lease = self.reuse.hold(&socket).expect("hold of a bound socket");
        (socket, lease)
    }

    async fn hold_with_route(&self, key: [u8; 32]) -> (UdxSocket, Lease) {
        let (socket, lease) = self.held().await;
        self.reuse.add_route(key, &socket, peer(4001));
        (socket, lease)
    }

    async fn send(&self, from: &UdxSocket, to: &UdxSocket, data: &[u8]) {
        let addr = to.local_addr().await.expect("target addr");
        from.send_to(data, addr).expect("send datagram");
    }

    async fn next_forwarded(&mut self) -> Forwarded {
        timeout(Duration::from_secs(2), self.forwarded.recv())
            .await
            .expect("a datagram forwarded within 2 s")
            .expect("forward channel open")
    }

    async fn nothing_forwarded_for(&mut self, window: Duration) -> bool {
        timeout(window, self.forwarded.recv()).await.is_err()
    }
}
