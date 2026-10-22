// Node lifecycle without a network (a node without peers): repeated `destroy`, rejections after `destroy`, one server per node,
// end of `Incoming` after `close`/`destroy`. Adapter internals (`node.rs`, `server.rs`); JS — server.js, index.js `destroy`.
use super::*;
use crate::server::PeeroxideServer;
use std::time::Duration;
use townjump_dht::{DhtServer, Incoming, ServerOpts};
use townjump_testkit::conformance::wait::within;
use townjump_testkit::key_pair;

#[allow(clippy::duplicate_mod)]
#[path = "support_tests.rs"]
mod support;

const WAIT: Duration = Duration::from_secs(30);

mod destroy {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn twice_is_harmless() {
        let node = support::isolated_node().await;
        assert!(node.destroy().await.is_ok());
        assert!(node.destroy().await.is_ok(), "repeated destroy");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn makes_connect_fail_with_node_destroyed() {
        let node = destroyed_node().await;
        let dial = node.connect([7; 32], ConnectOpts::default());
        let refused = within(WAIT, "connect", dial).await;
        assert!(
            matches!(refused, Err(DhtError::Destroyed)),
            "got {refused:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn makes_records_fail_with_node_destroyed() {
        let node = destroyed_node().await;
        let put = within(WAIT, "put", node.mutable_put(&key_pair(1), b"x", None)).await;
        assert!(matches!(put, Err(DhtError::Destroyed)), "put: {put:?}");
        let got = within(WAIT, "get", node.mutable_get([7; 32], None)).await;
        assert!(matches!(got, Err(DhtError::Destroyed)), "get: {got:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ends_the_incoming_of_a_server_of_the_node() {
        let node = support::isolated_node().await;
        let (_server, mut incoming) = server_of(&node);
        node.destroy().await.expect("destroy");
        assert_ended(&mut incoming).await;
    }
}

mod one_server_per_node {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn second_create_server_is_refused() {
        let node = support::isolated_node().await;
        let _first = server_of(&node);
        let second = node.create_server(ServerOpts::default());
        assert!(
            matches!(second, Err(DhtError::Backend(_))),
            "got {second:?}"
        );
    }
}

mod close {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ends_the_incoming_even_if_never_listened() {
        let node = support::isolated_node().await;
        let (server, mut incoming) = server_of(&node);
        within(WAIT, "close", server.close()).await.expect("close");
        assert_ended(&mut incoming).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn twice_is_harmless() {
        let node = support::isolated_node().await;
        let (server, _incoming) = server_of(&node);
        server.close().await.expect("first close");
        server.close().await.expect("second close");
    }
}

async fn destroyed_node() -> PeeroxideDht {
    let node = support::isolated_node().await;
    node.destroy().await.expect("destroy");
    node
}

fn server_of(node: &PeeroxideDht) -> (PeeroxideServer, Incoming) {
    node.create_server(ServerOpts::default())
        .expect("create_server")
}

async fn assert_ended(incoming: &mut Incoming) {
    let next = within(WAIT, "accept", incoming.accept()).await;
    assert!(next.is_none(), "the incoming queue must end");
}
