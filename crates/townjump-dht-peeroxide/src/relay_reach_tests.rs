// Relay reachability: its own port, the self-check (`assume_open = false`) that ends in `Open` on the testnet, the
// published address, and a relay without self-announce that is reached by address only.
// Native network of the fork in this process, clients limited to the relay (`relay_only`).
use crate::native_testnet_tests::{
    DEFAULT_SIZE, NativeTestnet, free_udp_port, lan_host, native_testnet, poll_stats,
};
use crate::{NatKind, PeeroxideRelay, RelayOpts, RelayReachability};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::{Duration, Instant};
use townjump_dht::{ConnectOpts, RelayAddress, ServerOpts};
use townjump_testkit::key_pair;

const RELAY: u8 = 0xd1;
const SERVER: u8 = 0xd2;
const CLIENT: u8 = 0xd3;
const CHECK_WAIT: Duration = Duration::from_secs(30);
const CHECK_STEP: Duration = Duration::from_millis(500);

mod chosen_port {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn is_the_port_of_the_relay() {
        let net = native_testnet(DEFAULT_SIZE).await;
        let port = free_udp_port();

        let relay = net
            .relay_with(RelayOpts {
                port,
                ..Default::default()
            })
            .await;

        assert_eq!(relay.local_port(), port);
        relay.close().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn is_the_port_in_reachability() {
        let net = native_testnet(DEFAULT_SIZE).await;
        let port = free_udp_port();
        let relay = net
            .relay_with(RelayOpts {
                port,
                ..Default::default()
            })
            .await;

        let reach = relay.reachability().await.expect("reachability");

        assert_eq!(reach.local_port, port);
        relay.close().await;
    }
}

mod zero_port {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn picks_a_real_port_and_reports_it() {
        let net = native_testnet(DEFAULT_SIZE).await;

        let relay = net.relay_with(RelayOpts::default()).await;

        assert_ne!(relay.local_port(), 0, "the real port, not the request");
        let reach = relay.reachability().await.expect("reachability");
        assert_eq!(reach.local_port, relay.local_port());
        relay.close().await;
    }
}

mod assumed_open_relay {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn is_open_at_once() {
        let net = native_testnet(DEFAULT_SIZE).await;
        let relay = net.relay_with(RelayOpts::default()).await;

        let reach = relay.reachability().await.expect("reachability");

        assert_eq!(reach.firewall, NatKind::Open);
        relay.close().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn public_address_is_its_host_and_port() {
        let net = native_testnet(DEFAULT_SIZE).await;
        let relay = net.relay_with(RelayOpts::default()).await;

        let reach = relay.reachability().await.expect("reachability");

        assert_eq!(reach.public_address, Some(relay_host(relay.local_port())));
        relay.close().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn relay_address_carries_relay_key_and_port() {
        let net = native_testnet(DEFAULT_SIZE).await;
        let relay = net.relay_with(opts_with_key()).await;

        let address = relay.relay_address().await.expect("relay_address");

        let expected = RelayAddress {
            public_key: key_pair(RELAY).public_key,
            addr: relay_host(relay.local_port()),
        };
        assert_eq!(address, Some(expected));
        relay.close().await;
    }
}

mod checked_relay {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn is_not_open_right_after_start() {
        let f = setup_checked().await;

        let reach = f.relay.reachability().await.expect("reachability");

        assert_ne!(
            reach.firewall,
            NatKind::Open,
            "proof takes ticks, not zero time"
        );
        assert_eq!(reach.public_address, None, "no address before the proof");
        assert_eq!(reach.local_port, f.relay.local_port());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn has_no_relay_address_before_the_proof() {
        let f = setup_checked().await;

        let address = f.relay.relay_address().await.expect("relay_address");

        assert_eq!(address, None);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn becomes_open_on_the_testnet_within_30_seconds() {
        let f = setup_checked().await;

        let reach = f.when_check_ends().await;

        assert_eq!(
            reach.firewall,
            NatKind::Open,
            "the self-check did not end in Open: {reach:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn then_publishes_relay_key_and_its_port() {
        let f = setup_checked().await;
        f.when_check_ends().await;

        let address = f.relay.relay_address().await.expect("relay_address");

        let expected = RelayAddress {
            public_key: key_pair(RELAY).public_key,
            addr: relay_host(f.relay.local_port()),
        };
        assert_eq!(address, Some(expected));
    }
}

mod relay_without_self_announce {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn is_not_found_in_the_network() {
        let f = setup_unannounced().await;

        let found = f.net.is_announced(&f.relay.public_key()).await;

        assert!(!found, "announce = false must not leave an announcement");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn still_pairs_peers_that_know_its_address() {
        let f = setup_unannounced().await;
        let client = ConnectOpts {
            relay_through: Some(f.address.public_key.into()),
            relay_addresses: vec![f.address],
            ..Default::default()
        };
        let server = ServerOpts {
            relay_addresses: vec![f.address],
            ..Default::default()
        };

        f.net.echo_via(SERVER, CLIENT, server, client).await;

        let stats = poll_stats(&f.relay, |s| s.pairings_matched >= 1).await;
        assert!(
            stats.pairings_matched >= 1,
            "the couple must pair at the relay: {stats:?}"
        );
        assert!(
            !f.net.is_announced(&f.relay.public_key()).await,
            "still not announced"
        );
    }
}

struct Checked {
    _net: NativeTestnet,
    relay: PeeroxideRelay,
}

/// A relay that has to prove it is reachable (`assume_open = false`).
async fn setup_checked() -> Checked {
    let net = native_testnet(DEFAULT_SIZE).await;
    let opts = RelayOpts {
        assume_open: false,
        ..opts_with_key()
    };
    let relay = net.relay_with(opts).await;
    Checked { _net: net, relay }
}

impl Checked {
    /// Polls `reachability()` every 500 ms until `Open` or 30 s; returns the last answer.
    async fn when_check_ends(&self) -> RelayReachability {
        let deadline = Instant::now() + CHECK_WAIT;
        loop {
            let reach = self.relay.reachability().await.expect("reachability");
            if reach.firewall == NatKind::Open || Instant::now() >= deadline {
                return reach;
            }
            tokio::time::sleep(CHECK_STEP).await;
        }
    }
}

struct Unannounced {
    net: NativeTestnet,
    relay: PeeroxideRelay,
    address: RelayAddress,
}

/// An assumed-open relay on the testnet that does not announce itself, and the address it publishes.
async fn setup_unannounced() -> Unannounced {
    let net = native_testnet(DEFAULT_SIZE).await;
    let opts = RelayOpts {
        announce: false,
        ..opts_with_key()
    };
    let relay = net.relay_with(opts).await;
    let address = relay.relay_address().await.expect("relay_address");
    let address = address.expect("an assumed-open relay on the testnet has an address");
    Unannounced {
        net,
        relay,
        address,
    }
}

fn opts_with_key() -> RelayOpts {
    RelayOpts {
        identity: Some(key_pair(RELAY)),
        ..Default::default()
    }
}

/// The address a test relay binds (`relay_try`: the LAN address of this machine) with `port`.
fn relay_host(port: u16) -> SocketAddrV4 {
    let host: Ipv4Addr = lan_host().parse().expect("LAN IPv4 address");
    SocketAddrV4::new(host, port)
}
