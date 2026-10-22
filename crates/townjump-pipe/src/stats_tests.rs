// Tests of `stats`: the `PipeStats` counters and `PiperOpts` options (`on_destroy`, `logger`) in the work of `conn_piper`.
// Default `PipeStats` and `PiperOpts` are used, so what is checked is the join's behavior alone.
use super::*;
use crate::piper::conn_piper;
use crate::support::*;
use std::io;
use std::sync::atomic::Ordering::SeqCst;
use tokio::io::{AsyncWriteExt, DuplexStream, duplex};

mod counters {
    use super::*;

    #[tokio::test]
    async fn one_live_pipe_counts_one_local_and_one_remote_and_no_reject() {
        let watch = Watch::new(false);
        let (mut remote, mut local, piper) = duplex_piper(&watch);
        drive(piper, async {
            round_trip(&mut remote, &mut local).await;
            assert_eq!(watch.loc_rem_cnt(), (1, 1));
            assert_eq!(
                watch.stats.reject_cnt.load(SeqCst),
                0,
                "no rejections: Rust has no null destination"
            );
        })
        .await;
    }

    #[tokio::test]
    async fn closed_pipe_returns_counters_to_zero() {
        let watch = Watch::new(false);
        let (mut remote, mut local, piper) = duplex_piper(&watch);
        drive(piper, async {
            round_trip(&mut remote, &mut local).await;
            close_both_ends(&mut remote, &mut local).await;
            watch.ended.wait("conn_piper returned").await;
            assert_eq!(watch.loc_rem_cnt(), (0, 0));
            assert_eq!(watch.stats.reject_cnt.load(SeqCst), 0);
        })
        .await;
    }

    #[tokio::test]
    async fn pipe_destroyed_by_an_error_returns_counters_to_zero() {
        let watch = Watch::new(false);
        let piper = failing_piper(&watch, "left", "right");
        drive(piper, async {
            watch.ended.wait("conn_piper returned").await;
            assert_eq!(watch.loc_rem_cnt(), (0, 0));
        })
        .await;
    }

    #[tokio::test]
    async fn pipes_sharing_stats_add_up_and_leave_one_by_one() {
        let stats = Arc::new(PipeStats::default());
        let (first, second) = (
            Watch::sharing(stats.clone(), false),
            Watch::sharing(stats, false),
        );
        let (mut remote1, mut local1, piper1) = duplex_piper(&first);
        let (mut remote2, mut local2, piper2) = duplex_piper(&second);
        drive(
            async {
                tokio::join!(piper1, piper2);
            },
            async {
                round_trip(&mut remote1, &mut local1).await;
                round_trip(&mut remote2, &mut local2).await;
                assert_eq!(
                    first.loc_rem_cnt(),
                    (2, 2),
                    "two live joins on shared counters"
                );
                close_both_ends(&mut remote1, &mut local1).await;
                first.ended.wait("first join returned").await;
                assert_eq!(second.loc_rem_cnt(), (1, 1), "one of the two left");
                close_both_ends(&mut remote2, &mut local2).await;
                second.ended.wait("second join returned").await;
                assert_eq!(second.loc_rem_cnt(), (0, 0));
            },
        )
        .await;
    }
}

mod on_destroy_hook {
    use super::*;

    #[tokio::test]
    async fn clean_close_calls_it_once_without_an_error() {
        let watch = Watch::new(false);
        let (mut remote, mut local, piper) = duplex_piper(&watch);
        drive(piper, async {
            close_both_ends(&mut remote, &mut local).await;
            watch.ended.wait("conn_piper returned").await;
            assert_eq!(watch.destroyed.calls(), vec![None]);
        })
        .await;
    }

    #[tokio::test]
    async fn read_error_calls_it_once_with_that_error() {
        let watch = Watch::new(false);
        let (_remote, piper) = piper_with_local(&watch, FailingStream::read_fails("boom"));
        drive(piper, async {
            watch.ended.wait("conn_piper returned").await;
            let boom = Some((io::ErrorKind::ConnectionReset, "boom".to_owned()));
            assert_eq!(watch.destroyed.calls(), vec![boom]);
        })
        .await;
    }

    #[tokio::test]
    async fn write_error_calls_it_once_with_an_error() {
        let watch = Watch::new(false);
        let (mut remote, piper) = piper_with_local(&watch, FailingStream::write_fails("no write"));
        drive(piper, async {
            remote.write_all(b"data").await.unwrap();
            watch.ended.wait("conn_piper returned").await;
            let calls = watch.destroyed.calls();
            assert!(
                matches!(calls.as_slice(), [Some(_)]),
                "expected one call with an error: {calls:?}"
            );
        })
        .await;
    }

    #[tokio::test]
    async fn errors_on_both_sides_still_call_it_once() {
        let watch = Watch::new(false);
        let piper = failing_piper(&watch, "left", "right");
        drive(piper, async {
            watch.ended.wait("conn_piper returned").await;
            let calls = watch.destroyed.calls();
            assert_eq!(calls.len(), 1, "teardown once, calls: {calls:?}");
        })
        .await;
    }
}

mod default_opts {
    use super::*;

    #[tokio::test]
    async fn pipe_without_logger_and_hook_still_pumps_and_finishes() {
        let ended = Flag::default();
        let (mut remote, remote_near) = duplex(PIPE_BUF);
        let (mut local, local_near) = duplex(PIPE_BUF);
        let stats = Arc::new(PipeStats::default());
        let piper = until_end(
            ended.clone(),
            conn_piper(remote_near, local_near, PiperOpts::default(), stats),
        );
        drive(piper, async {
            round_trip(&mut remote, &mut local).await;
            close_both_ends(&mut remote, &mut local).await;
            ended.wait("conn_piper returned without options").await;
        })
        .await;
    }
}

/// A byte there and a byte back: after this the join is certainly running.
async fn round_trip(remote: &mut DuplexStream, local: &mut DuplexStream) {
    remote.write_all(b"x").await.unwrap();
    assert_eq!(read_n(local, 1).await, b"x");
    local.write_all(b"y").await.unwrap();
    assert_eq!(read_n(remote, 1).await, b"y");
}

async fn close_both_ends(remote: &mut DuplexStream, local: &mut DuplexStream) {
    remote.shutdown().await.unwrap();
    local.shutdown().await.unwrap();
}
