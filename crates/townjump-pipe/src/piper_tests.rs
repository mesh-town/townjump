// Tests of `conn_piper`: pumping, half-close, teardown on error, log (pipe.md, «TCP: connPiper»).
// `remote` is the JS `connection`, `local` is `loc`. Counters and `on_destroy` are in stats_tests.rs; `piper_with_*` are at the bottom.
use crate::support::*;
use tokio::io::{AsyncWriteExt, DuplexStream};

mod data_flow {
    use super::*;

    #[tokio::test]
    async fn one_megabyte_each_way_arrives_whole_and_in_order() {
        let watch = Watch::new(false);
        let (remote, local, piper) = duplex_piper(&watch);
        drive(piper, exchange_megabyte(remote, local)).await;
    }

    #[tokio::test]
    async fn bytes_written_at_remote_come_out_at_local() {
        let watch = Watch::new(false);
        let (mut remote, mut local, piper) = duplex_piper(&watch);
        drive(piper, async {
            remote.write_all(b"ping").await.unwrap();
            assert_eq!(read_n(&mut local, 4).await, b"ping");
        })
        .await;
    }

    #[tokio::test]
    async fn bytes_written_at_local_come_out_at_remote() {
        let watch = Watch::new(false);
        let (mut remote, mut local, piper) = duplex_piper(&watch);
        drive(piper, async {
            local.write_all(b"pong").await.unwrap();
            assert_eq!(read_n(&mut remote, 4).await, b"pong");
        })
        .await;
    }

    #[tokio::test]
    async fn five_hundred_small_writes_keep_their_order() {
        let watch = Watch::new(false);
        let (mut remote, mut local, piper) = duplex_piper(&watch);
        let expected = pattern(1500, 3);
        drive(piper, async {
            for part in expected.chunks(3) {
                remote.write_all(part).await.unwrap();
            }
            assert!(
                read_n(&mut local, 1500).await == expected,
                "order or content violated"
            );
        })
        .await;
    }
}

mod half_close {
    use super::*;

    mod local_ends {
        use super::*;

        #[tokio::test]
        async fn remote_gets_eof_and_can_still_answer() {
            let watch = Watch::new(false);
            let (mut remote, mut local, piper) = duplex_piper(&watch);
            drive(piper, async {
                local.shutdown().await.unwrap();
                expect_eof(&mut remote, "remote after the end of local").await;
                remote.write_all(b"late").await.unwrap();
                assert_eq!(read_n(&mut local, 4).await, b"late");
                assert!(
                    !watch.ended.is_raised(),
                    "half-close must not end the join at all"
                );
            })
            .await;
        }

        #[tokio::test]
        async fn data_written_before_the_end_is_delivered_before_eof() {
            let watch = Watch::new(false);
            let (mut remote, mut local, piper) = duplex_piper(&watch);
            drive(piper, async {
                local.write_all(b"tail").await.unwrap();
                local.shutdown().await.unwrap();
                assert_eq!(read_n(&mut remote, 4).await, b"tail");
                expect_eof(&mut remote, "remote after the tail").await;
            })
            .await;
        }
    }

    mod remote_ends {
        use super::*;

        #[tokio::test]
        async fn local_gets_eof_and_can_still_answer() {
            let watch = Watch::new(false);
            let (mut remote, mut local, piper) = duplex_piper(&watch);
            drive(piper, async {
                remote.shutdown().await.unwrap();
                expect_eof(&mut local, "local after the end of remote").await;
                local.write_all(b"late").await.unwrap();
                assert_eq!(read_n(&mut remote, 4).await, b"late");
                assert!(
                    !watch.ended.is_raised(),
                    "half-close must not end the join at all"
                );
            })
            .await;
        }

        #[tokio::test]
        async fn data_written_before_the_end_is_delivered_before_eof() {
            let watch = Watch::new(false);
            let (mut remote, mut local, piper) = duplex_piper(&watch);
            drive(piper, async {
                remote.write_all(b"tail").await.unwrap();
                remote.shutdown().await.unwrap();
                assert_eq!(read_n(&mut local, 4).await, b"tail");
                expect_eof(&mut local, "local after the tail").await;
            })
            .await;
        }
    }

    #[tokio::test]
    async fn both_ends_finished_the_piper_returns_and_both_sides_see_eof() {
        let watch = Watch::new(false);
        let (mut remote, mut local, piper) = duplex_piper(&watch);
        drive(piper, async {
            local.shutdown().await.unwrap();
            remote.shutdown().await.unwrap();
            watch
                .ended
                .wait("conn_piper returned after both sides ended")
                .await;
            expect_eof(&mut remote, "remote").await;
            expect_eof(&mut local, "local").await;
        })
        .await;
    }
}

mod destroy_on_error {
    use super::*;

    #[tokio::test]
    async fn read_error_at_local_closes_remote_and_the_piper_returns() {
        let watch = Watch::new(false);
        let (mut remote, piper) = piper_with_local(&watch, FailingStream::read_fails("boom"));
        drive(piper, async {
            expect_closed(&mut remote, "remote after local error").await;
            watch.ended.wait("conn_piper returned").await;
        })
        .await;
    }

    #[tokio::test]
    async fn read_error_at_remote_closes_local_and_the_piper_returns() {
        let watch = Watch::new(false);
        let (mut local, piper) = piper_with_remote(&watch, FailingStream::read_fails("boom"));
        drive(piper, async {
            expect_closed(&mut local, "local after remote error").await;
            watch.ended.wait("conn_piper returned").await;
        })
        .await;
    }

    #[tokio::test]
    async fn write_error_at_local_closes_remote_and_the_piper_returns() {
        let watch = Watch::new(false);
        let (mut remote, piper) = piper_with_local(&watch, FailingStream::write_fails("no write"));
        drive(piper, async {
            remote.write_all(b"data").await.unwrap();
            expect_closed(&mut remote, "remote after write error to local").await;
            watch.ended.wait("conn_piper returned").await;
        })
        .await;
    }

    #[tokio::test]
    async fn write_error_at_remote_closes_local_and_the_piper_returns() {
        let watch = Watch::new(false);
        let (mut local, piper) = piper_with_remote(&watch, FailingStream::write_fails("no write"));
        drive(piper, async {
            local.write_all(b"data").await.unwrap();
            expect_closed(&mut local, "local after write error to remote").await;
            watch.ended.wait("conn_piper returned").await;
        })
        .await;
    }

    #[tokio::test]
    async fn local_peer_gone_gives_remote_eof_and_the_next_remote_write_collapses_the_pipe() {
        let watch = Watch::new(false);
        let (mut remote, local, piper) = duplex_piper(&watch);
        drive(piper, async {
            drop(local);
            expect_eof(&mut remote, "remote after local left").await;
            let _ = remote.write_all(b"more").await;
            watch
                .ended
                .wait("a write to the departed local tears the join down")
                .await;
        })
        .await;
    }
}

mod log {
    use super::*;

    #[tokio::test]
    async fn starting_the_piper_is_logged_at_info() {
        let watch = Watch::new(true);
        let (_remote, _local, piper) = duplex_piper(&watch);
        drive(
            piper,
            watch.log.wait_has("INFO", "Starting TCP connection piper"),
        )
        .await;
    }

    #[tokio::test]
    async fn bytes_from_local_are_logged_as_local_to_remote() {
        // JS (testvectors/probes-js/pipe.cjs, `connPiper_directions`): Rust `local` = JS `loc`, data from `loc` is "local to remote"; same on the server.
        let watch = Watch::new(true);
        let (_remote, mut local, piper) = duplex_piper(&watch);
        drive(piper, async {
            local.write_all(b"12345").await.unwrap();
            watch
                .log
                .wait_has("DEBUG", "Data from local to remote: 5 bytes")
                .await;
        })
        .await;
    }

    #[tokio::test]
    async fn bytes_from_remote_are_logged_as_remote_to_local() {
        let watch = Watch::new(true);
        let (mut remote, _local, piper) = duplex_piper(&watch);
        drive(piper, async {
            remote.write_all(b"1234567").await.unwrap();
            watch
                .log
                .wait_has("DEBUG", "Data from remote to local: 7 bytes")
                .await;
        })
        .await;
    }

    #[tokio::test]
    async fn end_of_local_is_logged() {
        let watch = Watch::new(true);
        let (_remote, mut local, piper) = duplex_piper(&watch);
        drive(piper, async {
            local.shutdown().await.unwrap();
            watch
                .log
                .wait_has("DEBUG", "Local end, ending connection")
                .await;
        })
        .await;
    }

    #[tokio::test]
    async fn end_of_remote_is_logged() {
        let watch = Watch::new(true);
        let (mut remote, _local, piper) = duplex_piper(&watch);
        drive(piper, async {
            remote.shutdown().await.unwrap();
            watch
                .log
                .wait_has("DEBUG", "Connection end, ending local")
                .await;
        })
        .await;
    }

    #[tokio::test]
    async fn clean_destroy_is_logged_without_a_reason() {
        let watch = Watch::new(true);
        let (mut remote, mut local, piper) = duplex_piper(&watch);
        drive(piper, async {
            local.shutdown().await.unwrap();
            remote.shutdown().await.unwrap();
            watch
                .log
                .wait_has("INFO", "Destroying connection piper")
                .await;
        })
        .await;
    }

    #[tokio::test]
    async fn destroy_on_error_is_logged_with_the_reason() {
        let watch = Watch::new(true);
        let (_remote, piper) = piper_with_local(&watch, FailingStream::read_fails("boom"));
        let reason = "Destroying connection piper due to error: boom";
        drive(piper, watch.log.wait_containing("INFO", reason)).await;
    }
}

async fn exchange_megabyte(remote: DuplexStream, local: DuplexStream) {
    const MB: usize = 1 << 20;
    let (sent_by_remote, sent_by_local) = (pattern(MB, 1), pattern(MB, 2));
    let (mut remote_rd, mut remote_wr) = tokio::io::split(remote);
    let (mut local_rd, mut local_wr) = tokio::io::split(local);
    let sending = async {
        tokio::join!(
            remote_wr.write_all(&sent_by_remote),
            local_wr.write_all(&sent_by_local)
        )
    };
    let receiving = async { tokio::join!(read_n(&mut local_rd, MB), read_n(&mut remote_rd, MB)) };
    let (sent, (got_at_local, got_at_remote)) = tokio::join!(sending, receiving);
    sent.0.expect("write to remote");
    sent.1.expect("write to local");
    assert!(
        got_at_local == sent_by_remote,
        "local received something other than what was written to remote"
    );
    assert!(
        got_at_remote == sent_by_local,
        "remote received something other than what was written to local"
    );
}
