// Tests of `pipe_udp_framed_server`, part 2: closes, counters, log. Helpers are in udp_server_tests.rs.
use super::tests::{attach, first_datagram, service};
use crate::pipe_udp_framed_server;
use crate::support::*;
use std::sync::atomic::Ordering::SeqCst;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

mod closing {
    use super::*;

    #[tokio::test]
    async fn end_of_the_stream_closes_the_socket_and_the_server_returns() {
        let (service, port) = service().await;
        let watch = Watch::new(false);
        let (mut far, server) = attach(&watch, port);
        drive(server, async {
            let socket_addr = first_datagram(&mut far, &service).await;
            far.shutdown().await.unwrap();
            watch
                .ended
                .wait("pipe_udp_framed_server returned after the stream ended")
                .await;
            wait_until(
                "the port of the server socket is released after the stream end",
                || std::net::UdpSocket::bind(socket_addr).is_ok(),
            )
            .await;
        })
        .await;
    }

    #[tokio::test]
    async fn error_of_the_stream_ends_the_server() {
        // JS (testvectors/probes-js/pipe.cjs, `udpServer_afterStreamError`): stream `error` → close the socket → `remoteStream.end()`, datagrams
        // no longer flow. In Rust the socket belongs to the function, so "server returned" = socket closed; that is all visible from outside.
        let (_service, port) = service().await;
        let watch = Watch::new(false);
        let server = pipe_udp_framed_server(
            FailingStream::read_fails("boom"),
            ("127.0.0.1".to_owned(), port),
            watch.log.logger(),
            watch.stats.clone(),
        );
        drive(
            until_end(watch.ended.clone(), server),
            watch.ended.wait("server did not stop on a stream error"),
        )
        .await;
    }

    #[tokio::test]
    async fn datagrams_after_the_end_are_not_forwarded() {
        let (service, port) = service().await;
        let watch = Watch::new(false);
        let (mut far, server) = attach(&watch, port);
        drive(server, async {
            let socket_addr = first_datagram(&mut far, &service).await;
            far.shutdown().await.unwrap();
            watch.ended.wait("pipe_udp_framed_server returned").await;
            service.send_to(b"late", socket_addr).await.unwrap();
            let mut buf = [0u8; 16];
            let read = within("read of remote after the end", far.read(&mut buf)).await;
            assert_eq!(read.unwrap_or(0), 0, "no frames after the stream ended");
        })
        .await;
    }
}

mod stats {
    use super::*;

    #[tokio::test]
    async fn counters_are_not_touched_by_udp() {
        let (service, port) = service().await;
        let watch = Watch::new(false);
        let (mut far, server) = attach(&watch, port);
        drive(server, async {
            let to_server = first_datagram(&mut far, &service).await;
            service.send_to(b"back", to_server).await.unwrap();
            assert_eq!(read_frame(&mut far).await, b"back");
            assert_eq!(watch.loc_rem_cnt(), (0, 0), "stats are not used for UDP");
            assert_eq!(watch.stats.reject_cnt.load(SeqCst), 0);
        })
        .await;
    }
}

mod log {
    use super::*;

    #[tokio::test]
    async fn stream_chunk_is_logged_with_its_size_including_the_header() {
        // JS: `chunk.length` is the stream chunk size, i.e. a frame with header (4 + 5); testvectors/probes-js/pipe.cjs, `udpServer_normal`; pipe.md, section «Clarifications».
        let (service, port) = service().await;
        let watch = Watch::new(false);
        let (mut far, server) = attach(&watch, port);
        drive(server, async {
            far.write_all(&frame(b"hello")).await.unwrap();
            let _ = recv(&service).await;
            watch
                .log
                .wait_has("DEBUG", "Data from remote to local: 9 bytes")
                .await;
        })
        .await;
    }

    #[tokio::test]
    async fn service_datagram_is_logged_as_local_to_remote() {
        // JS: `msg.length` is only the datagram body, without the header: `12345` → 5 (testvectors/probes-js/pipe.cjs, `udpServer_normal`).
        let (service, port) = service().await;
        let watch = Watch::new(false);
        let (mut far, server) = attach(&watch, port);
        drive(server, async {
            let to_server = first_datagram(&mut far, &service).await;
            service.send_to(b"12345", to_server).await.unwrap();
            let _ = read_frame(&mut far).await;
            watch
                .log
                .wait_has("DEBUG", "Data from local to remote: 5 bytes")
                .await;
        })
        .await;
    }

    #[tokio::test]
    async fn failed_send_to_local_is_logged_and_the_next_frame_still_goes() {
        // A body over 65507 bytes does not leave as a datagram (JS: ERROR `Send error to local: …`).
        let (service, port) = service().await;
        let watch = Watch::new(false);
        let (mut far, server) = attach(&watch, port);
        drive(server, async {
            far.write_all(&frame(&vec![1u8; 70_000])).await.unwrap();
            far.write_all(&frame(b"ok")).await.unwrap();
            assert_eq!(recv(&service).await.0, b"ok");
            watch
                .log
                .wait_containing("ERROR", "Send error to local:")
                .await;
        })
        .await;
    }
}
