// Tests of the connection metadata that `wrap_connection` and `Tunnels::register` hand out: `remote_addr` and `rtt`.
// A `PeerConnection` pair over UDX on 127.0.0.1 (pair_support_tests.rs), no DHT or JS.
use super::wrap_connection;
use crate::tunnels::Tunnels;
use std::net::SocketAddr;
use std::time::Duration;
use townjump_dht::DhtConnection;
use townjump_testkit::conformance::wait::within;

// One helper file is included in several test files: a separate copy in each would be pointless.
#[allow(clippy::duplicate_mod)]
#[path = "pair_support_tests.rs"]
mod pair;

/// A round trip on loopback must show up in the smoothed RTT within this.
const RTT_WAIT: Duration = Duration::from_secs(2);
const RTT_POLL: Duration = Duration::from_millis(10);

mod loopback_pair {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn wrapped_connection_keeps_the_remote_addr() {
        let addr: SocketAddr = "203.0.113.7:4242".parse().expect("address");
        let (mut a, b, _rt) = pair::pair_within().await;
        a.remote_addr = Some(addr);
        let (wrapped_a, wrapped_b) = (wrap_connection(a), wrap_connection(b));
        assert_eq!(wrapped_a.remote_addr(), Some(addr));
        assert_eq!(
            wrapped_b.remote_addr(),
            None,
            "the other end had no address"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn wrapped_connection_without_addr_has_none() {
        let (a, _b, _rt) = pair::pair_within().await;
        assert_eq!(wrap_connection(a).remote_addr(), None);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rtt_appears_after_a_round_trip() {
        let (a, b, _rt) = pair::pair_within().await;
        let (mut a, mut b) = (wrap_connection(a), wrap_connection(b));
        assert_eq!(a.rtt(), None, "no sample before any traffic");
        let rtt = rtt_after_traffic(&mut a, &mut b).await;
        assert!(rtt >= Duration::from_millis(1), "got {rtt:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rtt_is_none_after_the_tunnel_is_closed() {
        let tunnels = Tunnels::default();
        let (a, b, _rt) = pair::pair_within().await;
        let (mut a, mut b) = (tunnels.register(a), wrap_connection(b));
        rtt_after_traffic(&mut a, &mut b).await;
        tunnels.close_all();
        assert_eq!(a.rtt(), None, "the connection is gone");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rtt_probe_does_not_keep_the_tunnel_alive() {
        let (a, b, _rt) = pair::pair_within().await;
        let (mut a, mut b) = (wrap_connection(a), wrap_connection(b));
        rtt_after_traffic(&mut a, &mut b).await;
        let probe = a.rtt_probe().expect("a wrapped connection has a probe");
        assert!(probe().is_some(), "the probe reads the live connection");
        drop(a);
        assert_eq!(probe(), None, "the probe must not own the connection");
    }
}

/// Sends data `a` → `b` and back until the smoothed RTT of `a` shows up; fails if it takes longer than `RTT_WAIT`.
async fn rtt_after_traffic(a: &mut DhtConnection, b: &mut DhtConnection) -> Duration {
    let sample = async {
        loop {
            pair::carry(&mut a.stream, &mut b.stream, b"ping").await;
            pair::carry(&mut b.stream, &mut a.stream, b"pong").await;
            if let Some(rtt) = a.rtt() {
                return rtt;
            }
            tokio::time::sleep(RTT_POLL).await;
        }
    };
    within(RTT_WAIT, "smoothed RTT after a round trip", sample).await
}
