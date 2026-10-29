// Relay lifetime: `close` and `Drop` stop the node and free its port, so a relay can be recreated on the same key
// and on the same port. Native network of the fork in this process, clients limited to the relay (`relay_only`).
use crate::native_testnet_tests::{
    DEFAULT_SIZE, NativeTestnet, free_udp_port, native_testnet, poll_stats,
};
use crate::{PeeroxideRelay, RelayOpts};
use std::time::{Duration, Instant};
use townjump_dht::{ConnectOpts, ServerOpts};
use townjump_testkit::key_pair;

const RELAY: u8 = 0xd1;
const FIRST_SERVER: u8 = 0xd2;
const FIRST_CLIENT: u8 = 0xd3;
const SECOND_SERVER: u8 = 0xd4;
const SECOND_CLIENT: u8 = 0xd5;
const FREE_PORT_WAIT: Duration = Duration::from_secs(2);
const RETRY_STEP: Duration = Duration::from_millis(100);

mod relay_closed_and_recreated_on_the_same_key {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn keeps_its_public_key() {
        let f = setup().await;

        let key = f.second.public_key();

        assert_eq!(key, key_pair(RELAY).public_key);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pairs_a_new_couple() {
        let f = setup().await;

        f.when_new_couple_talks().await;

        let stats = poll_stats(&f.second, |s| s.pairings_matched >= 1).await;
        assert!(
            stats.pairings_matched >= 1,
            "the new relay must pair: {stats:?}"
        );
    }
}

mod relay_closed_and_recreated_on_the_same_port {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn starts_right_after_close_on_that_port() {
        let net = native_testnet(DEFAULT_SIZE).await;
        let port = free_udp_port();
        let first = net.relay_with(on_port(port)).await;
        assert_eq!(first.local_port(), port, "the chosen port is used");

        first.close().await;
        let second = net.relay_with(on_port(port)).await;

        assert_eq!(second.local_port(), port);
        second.close().await;
    }
}

mod relay_dropped {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn frees_its_port_within_two_seconds() {
        let net = native_testnet(DEFAULT_SIZE).await;
        let port = free_udp_port();
        let relay = net.relay_with(on_port(port)).await;

        drop(relay);

        let again = recreate_until_free(&net, port).await;
        let again = again.expect("the port stays busy 2 s after the drop");
        assert_eq!(again.local_port(), port);
        again.close().await;
    }
}

mod relay_on_a_busy_port {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn start_is_an_error() {
        let net = native_testnet(DEFAULT_SIZE).await;
        let first = net.relay_with(RelayOpts::default()).await;

        let second = net.relay_try(on_port(first.local_port())).await;

        assert!(
            second.is_err(),
            "a second relay on a taken port must not start"
        );
        first.close().await;
    }
}

struct Restarted {
    net: NativeTestnet,
    second: PeeroxideRelay,
}

/// A relay on a persistent key serves one couple and is closed; a new relay starts on the same key.
async fn setup() -> Restarted {
    let net = native_testnet(DEFAULT_SIZE).await;
    let first = net.relay_with(on_key()).await;
    net.wait_announced(&first.public_key()).await;
    net.echo_via(
        FIRST_SERVER,
        FIRST_CLIENT,
        ServerOpts::default(),
        via(&first),
    )
    .await;
    first.close().await;
    let second = net.relay_with(on_key()).await;
    net.wait_announced(&second.public_key()).await;
    Restarted { net, second }
}

impl Restarted {
    /// A server and a client that have not seen the first relay talk through the new one.
    async fn when_new_couple_talks(&self) {
        let opts = via(&self.second);
        let server = ServerOpts::default();
        self.net
            .echo_via(SECOND_SERVER, SECOND_CLIENT, server, opts)
            .await;
    }
}

fn on_key() -> RelayOpts {
    RelayOpts {
        identity: Some(key_pair(RELAY)),
        ..Default::default()
    }
}

fn on_port(port: u16) -> RelayOpts {
    RelayOpts {
        port,
        ..Default::default()
    }
}

/// Client options asking `relay` to pair.
fn via(relay: &PeeroxideRelay) -> ConnectOpts {
    ConnectOpts {
        relay_through: Some(relay.public_key().into()),
        ..Default::default()
    }
}

/// Tries to start a relay on `port` every 100 ms until it works or `FREE_PORT_WAIT` has passed.
async fn recreate_until_free(net: &NativeTestnet, port: u16) -> Option<PeeroxideRelay> {
    let deadline = Instant::now() + FREE_PORT_WAIT;
    loop {
        match net.relay_try(on_port(port)).await {
            Ok(relay) => return Some(relay),
            Err(_) if Instant::now() >= deadline => return None,
            Err(_) => tokio::time::sleep(RETRY_STEP).await,
        }
    }
}
