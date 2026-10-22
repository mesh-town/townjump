// Tests of `Tunnels`: the registry of issued tunnels — `close_all` tears them all down (like JS `rawStreams.clear()`), dropping a stream
// frees its entry, after an abort a new one opens. UDX pairs on 127.0.0.1 (pair_support_tests.rs), no DHT or JS.
use super::*;
use libudx::UdxRuntime;
use std::io::ErrorKind;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;
use townjump_dht::DhtConnection;
use townjump_testkit::conformance::wait::{STREAM_WAIT, within};

// One helper file is included in three test files: a separate copy in each would be pointless.
#[allow(clippy::duplicate_mod)]
#[path = "pair_support_tests.rs"]
mod pair;

/// How long after `close_all` we wait for a waiter to wake: it must wake by itself, without outside polling.
const WAKE_WAIT: Duration = Duration::from_secs(2);

mod register {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn keeps_the_public_key_of_the_peer() {
        let tunnels = Tunnels::default();
        let ends = tunnel(&tunnels).await;
        assert_eq!(ends.local.remote_public_key, pair::KEY_B);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn counts_every_tunnel_as_open() {
        let tunnels = Tunnels::default();
        assert_eq!(tunnels.open(), 0, "empty registry");
        let _first = tunnel(&tunnels).await;
        assert_eq!(tunnels.open(), 1);
        let _second = tunnel(&tunnels).await;
        assert_eq!(tunnels.open(), 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn gives_a_working_tunnel_both_ways() {
        let tunnels = Tunnels::default();
        let mut ends = tunnel(&tunnels).await;
        let (local, remote) = (&mut ends.local.stream, &mut ends.remote.stream);
        assert_eq!(pair::carry(local, remote, b"ping").await, b"ping");
        assert_eq!(pair::carry(remote, local, b"pong").await, b"pong");
    }
}

mod close_all {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn empties_the_registry() {
        let tunnels = Tunnels::default();
        let _first = tunnel(&tunnels).await;
        let _second = tunnel(&tunnels).await;
        assert_eq!(tunnels.open(), 2);
        tunnels.close_all();
        assert_eq!(tunnels.open(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn gives_every_peer_an_error_not_a_clean_eof() {
        let tunnels = Tunnels::default();
        let mut first = tunnel(&tunnels).await;
        let mut second = tunnel(&tunnels).await;
        tunnels.close_all();
        for (name, ends) in [("first", &mut first), ("second", &mut second)] {
            let what = format!("remote peer: {name} tunnel after close_all");
            let read = pair::read_to_end_within(&mut ends.remote.stream, &what).await;
            assert!(read.is_err(), "{what}: an abort must not look like EOF");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn aborts_the_local_streams_with_connection_aborted() {
        let tunnels = Tunnels::default();
        let mut first = tunnel(&tunnels).await;
        let mut second = tunnel(&tunnels).await;
        tunnels.close_all();
        for ends in [&mut first, &mut second] {
            let read = within(STREAM_WAIT, "read", ends.local.stream.read(&mut [0u8; 8])).await;
            let write = within(STREAM_WAIT, "write", ends.local.stream.write(b"late")).await;
            let kinds = (read.map_err(|e| e.kind()), write.map_err(|e| e.kind()));
            assert_eq!(
                kinds,
                (
                    Err(ErrorKind::ConnectionAborted),
                    Err(ErrorKind::ConnectionAborted)
                )
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn wakes_a_reader_of_a_split_stream_parked_in_a_task() {
        let tunnels = Tunnels::default();
        let ends = tunnel(&tunnels).await;
        let (reader, _writer) = tokio::io::split(ends.local.stream);
        let (parked_tx, parked_rx) = oneshot::channel();
        let task = tokio::spawn(pair::read_until_parked(reader, parked_tx));
        within(STREAM_WAIT, "reader parked in pending", parked_rx)
            .await
            .expect("reader finished without parking in pending");
        tunnels.close_all();
        let read = within(WAKE_WAIT, "reader after close_all", task).await;
        let error = read.expect("reader task").expect_err("error");
        assert_eq!(error.kind(), ErrorKind::ConnectionAborted);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn on_an_empty_registry_is_harmless() {
        let tunnels = Tunnels::default();
        tunnels.close_all();
        tunnels.close_all();
        assert_eq!(tunnels.open(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn twice_in_a_row_is_harmless() {
        let tunnels = Tunnels::default();
        let _ends = tunnel(&tunnels).await;
        tunnels.close_all();
        tunnels.close_all();
        assert_eq!(tunnels.open(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn skips_the_tunnels_already_dropped() {
        let tunnels = Tunnels::default();
        drop(tunnel(&tunnels).await);
        let mut alive = tunnel(&tunnels).await;
        tunnels.close_all();
        let read = within(STREAM_WAIT, "read", alive.local.stream.read(&mut [0u8; 8])).await;
        assert_eq!(
            read.map_err(|e| e.kind()),
            Err(ErrorKind::ConnectionAborted)
        );
    }
}

mod a_dropped_stream {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn is_no_longer_open() {
        let tunnels = Tunnels::default();
        let ends = tunnel(&tunnels).await;
        assert_eq!(tunnels.open(), 1);
        drop(ends.local);
        assert_eq!(tunnels.open(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn is_forgotten_by_the_next_register() {
        let tunnels = Tunnels::default();
        drop(tunnel(&tunnels).await);
        assert_eq!(tunnels.open(), 0);
        let _next = tunnel(&tunnels).await;
        assert_eq!(tunnels.open(), 1);
        assert_eq!(
            registry_len(&tunnels),
            1,
            "dead entry stayed in the registry"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn gives_the_peer_an_error_not_a_clean_eof() {
        let tunnels = Tunnels::default();
        let mut ends = tunnel(&tunnels).await;
        drop(ends.local);
        let read = pair::read_to_end_within(&mut ends.remote.stream, "peer after drop").await;
        assert!(
            read.is_err(),
            "drop before shutdown (DESTROY) must not look like EOF"
        );
    }
}

mod after_close_all {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_new_tunnel_carries_bytes_both_ways() {
        let tunnels = Tunnels::default();
        let _old = tunnel(&tunnels).await;
        tunnels.close_all();
        let mut fresh = tunnel(&tunnels).await;
        assert_eq!(tunnels.open(), 1, "the new tunnel is not in the registry");
        let (local, remote) = (&mut fresh.local.stream, &mut fresh.remote.stream);
        assert_eq!(pair::carry(local, remote, b"again").await, b"again");
        assert_eq!(pair::carry(remote, local, b"back").await, b"back");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_next_close_all_breaks_only_the_new_tunnel_still_open() {
        let tunnels = Tunnels::default();
        let _old = tunnel(&tunnels).await;
        tunnels.close_all();
        let _fresh = tunnel(&tunnels).await;
        assert_eq!(tunnels.open(), 1);
        tunnels.close_all();
        assert_eq!(tunnels.open(), 0);
    }
}

/// A tunnel from the registry (`local`) and the remote peer on the other side of the pair (`remote`, no registry); `_rt` keeps UDX alive.
struct Tunnel {
    local: DhtConnection,
    remote: DhtConnection,
    _rt: [UdxRuntime; 2],
}

/// A new connection pair: side A is recorded in the registry, side B is just a stream.
async fn tunnel(tunnels: &Tunnels) -> Tunnel {
    let (a, b, rt) = pair::pair_within().await;
    Tunnel {
        local: tunnels.register(a),
        remote: crate::stream::wrap_connection(b),
        _rt: rt,
    }
}

/// How many entries are in the registry, live and dead.
fn registry_len(tunnels: &Tunnels) -> usize {
    tunnels
        .slots
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .len()
}
