// Admission at the relay: `RelayOpts.firewall` reaches the relay service, so only listed node keys get paired.
// Per decision 3 the server goes to the relay under its node key, so the allow-list holds the server key and the client.
// Native network of the fork in this process, clients limited to the relay (`relay_only`).
use crate::native_testnet_tests::{
    DEFAULT_SIZE, NativeTestnet, assert_echo, echo_first, native_testnet, poll_stats, relay_only,
};
use crate::server::PeeroxideServer;
use crate::{PeeroxideDht, PeeroxideRelay, RelayOpts};
use std::sync::Arc;
use townjump_dht::{ConnectOpts, DhtConnection, DhtError, Firewall, Incoming, ServerOpts};
use townjump_testkit::key_pair;

const RELAY: u8 = 0xd1;
const SERVER: u8 = 0xd2;
const LISTED_CLIENT: u8 = 0xd3;
const OUTSIDER: u8 = 0xd4;

mod client_outside_the_list {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn connect_ends_with_an_error() {
        let f = setup().await;

        let (_node, result) = f.when_client_connects(OUTSIDER).await;

        assert!(result.is_err(), "the relay must not pair an unlisted key");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn no_pairing_is_counted() {
        let f = setup().await;

        let (_node, _result) = f.when_client_connects(OUTSIDER).await;

        // `sessions_active` is not checked here: the listed server legitimately opens its own sessions to the
        // relay for this attempt. "A refused key gets no session" is covered by the relay service admission tests.
        let stats = f.relay.stats();
        assert_eq!(stats.pairings_matched, 0, "{stats:?}");
    }
}

mod client_in_the_list {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn echoes_through_the_relay() {
        let f = setup().await;

        let (_node, result) = f.when_client_connects(LISTED_CLIENT).await;

        assert_echo(result.expect("a listed key is paired")).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn one_pairing_is_counted() {
        let f = setup().await;

        let (_node, result) = f.when_client_connects(LISTED_CLIENT).await;
        assert_echo(result.expect("a listed key is paired")).await;

        let stats = poll_stats(&f.relay, |s| s.pairings_matched >= 1).await;
        assert_eq!(stats.pairings_matched, 1, "{stats:?}");
    }
}

mod refused_client_followed_by_a_listed_one {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn only_the_listed_one_grows_the_pairing_counter() {
        let f = setup().await;
        let before = f.relay.stats().pairings_matched;

        let (_b, refused) = f.when_client_connects(OUTSIDER).await;
        let after_refused = f.relay.stats().pairings_matched;
        let (_a, listed) = f.when_client_connects(LISTED_CLIENT).await;
        assert_echo(listed.expect("the listed client is served after a refusal")).await;

        assert!(refused.is_err(), "the unlisted client must fail");
        assert_eq!(after_refused, before, "a refused key must not count");
        let stats = poll_stats(&f.relay, |s| s.pairings_matched > before).await;
        assert_eq!(stats.pairings_matched, before + 1, "{stats:?}");
    }
}

struct Fixture {
    net: NativeTestnet,
    relay: PeeroxideRelay,
    _server: (PeeroxideDht, PeeroxideServer),
}

/// A relay admitting the server node key and `LISTED_CLIENT` only; an echo server behind it, announced relay.
async fn setup() -> Fixture {
    let net = native_testnet(DEFAULT_SIZE).await;
    let allowed = vec![
        key_pair(SERVER).public_key,
        key_pair(LISTED_CLIENT).public_key,
    ];
    let opts = RelayOpts {
        identity: Some(key_pair(RELAY)),
        firewall: Some(allow_only(allowed)),
        ..Default::default()
    };
    let relay = net.relay_with(opts).await;
    net.wait_announced(&relay.public_key()).await;
    let (node, server, incoming): (PeeroxideDht, PeeroxideServer, Incoming) =
        net.listening(SERVER, ServerOpts::default()).await;
    echo_first(incoming);
    Fixture {
        net,
        relay,
        _server: (node, server),
    }
}

impl Fixture {
    /// A firewalled client on the key of `tag` asks the relay for the server; the node must outlive the result.
    async fn when_client_connects(
        &self,
        tag: u8,
    ) -> (PeeroxideDht, Result<DhtConnection, DhtError>) {
        let mut opts = ConnectOpts {
            relay_through: Some(self.relay.public_key().into()),
            ..Default::default()
        };
        relay_only(&mut opts);
        let server = key_pair(SERVER).public_key;
        self.net.connect_from(tag, server, opts).await
    }
}

/// Firewall callback: `true` = refuse everything outside `keys`.
fn allow_only(keys: Vec<[u8; 32]>) -> Firewall {
    Arc::new(move |key| !keys.contains(key))
}
