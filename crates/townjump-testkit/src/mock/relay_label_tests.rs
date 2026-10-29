// Tests of the relay path label in the mock: `relay_through` on either side labels both ends `Relayed`;
// no pairing, no admission, never an upgrade to direct.
use crate::conformance::key_pair;
use crate::mock::test_util::{node, serve_with};
use crate::mock::{MockDht, MockFactory, MockServer};
use std::time::Duration;
use townjump_dht::{
    ConnectOpts, Dht, DhtConnection, Incoming, RelayPath, RelayThrough, ServerOpts,
};

const SERVER_TAG: u8 = 0x91;
const RELAY_KEY: [u8; 32] = [9; 32];
const QUIET: Duration = Duration::from_millis(100);

mod relay_through_on_the_client {
    use super::*;

    #[tokio::test]
    async fn both_ends_are_relayed() {
        let Pair {
            client, accepted, ..
        } = connect(Side::Client).await;

        assert!(client.is_relayed(), "client end");
        assert!(accepted.is_relayed(), "end taken from Incoming");
    }
}

mod relay_through_on_the_server {
    use super::*;

    #[tokio::test]
    async fn both_ends_are_relayed() {
        let Pair {
            client, accepted, ..
        } = connect(Side::Server).await;

        assert!(client.is_relayed(), "client end");
        assert!(accepted.is_relayed(), "end taken from Incoming");
    }
}

mod relay_through_on_both_sides {
    use super::*;

    #[tokio::test]
    async fn both_ends_are_relayed() {
        let Pair {
            client, accepted, ..
        } = connect(Side::Both).await;

        assert!(client.is_relayed(), "client end");
        assert!(accepted.is_relayed(), "end taken from Incoming");
    }
}

mod without_relay_through {
    use super::*;

    #[tokio::test]
    async fn neither_end_has_a_label() {
        let Pair {
            client, accepted, ..
        } = connect(Side::Neither).await;

        assert!(client.relay_path().is_none(), "client end");
        assert!(accepted.relay_path().is_none(), "end taken from Incoming");
        assert!(!client.is_relayed());
        assert!(!accepted.is_relayed());
    }
}

mod the_label_stays {
    use super::*;

    #[tokio::test]
    async fn relayed_and_never_changes_to_direct() {
        let Pair {
            client, accepted, ..
        } = connect(Side::Client).await;

        tokio::time::sleep(QUIET).await;

        for (end, conn) in [("client", &client), ("accepted", &accepted)] {
            let rx = conn.relay_path().expect("labelled");
            assert!(!rx.has_changed().unwrap_or(true), "{end} end changed");
            assert_eq!(*rx.borrow(), RelayPath::Relayed, "{end} end");
        }
    }
}

#[derive(Clone, Copy)]
enum Side {
    Client,
    Server,
    Both,
    Neither,
}

impl Side {
    fn client_asks(self) -> bool {
        matches!(self, Side::Client | Side::Both)
    }

    fn server_asks(self) -> bool {
        matches!(self, Side::Server | Side::Both)
    }
}

/// Both ends of one connection; the server stays alive as long as the pair does.
struct Pair {
    client: DhtConnection,
    accepted: DhtConnection,
    _server: MockServer,
    _incoming: Incoming,
    _nodes: (MockDht, MockDht),
}

/// One connection in the mock network; `side` says who sets `relay_through`.
async fn connect(side: Side) -> Pair {
    let factory = MockFactory::new();
    let (owner, caller) = (node(&factory, 1).await, node(&factory, 2).await);
    let server_opts = ServerOpts {
        relay_through: side.server_asks().then_some(RelayThrough::Key(RELAY_KEY)),
        ..Default::default()
    };
    let (server, mut incoming) = serve_with(&owner, SERVER_TAG, server_opts).await;
    let opts = ConnectOpts {
        relay_through: side.client_asks().then_some(RelayThrough::Key(RELAY_KEY)),
        ..Default::default()
    };

    let client = caller.connect(key_pair(SERVER_TAG).public_key, opts).await;
    let accepted = incoming.accept().await;

    Pair {
        client: client.expect("connect"),
        accepted: accepted.expect("the server got the call"),
        _server: server,
        _incoming: incoming,
        _nodes: (owner, caller),
    }
}
