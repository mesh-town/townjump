// Tests of `pipe_tcp_server`: an incoming `remote` is joined with a new TCP to `local` (pipe.md, `pipeTcpServer`).
// The target service is a listener on 127.0.0.1:0; `attach` and helpers are at the bottom of the file.
use super::*;
use crate::support::*;
use std::io;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, duplex};
use tokio::net::{TcpListener, TcpStream};

mod connects_to_local {
    use super::*;

    #[tokio::test]
    async fn remote_bytes_reach_the_target() {
        let (target, port) = listener().await;
        let watch = Watch::new(false);
        let (mut far, piper) = attach(&watch, "127.0.0.1", port);
        drive(piper, async {
            far.write_all(b"hello").await.unwrap();
            let mut accepted = accept(&target).await;
            assert_eq!(read_n(&mut accepted, 5).await, b"hello");
        })
        .await;
    }

    #[tokio::test]
    async fn target_bytes_reach_the_remote() {
        let (target, port) = listener().await;
        let watch = Watch::new(false);
        let (mut far, piper) = attach(&watch, "127.0.0.1", port);
        drive(piper, async {
            let mut accepted = accept(&target).await;
            accepted.write_all(b"world").await.unwrap();
            assert_eq!(read_n(&mut far, 5).await, b"world");
        })
        .await;
    }

    #[tokio::test]
    async fn a_host_name_is_resolved_as_is() {
        let (target, port) = listener().await;
        let watch = Watch::new(false);
        let (mut far, piper) = attach(&watch, "localhost", port);
        drive(piper, async {
            far.write_all(b"by-name").await.unwrap();
            let mut accepted = accept(&target).await;
            assert_eq!(read_n(&mut accepted, 7).await, b"by-name");
        })
        .await;
    }

    #[tokio::test]
    async fn each_remote_gets_its_own_tcp_connection() {
        let (target, port) = listener().await;
        let (first, second) = (Watch::new(false), Watch::new(false));
        let (mut far1, piper1) = attach(&first, "127.0.0.1", port);
        let (mut far2, piper2) = attach(&second, "127.0.0.1", port);
        drive(
            async {
                tokio::join!(piper1, piper2);
            },
            async {
                far1.write_all(b"one").await.unwrap();
                far2.write_all(b"two").await.unwrap();
                let (mut a, mut b) = (accept(&target).await, accept(&target).await);
                let (got_a, got_b) = (read_n(&mut a, 3).await, read_n(&mut b, 3).await);
                let mut got = [got_a.clone(), got_b];
                got.sort();
                assert_eq!(
                    got,
                    [b"one".to_vec(), b"two".to_vec()],
                    "one message per connection"
                );
                let mut for_one = if got_a == b"one" { a } else { b };
                for_one.write_all(b"R1").await.unwrap();
                assert_eq!(read_n(&mut far1, 2).await, b"R1");
                let mut byte = [0u8; 1];
                stays_pending("1st reply reached the 2nd", far2.read(&mut byte)).await;
            },
        )
        .await;
    }
}

mod local_unreachable {
    use super::*;

    #[tokio::test]
    async fn remote_is_closed_and_the_server_returns() {
        let watch = Watch::new(false);
        let (mut far, piper) = attach(&watch, "127.0.0.1", free_tcp_port());
        drive(piper, async {
            expect_closed(&mut far, "remote with unreachable local").await;
            watch.ended.wait("pipe_tcp_server returned").await;
        })
        .await;
    }

    #[tokio::test]
    async fn counters_are_zero_after_the_refusal() {
        let watch = Watch::new(false);
        let (_far, piper) = attach(&watch, "127.0.0.1", free_tcp_port());
        drive(piper, async {
            watch.ended.wait("pipe_tcp_server returned").await;
            assert_eq!(watch.loc_rem_cnt(), (0, 0));
        })
        .await;
    }

    #[tokio::test]
    async fn on_destroy_gets_connection_refused_once() {
        let watch = Watch::new(false);
        let (_far, piper) = attach(&watch, "127.0.0.1", free_tcp_port());
        drive(piper, async {
            watch.ended.wait("pipe_tcp_server returned").await;
            let calls = watch.destroyed.calls();
            let kind = calls
                .iter()
                .map(|c| c.as_ref().map(|(kind, _)| *kind))
                .collect::<Vec<_>>();
            assert_eq!(
                kind,
                vec![Some(io::ErrorKind::ConnectionRefused)],
                "calls: {calls:?}"
            );
        })
        .await;
    }

    #[tokio::test]
    async fn destroy_is_logged_with_a_reason() {
        let watch = Watch::new(true);
        let (_far, piper) = attach(&watch, "127.0.0.1", free_tcp_port());
        let reason = "Destroying connection piper due to error: ";
        drive(piper, watch.log.wait_containing("INFO", reason)).await;
    }
}

mod closing {
    use super::*;

    #[tokio::test]
    async fn end_of_the_target_reaches_remote_and_the_remote_can_still_answer() {
        let (target, port) = listener().await;
        let watch = Watch::new(false);
        let (mut far, piper) = attach(&watch, "127.0.0.1", port);
        drive(piper, async {
            let mut accepted = accept(&target).await;
            accepted.shutdown().await.unwrap();
            expect_eof(&mut far, "remote after the target service's end").await;
            far.write_all(b"late").await.unwrap();
            assert_eq!(read_n(&mut accepted, 4).await, b"late");
        })
        .await;
    }

    #[tokio::test]
    async fn end_of_the_remote_reaches_the_target_and_the_target_can_still_answer() {
        let (target, port) = listener().await;
        let watch = Watch::new(false);
        let (mut far, piper) = attach(&watch, "127.0.0.1", port);
        drive(piper, async {
            let mut accepted = accept(&target).await;
            far.shutdown().await.unwrap();
            expect_eof(&mut accepted, "target service after the end of remote").await;
            accepted.write_all(b"late").await.unwrap();
            assert_eq!(read_n(&mut far, 4).await, b"late");
        })
        .await;
    }

    #[tokio::test]
    async fn both_ends_finished_the_server_returns_and_counters_go_to_zero() {
        let (target, port) = listener().await;
        let watch = Watch::new(false);
        let (mut far, piper) = attach(&watch, "127.0.0.1", port);
        drive(piper, async {
            let mut accepted = accept(&target).await;
            far.shutdown().await.unwrap();
            accepted.shutdown().await.unwrap();
            watch.ended.wait("pipe_tcp_server returned").await;
            assert_eq!(watch.loc_rem_cnt(), (0, 0));
            assert_eq!(watch.destroyed.calls(), vec![None]);
        })
        .await;
    }
}

mod log {
    use super::*;

    #[tokio::test]
    async fn connected_is_logged_after_starting_the_piper() {
        let (target, port) = listener().await;
        let watch = Watch::new(true);
        let (_far, piper) = attach(&watch, "127.0.0.1", port);
        drive(piper, async {
            let _accepted = accept(&target).await;
            watch.log.wait_has("INFO", "Connected").await;
            let start = watch.log.positions("INFO", "Starting TCP connection piper");
            let connected = watch.log.positions("INFO", "Connected");
            assert!(
                start[0] < connected[0],
                "Starting before Connected: {:?}",
                watch.log.lines()
            );
        })
        .await;
    }

    #[tokio::test]
    async fn target_bytes_are_logged_as_local_to_remote() {
        let (target, port) = listener().await;
        let watch = Watch::new(true);
        let (_far, piper) = attach(&watch, "127.0.0.1", port);
        drive(piper, async {
            let mut accepted = accept(&target).await;
            accepted.write_all(b"12345").await.unwrap();
            watch
                .log
                .wait_has("DEBUG", "Data from local to remote: 5 bytes")
                .await;
        })
        .await;
    }

    #[tokio::test]
    async fn remote_bytes_are_logged_as_remote_to_local() {
        let (target, port) = listener().await;
        let watch = Watch::new(true);
        let (mut far, piper) = attach(&watch, "127.0.0.1", port);
        drive(piper, async {
            let _accepted = accept(&target).await;
            far.write_all(b"123").await.unwrap();
            watch
                .log
                .wait_has("DEBUG", "Data from remote to local: 3 bytes")
                .await;
        })
        .await;
    }
}

async fn listener() -> (TcpListener, u16) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind of the target service");
    let port = listener.local_addr().expect("local_addr").port();
    (listener, port)
}

async fn accept(target: &TcpListener) -> TcpStream {
    let accepted = within("server did not connect to local", target.accept()).await;
    accepted.expect("accept").0
}

/// `pipe_tcp_server` for a new incoming stream; returns its far end and the join future.
fn attach(
    watch: &Watch,
    host: &str,
    port: u16,
) -> (DuplexStream, impl Future<Output = ()> + use<>) {
    let (far, near) = duplex(PIPE_BUF);
    let piper = pipe_tcp_server(
        near,
        (host.to_owned(), port),
        watch.opts(),
        watch.stats.clone(),
    );
    (far, until_end(watch.ended.clone(), piper))
}
