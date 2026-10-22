// Tests of `Slot`: `kill` wakes the waiting reader and writer, afterwards read, write and flush give `ConnectionAborted`,
// shutdown gives `Ok`, the remote peer gets an error, not EOF. A UDX pair on 127.0.0.1 (pair_support_tests.rs), no DHT or JS.
use super::*;
use crate::stream::{wrap_connection, wrap_slot};
use libudx::UdxRuntime;
use std::io::ErrorKind;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;
use townjump_dht::DhtConnection;
use townjump_testkit::conformance::wait::{STREAM_WAIT, within};

// One helper file is included in three test files: a separate copy in each would be pointless.
#[allow(clippy::duplicate_mod)]
#[path = "pair_support_tests.rs"]
mod pair;

/// How long after `kill` we wait for a waiter to wake: it must wake by itself, without outside polling.
const WAKE_WAIT: Duration = Duration::from_secs(2);

mod kill_wakes_the_waiting {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_reader_parked_in_pending_wakes_with_connection_aborted() {
        let ends = slot_pair().await;
        let (reader, _writer) = tokio::io::split(ends.conn.stream);
        let (parked_tx, parked_rx) = oneshot::channel();
        let task = tokio::spawn(pair::read_until_parked(reader, parked_tx));
        within(STREAM_WAIT, "reader parked in pending", parked_rx)
            .await
            .expect("reader finished without parking in pending");
        ends.slot.kill();
        let read = within(WAKE_WAIT, "reader after kill", task).await;
        let error = read
            .expect("reader task")
            .expect_err("kill must produce an error");
        assert_eq!(error.kind(), ErrorKind::ConnectionAborted);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_writer_stuck_on_a_full_window_wakes_with_connection_aborted() {
        let ends = slot_pair().await;
        let (_reader, writer) = tokio::io::split(ends.conn.stream);
        let (parked_tx, parked_rx) = oneshot::channel();
        let task = tokio::spawn(pair::write_until_parked(writer, parked_tx));
        let sent = within(STREAM_WAIT, "write parked on a full window", parked_rx).await;
        sent.expect("write did not park within the cap: the remote peer does not read, the window must fill up");
        ends.slot.kill();
        let write = within(WAKE_WAIT, "writer after kill", task).await;
        let error = write
            .expect("writer task")
            .expect_err("kill must produce an error");
        assert_eq!(error.kind(), ErrorKind::ConnectionAborted);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn both_the_reader_and_the_writer_of_one_split_stream_wake() {
        let ends = slot_pair().await;
        let (reader, writer) = tokio::io::split(ends.conn.stream);
        let (read_tx, read_rx) = oneshot::channel();
        let (write_tx, write_rx) = oneshot::channel();
        let reading = tokio::spawn(pair::read_until_parked(reader, read_tx));
        let writing = tokio::spawn(pair::write_until_parked(writer, write_tx));
        within(STREAM_WAIT, "reader parked", read_rx)
            .await
            .expect("reader parked");
        within(STREAM_WAIT, "writer parked", write_rx)
            .await
            .expect("writer parked");
        ends.slot.kill();
        let read = within(WAKE_WAIT, "reader after kill", reading).await;
        let write = within(WAKE_WAIT, "writer after kill", writing).await;
        let read_kind = read.expect("reader task").expect_err("reader").kind();
        let write_kind = write
            .expect("writer task")
            .expect_err("writer must fail")
            .kind();
        assert_eq!(
            (read_kind, write_kind),
            (ErrorKind::ConnectionAborted, ErrorKind::ConnectionAborted)
        );
    }
}

mod after_kill {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn read_fails_with_connection_aborted() {
        let mut ends = slot_pair().await;
        ends.slot.kill();
        let mut buf = [0u8; 8];
        let read = ends.conn.stream.read(&mut buf);
        let error = within(STREAM_WAIT, "read", read).await.expect_err("read");
        assert_eq!(error.kind(), ErrorKind::ConnectionAborted);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn write_fails_with_connection_aborted() {
        let mut ends = slot_pair().await;
        ends.slot.kill();
        let error = within(STREAM_WAIT, "write", ends.conn.stream.write(b"late"))
            .await
            .expect_err("write");
        assert_eq!(error.kind(), ErrorKind::ConnectionAborted);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn flush_fails_with_connection_aborted() {
        let mut ends = slot_pair().await;
        ends.slot.kill();
        let error = ends.conn.stream.flush().await.expect_err("flush");
        assert_eq!(error.kind(), ErrorKind::ConnectionAborted);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_is_ok_like_end_on_a_destroyed_js_stream() {
        let mut ends = slot_pair().await;
        ends.slot.kill();
        let done = within(
            WAKE_WAIT,
            "shutdown after kill",
            ends.conn.stream.shutdown(),
        )
        .await;
        assert!(done.is_ok(), "shutdown after kill returned {done:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_slot_reports_killed_and_keeps_the_peer_key() {
        let ends = slot_pair().await;
        assert!(!ends.slot.is_killed(), "new slot is already killed");
        ends.slot.kill();
        assert!(ends.slot.is_killed(), "slot is not killed after kill");
        assert_eq!(ends.slot.remote_public_key, pair::KEY_B);
    }
}

mod the_peer {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn gets_an_error_after_kill_not_a_clean_eof() {
        let mut ends = slot_pair().await;
        ends.slot.kill();
        let read = pair::read_to_end_within(&mut ends.peer.stream, "remote peer after kill").await;
        assert!(
            read.is_err(),
            "kill (DESTROY) must not look like a clean EOF"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn receives_the_bytes_sent_before_kill() {
        let mut ends = slot_pair().await;
        let got = pair::carry(&mut ends.conn.stream, &mut ends.peer.stream, b"before").await;
        assert_eq!(got, b"before");
        ends.slot.kill();
        let read = pair::read_to_end_within(&mut ends.peer.stream, "remote peer after kill").await;
        assert!(
            read.is_err(),
            "after kill the remote peer must get an error"
        );
    }
}

mod repeated_kill {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn is_harmless() {
        let mut ends = slot_pair().await;
        ends.slot.kill();
        ends.slot.kill();
        ends.slot.kill();
        assert!(ends.slot.is_killed());
        let error = within(STREAM_WAIT, "write", ends.conn.stream.write(b"x"))
            .await
            .expect_err("write");
        assert_eq!(error.kind(), ErrorKind::ConnectionAborted);
    }
}

mod before_kill {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn bytes_go_both_ways_through_the_slot() {
        let mut ends = slot_pair().await;
        let ping = pair::carry(&mut ends.conn.stream, &mut ends.peer.stream, b"ping").await;
        let pong = pair::carry(&mut ends.peer.stream, &mut ends.conn.stream, b"pong").await;
        assert_eq!(
            (ping.as_slice(), pong.as_slice()),
            (&b"ping"[..], &b"pong"[..])
        );
    }
}

/// Slot A, the stream over it (`conn`), remote peer B without a slot (`peer`); `_rt` keeps the UDX runtimes alive.
struct Ends {
    slot: Arc<Slot>,
    conn: DhtConnection,
    peer: DhtConnection,
    _rt: [UdxRuntime; 2],
}

async fn slot_pair() -> Ends {
    let (a, b, rt) = pair::pair_within().await;
    let slot = Arc::new(Slot::new(a));
    Ends {
        conn: wrap_slot(Arc::clone(&slot)),
        peer: wrap_connection(b),
        slot,
        _rt: rt,
    }
}
