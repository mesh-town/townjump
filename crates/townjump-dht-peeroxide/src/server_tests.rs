// Tests of the `server` module: PeeroxideServer. Without a network — `listen` on a destroyed node;
// live ones (`#[ignore]`, `-- --ignored`) — one key, firewall, `close` on the JS testnet (server.js, firewall.md).
use std::sync::Arc;
use std::time::Duration;
use townjump_dht::{Dht, DhtError, DhtServer, Firewall, ServerOpts};
use townjump_testkit::conformance::setup::{connect_pair, listen_on, node};
use townjump_testkit::conformance::wait::{ACCEPT_WAIT, assert_empty, connect_refused, within};
use townjump_testkit::conformance::{firewall_foreign_key, one_key_five_connects};
use townjump_testkit::key_pair;

// One helper file is included in three test files: a separate copy in each would be pointless.
#[allow(clippy::duplicate_mod)]
#[path = "support_tests.rs"]
mod support;

const OFFLINE_WAIT: Duration = Duration::from_secs(30);

mod listen_on_a_destroyed_node {
    use super::support::isolated_node;
    use super::*;

    // JS: server.js `listen` throws NODE_DESTROYED when `dht.destroyed`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fails_with_node_destroyed() {
        let node = isolated_node().await;
        let (server, _incoming) = node
            .create_server(ServerOpts::default())
            .expect("create_server");
        node.destroy().await.expect("destroy");
        let listened = within(OFFLINE_WAIT, "listen", server.listen(key_pair(0xb1))).await;
        assert!(
            matches!(listened, Err(DhtError::Destroyed)),
            "got {listened:?}"
        );
    }
}

mod live {
    use super::support::start_testnet;
    use super::*;

    mod conformance {
        use super::*;

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        #[ignore = "needs the JS testnet: node and .tmp/npm-holesail"]
        async fn five_connects_of_one_identity_give_five_incoming() {
            let testnet = start_testnet();
            one_key_five_connects(&testnet.factory).await;
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        #[ignore = "needs the JS testnet: node and .tmp/npm-holesail"]
        async fn foreign_key_is_refused_by_the_firewall_and_own_key_passes() {
            let testnet = start_testnet();
            firewall_foreign_key(&testnet.factory).await;
        }
    }

    mod firewall_callback_panics {
        use super::*;

        // firewall.md, test 5: a callback panic is a rejection, the server keeps working.
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        #[ignore = "needs the JS testnet: node and .tmp/npm-holesail"]
        async fn refuses_the_stranger_and_the_server_keeps_working() {
            let testnet = start_testnet();
            let own = key_pair(0x92).public_key;
            let mut server = listen_on(
                &testnet.factory,
                key_pair(0x91),
                Some(panicking_for_others(own)),
            )
            .await;
            let stranger = node(&testnet.factory, 0x93).await;
            connect_refused(&stranger, server.pk, "firewall with a panic: foreign key").await;
            assert_empty(&mut server.incoming, "firewall with a panic: foreign key").await;
            let friend = node(&testnet.factory, 0x92).await;
            connect_pair(&friend, own, &mut server, "firewall with a panic: own key").await;
        }
    }

    mod close {
        use super::*;

        // `close` withdraws the announce; `Incoming::accept` after close returns `None`.
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        #[ignore = "needs the JS testnet: node and .tmp/npm-holesail"]
        async fn ends_incoming_and_refuses_new_connects() {
            let testnet = start_testnet();
            let mut server = listen_on(&testnet.factory, key_pair(0x81), None).await;
            let client = node(&testnet.factory, 0x82).await;
            let client_pk = key_pair(0x82).public_key;
            connect_pair(&client, client_pk, &mut server, "close: before closing").await;
            within(ACCEPT_WAIT, "close", server.server.close())
                .await
                .expect("close");
            let next = within(ACCEPT_WAIT, "accept after close", server.incoming.accept()).await;
            assert!(
                next.is_none(),
                "after close the incoming queue must reach its end"
            );
            connect_refused(&client, server.pk, "close: after closing").await;
        }
    }
}

/// A firewall callback that panics on any key except `own`.
fn panicking_for_others(own: [u8; 32]) -> Firewall {
    Arc::new(move |pk| {
        if *pk != own {
            panic!("firewall callback panics on a foreign key");
        }
        false
    })
}
