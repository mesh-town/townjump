// Tests of `Node::suspend/resume/adopt` without a network: a node without peers (bootstrap to a closed loopback port), UDX pairs on 127.0.0.1
// (pair_support_tests.rs). JS: `suspend()` tears down all tunnels (the remote peer sees an error), a sleeping node gives `SUSPENDED`.
use super::*;
use crate::bootstrap::BootstrapPlan;
use libudx::UdxRuntime;
use std::io::ErrorKind;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;
use townjump_testkit::conformance::wait::within;

// One helper file is included in three test files: a separate copy in each would be pointless.
#[allow(clippy::duplicate_mod)]
#[path = "pair_support_tests.rs"]
mod pair;

/// A port nobody listens on: bootstrap of a node without peers.
const CLOSED_BOOTSTRAP: &str = "127.0.0.1:1";
/// Node start, `suspend` and `resume` without peers end at once; the ceiling is only against hangs.
const STEP_WAIT: Duration = Duration::from_secs(10);
/// How long after `suspend` we wait for a waiting reader to wake up.
const WAKE_WAIT: Duration = Duration::from_secs(2);
const SUSPENDED_TEXT: &str = "SUSPENDED: Suspended";

mod adopt {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn on_a_live_node_gives_a_working_tunnel() {
        let node = isolated_node().await;
        let mut ends = adopted(&node).await;
        assert_eq!(ends.local.remote_public_key, pair::KEY_B);
        let (local, remote) = (&mut ends.local.stream, &mut ends.remote.stream);
        assert_eq!(pair::carry(local, remote, b"ping").await, b"ping");
        assert_eq!(pair::carry(remote, local, b"pong").await, b"pong");
        assert_eq!(node.tunnels.open(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn on_a_sleeping_node_is_the_suspended_error() {
        let node = isolated_node().await;
        step("suspend", node.suspend()).await.expect("suspend");
        let (a, _b, _rt) = pair::pair_within().await;
        let error = node.adopt(a).expect_err("sleeping node refuses adopt");
        assert_eq!(error.to_string(), SUSPENDED_TEXT);
        assert!(matches!(error, DhtError::Backend(_)), "got {error:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn on_a_sleeping_node_drops_the_connection_so_the_peer_gets_an_error() {
        let node = isolated_node().await;
        step("suspend", node.suspend()).await.expect("suspend");
        let (a, b, _rt) = pair::pair_within().await;
        assert!(node.adopt(a).is_err());
        let mut peer = crate::stream::wrap_connection(b);
        let read = pair::read_to_end_within(&mut peer.stream, "peer of the sleeping node").await;
        assert!(
            read.is_err(),
            "connection dropped: the remote peer expects an error, not EOF"
        );
        assert_eq!(
            node.tunnels.open(),
            0,
            "the rejected connection stayed in the registry"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn works_again_after_resume() {
        let node = isolated_node().await;
        step("suspend", node.suspend()).await.expect("suspend");
        step("resume", node.resume()).await.expect("resume");
        let mut ends = adopted(&node).await;
        let (local, remote) = (&mut ends.local.stream, &mut ends.remote.stream);
        assert_eq!(pair::carry(local, remote, b"again").await, b"again");
    }
}

mod suspend {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn breaks_a_registered_tunnel_for_the_peer() {
        let node = isolated_node().await;
        let mut ends = adopted(&node).await;
        step("suspend", node.suspend()).await.expect("suspend");
        let read =
            pair::read_to_end_within(&mut ends.remote.stream, "remote peer after suspend").await;
        assert!(
            read.is_err(),
            "suspend (DESTROY) must not look like a clean EOF"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn aborts_the_local_stream_with_connection_aborted() {
        let node = isolated_node().await;
        let mut ends = adopted(&node).await;
        step("suspend", node.suspend()).await.expect("suspend");
        let read = step("read", ends.local.stream.read(&mut [0u8; 8])).await;
        let write = step("write", ends.local.stream.write(b"late")).await;
        let kinds = (read.map_err(|e| e.kind()), write.map_err(|e| e.kind()));
        assert_eq!(
            kinds,
            (
                Err(ErrorKind::ConnectionAborted),
                Err(ErrorKind::ConnectionAborted)
            )
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn empties_the_tunnel_registry() {
        let node = isolated_node().await;
        let _first = adopted(&node).await;
        let _second = adopted(&node).await;
        assert_eq!(node.tunnels.open(), 2);
        step("suspend", node.suspend()).await.expect("suspend");
        assert_eq!(node.tunnels.open(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn wakes_a_reader_parked_in_a_task() {
        let node = isolated_node().await;
        let ends = adopted(&node).await;
        let (reader, _writer) = tokio::io::split(ends.local.stream);
        let (parked_tx, parked_rx) = oneshot::channel();
        let task = tokio::spawn(pair::read_until_parked(reader, parked_tx));
        step("reader parked in pending", parked_rx)
            .await
            .expect("reader parked");
        step("suspend", node.suspend()).await.expect("suspend");
        let read = within(WAKE_WAIT, "reader after suspend", task).await;
        let error = read.expect("reader task").expect_err("error");
        assert_eq!(error.kind(), ErrorKind::ConnectionAborted);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn puts_the_node_to_sleep_and_resume_wakes_it() {
        let node = isolated_node().await;
        assert!(!node.handle.is_suspended(), "new node is suspended");
        step("suspend", node.suspend()).await.expect("suspend");
        assert!(
            node.handle.is_suspended(),
            "node is not suspended after suspend"
        );
        step("resume", node.resume()).await.expect("resume");
        assert!(
            !node.handle.is_suspended(),
            "node is still suspended after resume"
        );
    }
}

mod repeated_calls {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn suspend_twice_is_ok() {
        let node = isolated_node().await;
        step("suspend", node.suspend())
            .await
            .expect("first suspend");
        step("suspend", node.suspend())
            .await
            .expect("second suspend");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn resume_twice_is_ok() {
        let node = isolated_node().await;
        step("suspend", node.suspend()).await.expect("suspend");
        step("resume", node.resume()).await.expect("first resume");
        step("resume", node.resume()).await.expect("second resume");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn resume_of_a_node_that_never_slept_is_ok() {
        let node = isolated_node().await;
        step("resume", node.resume()).await.expect("resume");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn suspend_resume_cycles_keep_working() {
        let node = isolated_node().await;
        for round in 1..=2 {
            step("suspend", node.suspend())
                .await
                .unwrap_or_else(|e| panic!("suspend {round}: {e}"));
            step("resume", node.resume())
                .await
                .unwrap_or_else(|e| panic!("resume {round}: {e}"));
        }
        let mut ends = adopted(&node).await;
        let (local, remote) = (&mut ends.local.stream, &mut ends.remote.stream);
        assert_eq!(pair::carry(local, remote, b"cycle").await, b"cycle");
    }
}

mod after_destroy {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn suspend_is_ok_like_in_js() {
        let node = isolated_node().await;
        step("destroy", node.destroy()).await.expect("destroy");
        step("suspend", node.suspend())
            .await
            .expect("suspend after destroy");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn resume_is_ok_like_in_js() {
        let node = isolated_node().await;
        step("destroy", node.destroy()).await.expect("destroy");
        step("resume", node.resume())
            .await
            .expect("resume after destroy");
    }
}

/// A tunnel accepted by the node (`local`) and the remote peer on the other side of the pair (`remote`); `_rt` keeps UDX alive.
struct Adopted {
    local: DhtConnection,
    remote: DhtConnection,
    _rt: [UdxRuntime; 2],
}

/// A new connection pair: side A goes to `node.adopt`, side B is just a stream.
async fn adopted(node: &Node) -> Adopted {
    let (a, b, rt) = pair::pair_within().await;
    Adopted {
        local: node.adopt(a).expect("adopt on a live node"),
        remote: crate::stream::wrap_connection(b),
        _rt: rt,
    }
}

/// A node without peers: bootstrap points at a closed loopback port, no network needed.
async fn isolated_node() -> Arc<Node> {
    let bootstrap = [CLOSED_BOOTSTRAP.to_string()];
    let plan = BootstrapPlan::parse(Some(&bootstrap)).expect("bootstrap of a node without peers");
    let started = step("start without peers", Node::start(plan.config(), None)).await;
    started.expect("Node::start: a node without peers needs no network")
}

/// A test step capped by `STEP_WAIT`: a hang is a labeled failure, not an endless wait.
async fn step<T>(what: &str, fut: impl std::future::Future<Output = T>) -> T {
    within(STEP_WAIT, what, fut).await
}
