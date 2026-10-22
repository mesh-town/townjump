// Tests of the `dht` module: the PeeroxideDht node. Without a network — `destroy`, `suspend`/`resume` on a node without peers;
// live ones (`#[ignore]`, `-- --ignored`) — the common conformance suite, a DHT record and bootstrap addresses on the JS testnet.
use super::*;
use crate::factory::PeeroxideFactory;
use std::time::Duration;
use townjump_dht::{ConnectOpts, DhtFactory};
use townjump_testkit::conformance::wait::{ACCEPT_WAIT, REFUSE_WAIT, within};
use townjump_testkit::conformance::{mutable_record_seq, suspend_resume};
use townjump_testkit::key_pair;

// One helper file is included in three test files: a separate copy in each would be pointless.
#[allow(clippy::duplicate_mod)]
#[path = "support_tests.rs"]
mod support;

const OFFLINE_WAIT: Duration = Duration::from_secs(30);

mod isolated_node {
    use super::support::isolated_node;
    use super::*;

    // JS (testvectors/probes-js/server-facade-dht.cjs): `new HyperDHT({ bootstrap: ['127.0.0.1:1'] })` returns in 0.2 ms — `create` does not wait for bootstrap.
    mod create_then_destroy {
        use super::*;

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn succeeds_without_any_peer() {
            let node = isolated_node().await;
            let destroyed = within(OFFLINE_WAIT, "destroy", node.destroy()).await;
            assert!(destroyed.is_ok(), "destroy without peers: {destroyed:?}");
        }
    }

    mod suspend_then_resume {
        use super::*;

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn both_succeed_on_an_idle_node() {
            let node = isolated_node().await;
            let suspended = within(OFFLINE_WAIT, "suspend", node.suspend()).await;
            assert!(suspended.is_ok(), "suspend: {suspended:?}");
            let resumed = within(OFFLINE_WAIT, "resume", node.resume()).await;
            assert!(resumed.is_ok(), "resume: {resumed:?}");
        }
    }
}

mod node_mode {
    use super::support::isolated_node;
    use super::*;
    use townjump_dht::{Dht, DhtError};

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn isolated_node_is_not_persistent() {
        let node = isolated_node().await;
        assert!(!node.node_mode().persistent);
        let _ = node.destroy().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn set_adaptive_false_is_visible_in_the_mode() {
        let node = isolated_node().await;
        let set = within(OFFLINE_WAIT, "set_adaptive", node.set_adaptive(false)).await;
        assert!(set.is_ok(), "set_adaptive(false): {set:?}");
        assert!(!node.node_mode().adaptive);
        let _ = node.destroy().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn destroyed_node_refuses_mode_calls() {
        let node = isolated_node().await;
        let mode = node.node_mode();
        node.destroy().await.expect("destroy");
        let set = node.set_adaptive(false).await;
        assert!(matches!(set, Err(DhtError::Destroyed)), "{set:?}");
        let changed = node.mode_changed(mode).await;
        assert!(matches!(changed, Err(DhtError::Destroyed)), "{changed:?}");
    }
}

mod table_export {
    use super::support::isolated_node;
    use townjump_dht::{Dht, DhtError};

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn isolated_node_exports_empty() {
        let node = isolated_node().await;
        assert!(node.to_array(None).await.expect("export").is_empty());
        let _ = node.destroy().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn destroyed_node_refuses_export() {
        let node = isolated_node().await;
        node.destroy().await.expect("destroy");
        let got = node.to_array(None).await;
        assert!(matches!(got, Err(DhtError::Destroyed)), "{got:?}");
    }
}

mod live {
    use super::support::start_testnet;
    use super::*;

    mod conformance {
        use super::*;

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        #[ignore = "needs the JS testnet: node and .tmp/npm-holesail"]
        async fn mutable_record_seq_is_read_back_by_another_node() {
            let testnet = start_testnet();
            mutable_record_seq(&testnet.factory).await;
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        #[ignore = "needs the JS testnet: node and .tmp/npm-holesail"]
        async fn sleeping_node_refuses_and_resumed_node_accepts() {
            let testnet = start_testnet();
            suspend_resume(&testnet.factory).await;
        }
    }

    mod suspend_and_resume {
        use super::*;

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        #[ignore = "needs the JS testnet: node and .tmp/npm-holesail"]
        async fn answer_ok_and_the_node_still_writes_records_afterwards() {
            let testnet = start_testnet();
            let node = created(&testnet.factory, 0xa1).await;
            within(ACCEPT_WAIT, "suspend", node.suspend())
                .await
                .expect("suspend");
            within(ACCEPT_WAIT, "resume", node.resume())
                .await
                .expect("resume");
            let writer_pair = key_pair(0xa1);
            let put = node.mutable_put(&writer_pair, b"after resume", Some(1));
            assert_eq!(within(ACCEPT_WAIT, "put", put).await.expect("put"), 1);
            let reader = created(&testnet.factory, 0xa2).await;
            let got = within(
                ACCEPT_WAIT,
                "get",
                reader.mutable_get(key_pair(0xa1).public_key, None),
            );
            assert_eq!(got.await.expect("get"), record(1, b"after resume"));
        }
    }

    mod connect_to_a_key_nobody_serves {
        use super::*;

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        #[ignore = "needs the JS testnet: node and .tmp/npm-holesail"]
        async fn fails_with_peer_not_found() {
            let testnet = start_testnet();
            let node = created(&testnet.factory, 0xa3).await;
            let dial = node.connect(key_pair(0xa4).public_key, ConnectOpts::default());
            let refused = within(REFUSE_WAIT, "connect", dial).await;
            let error = refused.expect_err("connect to a key without a server must fail");
            assert!(matches!(error, DhtError::PeerNotFound), "got {error:?}");
            assert_eq!(error.to_string(), "PEER_NOT_FOUND: Peer not found");
        }
    }

    // If all bootstrap addresses are loopback, the host is `127.0.0.1` (the testnet listens only on it).
    mod bootstrap_host {
        use super::*;

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        #[ignore = "needs the JS testnet: node and .tmp/npm-holesail"]
        async fn localhost_reaches_the_testnet() {
            assert_record_travels_through_host("localhost").await;
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        #[ignore = "needs the JS testnet: node and .tmp/npm-holesail"]
        async fn unspecified_address_reaches_the_testnet() {
            assert_record_travels_through_host("0.0.0.0").await;
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        #[ignore = "needs the JS testnet: node and .tmp/npm-holesail"]
        async fn ipv6_loopback_reaches_the_testnet() {
            assert_record_travels_through_host("[::1]").await;
        }
    }
}

fn record(seq: u64, value: &[u8]) -> Option<townjump_dht::MutableRecord> {
    Some(townjump_dht::MutableRecord {
        seq,
        value: value.to_vec(),
    })
}

/// A node with a key pair by label on the testnet factory.
async fn created(factory: &PeeroxideFactory, tag: u8) -> PeeroxideDht {
    let made = within(ACCEPT_WAIT, "create", factory.create(Some(key_pair(tag)))).await;
    made.expect("create")
}

/// Port of the first testnet bootstrap address (`127.0.0.1:<port>`).
fn testnet_port(factory: &PeeroxideFactory) -> String {
    let first = &factory.bootstrap.as_ref().expect("testnet bootstrap")[0];
    first.rsplit_once(':').expect("host:port").1.to_string()
}

/// A node whose bootstrap is the testnet port with host `host` writes a record; a node on the plain testnet address reads it.
async fn assert_record_travels_through_host(host: &str) {
    let testnet = support::start_testnet();
    let via_host = PeeroxideFactory {
        bootstrap: Some(vec![format!("{host}:{}", testnet_port(&testnet.factory))]),
        ..Default::default()
    };
    let writer = created(&via_host, 0xa5).await;
    let writer_pair = key_pair(0xa5);
    let put = writer.mutable_put(&writer_pair, b"via host", Some(1));
    within(ACCEPT_WAIT, "put via the host", put)
        .await
        .expect("put");
    let reader = created(&testnet.factory, 0xa6).await;
    let got = reader.mutable_get(key_pair(0xa5).public_key, None);
    let got = within(ACCEPT_WAIT, "get", got).await.expect("get");
    assert_eq!(
        got,
        record(1, b"via host"),
        "record put via bootstrap {host}"
    );
}
