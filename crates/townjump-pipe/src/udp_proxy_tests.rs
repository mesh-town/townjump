// Tests of `create_udp_framed_proxy`: application datagrams ↔ frames in DHT streams, one stream per sender (pipe.md).
// The application is plain UDP sockets on 127.0.0.1; the remote side is a `duplex` from `remotes()`; `setup` is at the bottom of the file.
use super::*;
use crate::support::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::UdpSocket;
use tokio::time::sleep;

mod binding {
    use super::*;

    #[tokio::test]
    async fn port_is_chosen_by_the_os_and_reported() {
        let p = setup().await;
        assert_ne!(p.proxy.local_addr.port(), 0);
        assert!(p.proxy.local_addr.ip().is_loopback());
    }

    #[tokio::test]
    async fn a_datagram_right_after_return_reaches_connect() {
        let mut p = setup().await;
        let app = udp_socket().await;
        app.send_to(b"hi", p.proxy.local_addr).await.unwrap();
        let _far = p.remotes.next().await;
    }

    #[tokio::test]
    async fn busy_port_gives_an_addr_in_use_error() {
        // JS (testvectors/probes-js/pipe.cjs, `udpProxy_busyPort`): `bind` on a busy port throws synchronously `Error: address already in use`
        // (EADDRINUSE, bare-dgram over udx-native), no log and no `onBind`; the ERROR `Proxy socket error` is only for an asynchronous error.
        let holder = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = holder.local_addr().unwrap().port();
        let (_remotes, connect) = remotes(0);
        let log = LogCapture::default();
        let result =
            create_udp_framed_proxy(("127.0.0.1".to_owned(), port), connect, log.logger()).await;
        assert_eq!(
            result.expect_err("port busy").kind(),
            io::ErrorKind::AddrInUse
        );
    }

    #[tokio::test]
    async fn abort_closes_the_socket_and_the_sender_streams() {
        let mut p = setup().await;
        let app = udp_socket().await;
        app.send_to(b"hi", p.proxy.local_addr).await.unwrap();
        let mut far = p.remotes.next().await;
        p.proxy.task.abort();
        let _ = within("proxy task stopped", &mut p.proxy.task).await;
        expect_closed(&mut far, "sender stream after abort").await;
        let _ = app.send_to(b"late", p.proxy.local_addr).await;
        p.remotes.none_new().await;
    }
}

mod datagram_to_remote {
    use super::*;

    #[tokio::test]
    async fn becomes_a_frame_with_a_length_prefix() {
        let (p, app, mut far) = sender_with_stream().await;
        app.send_to(b"hello", p.proxy.local_addr).await.unwrap();
        assert_eq!(
            read_n(&mut far, 9).await,
            [0, 0, 0, 5, b'h', b'e', b'l', b'l', b'o']
        );
    }

    #[tokio::test]
    async fn empty_datagram_becomes_a_zero_length_frame() {
        let (p, app, mut far) = sender_with_stream().await;
        app.send_to(b"", p.proxy.local_addr).await.unwrap();
        assert_eq!(read_n(&mut far, 4).await, [0, 0, 0, 0]);
    }

    #[tokio::test]
    async fn datagram_of_2048_bytes_passes_whole() {
        let (p, app, mut far) = sender_with_stream().await;
        let data = pattern(2048, 4);
        app.send_to(&data, p.proxy.local_addr).await.unwrap();
        assert!(
            read_n(&mut far, 4 + 2048).await == frame(&data),
            "2048-byte frame: [0,0,8,0] and the whole body"
        );
    }

    #[tokio::test]
    async fn datagram_of_2049_bytes_is_cut_to_2048() {
        // Reference quirk (pipe.md): bare-dgram accepts up to 2048 bytes, the rest is lost.
        assert_cut_to_2048(2049).await;
    }

    #[tokio::test]
    async fn datagram_of_4000_bytes_is_cut_to_2048() {
        assert_cut_to_2048(4000).await;
    }

    #[tokio::test]
    async fn datagrams_keep_their_order() {
        let (p, app, mut far) = sender_with_stream().await;
        let sent: Vec<Vec<u8>> = (0..30).map(|i| pattern(10 + i, i as u32 + 1)).collect();
        for datagram in &sent {
            app.send_to(datagram, p.proxy.local_addr).await.unwrap();
        }
        for datagram in &sent {
            assert!(
                read_frame(&mut far).await == *datagram,
                "frames arrived out of send order"
            );
        }
    }
}

mod frame_to_datagram {
    use super::*;

    #[tokio::test]
    async fn frame_becomes_a_datagram_from_the_proxy_address() {
        let (p, app, mut far) = sender_with_stream().await;
        far.write_all(&[0, 0, 0, 3, b'a', b'b', b'c'])
            .await
            .unwrap();
        assert_eq!(recv(&app).await, (b"abc".to_vec(), p.proxy.local_addr));
    }

    #[tokio::test]
    async fn zero_length_frame_becomes_an_empty_datagram() {
        let (p, app, mut far) = sender_with_stream().await;
        far.write_all(&[0, 0, 0, 0]).await.unwrap();
        assert_eq!(recv(&app).await, (Vec::new(), p.proxy.local_addr));
    }

    #[tokio::test]
    async fn two_frames_in_one_write_become_two_datagrams_in_order() {
        let (_p, app, mut far) = sender_with_stream().await;
        far.write_all(&[0, 0, 0, 1, b'1', 0, 0, 0, 2, b'2', b'2'])
            .await
            .unwrap();
        assert_eq!(recv(&app).await.0, b"1");
        assert_eq!(recv(&app).await.0, b"22");
    }

    #[tokio::test]
    async fn frame_written_in_pieces_becomes_one_datagram() {
        let (_p, app, mut far) = sender_with_stream().await;
        for piece in [&[0u8, 0][..], &[0, 3, b'a'], b"bc"] {
            far.write_all(piece).await.unwrap();
            far.flush().await.unwrap();
            sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(recv(&app).await.0, b"abc");
        stays_pending("second datagram from one frame", recv(&app)).await;
    }

    #[tokio::test]
    async fn failed_send_is_logged_and_the_next_frame_is_still_delivered() {
        // A frame does not limit the length: a body over 65507 bytes does not leave as a datagram (JS: `Send error to <id>`).
        let (p, app, mut far) = sender_with_stream().await;
        far.write_all(&frame(&vec![1u8; 70_000])).await.unwrap();
        far.write_all(&frame(b"ok")).await.unwrap();
        assert_eq!(recv(&app).await.0, b"ok");
        p.log
            .wait_containing("ERROR", &format!("Send error to {}:", id_of(&app)))
            .await;
    }
}

mod senders {
    use super::*;

    #[tokio::test]
    async fn connect_is_not_called_before_the_first_datagram() {
        let mut p = setup().await;
        p.remotes.none_new().await;
        assert_eq!(p.remotes.calls(), 0);
    }

    #[tokio::test]
    async fn one_sender_keeps_one_stream() {
        let (p, app, mut far) = sender_with_stream().await;
        for text in [b"a", b"b", b"c"] {
            app.send_to(text, p.proxy.local_addr).await.unwrap();
        }
        for text in [b"a", b"b", b"c"] {
            assert_eq!(read_frame(&mut far).await, text);
        }
        assert_eq!(
            p.remotes.calls(),
            1,
            "one connect per sender, not per datagram"
        );
    }

    #[tokio::test]
    async fn each_sender_gets_its_own_stream_and_its_own_replies() {
        let mut p = setup().await;
        let (app_a, app_b) = (udp_socket().await, udp_socket().await);
        app_a.send_to(b"from-a", p.proxy.local_addr).await.unwrap();
        let mut far_a = p.remotes.next().await;
        app_b.send_to(b"from-b", p.proxy.local_addr).await.unwrap();
        let mut far_b = p.remotes.next().await;
        assert_eq!(read_frame(&mut far_a).await, b"from-a");
        assert_eq!(read_frame(&mut far_b).await, b"from-b");
        far_b.write_all(&frame(b"to-b")).await.unwrap();
        assert_eq!(recv(&app_b).await.0, b"to-b");
        stays_pending("reply for B reached A", recv(&app_a)).await;
    }
}

mod remote_stream_ends {
    use super::*;

    #[tokio::test]
    async fn error_is_logged_and_the_sender_gets_a_new_stream() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let connect = move || {
            let stream = match counter.fetch_add(1, SeqCst) {
                0 => FailingStream::read_fails("boom"),
                _ => FailingStream::silent(),
            };
            std::future::ready(Ok::<_, io::Error>(stream))
        };
        let log = LogCapture::default();
        let proxy =
            create_udp_framed_proxy(("127.0.0.1".to_owned(), 0), connect, log.logger()).await;
        let addr = proxy.expect("bind").local_addr;
        let app = udp_socket().await;
        app.send_to(b"x", addr).await.unwrap();
        log.wait_containing("ERROR", &format!("Remote error for {}: boom", id_of(&app)))
            .await;
        resend_until(&app, addr, || calls.load(SeqCst) == 2).await;
    }

    #[tokio::test]
    async fn closed_stream_is_logged_and_the_next_datagram_gets_a_new_stream() {
        // JS (testvectors/probes-js/pipe.cjs, `udpProxy_remoteEndVsDestroy`): a lone `end` of the write does not remove the entry from `clients`, the `Remote close` line
        // and the removal come from `close` (stream destruction). `drop(far)` is a full break: the reader sees EOF, writes fail,
        // so a whole-stream close is detected by the very first write into the stream.
        let (mut p, app, far) = sender_with_stream().await;
        drop(far);
        let mut second = new_stream_after_resend(&mut p, &app, b"again").await;
        p.log
            .wait_has("DEBUG", &format!("Remote close for {}", id_of(&app)))
            .await;
        assert_eq!(read_frame(&mut second).await, b"again");
    }

    #[tokio::test]
    async fn end_of_the_stream_reading_side_keeps_the_sender() {
        // A stream FIN (read EOF) does not remove the client; replies to the application and datagrams into the stream continue.
        let (mut p, app, mut far) = sender_with_stream().await;
        far.shutdown().await.unwrap();
        sleep(Duration::from_millis(100)).await;
        app.send_to(b"still", p.proxy.local_addr).await.unwrap();
        assert_eq!(read_frame(&mut far).await, b"still");
        p.remotes.none_new().await;
        assert!(
            !p.log
                .has("DEBUG", &format!("Remote close for {}", id_of(&app)))
        );
    }

    #[tokio::test]
    async fn failed_connect_leaves_the_proxy_alive_and_the_sender_may_retry() {
        let (remotes, connect) = remotes(1);
        let log = LogCapture::default();
        let proxy =
            create_udp_framed_proxy(("127.0.0.1".to_owned(), 0), connect, log.logger()).await;
        let mut p = Proxy {
            proxy: proxy.expect("bind"),
            remotes,
            log,
        };
        let app = udp_socket().await;
        app.send_to(b"x", p.proxy.local_addr).await.unwrap();
        wait_until("first connect to the remote made", || {
            p.remotes.calls() == 1
        })
        .await;
        let mut far = new_stream_after_resend(&mut p, &app, b"retry").await;
        assert_eq!(read_frame(&mut far).await, b"retry");
        assert_eq!(p.remotes.calls(), 2);
    }
}

mod log {
    use super::*;

    #[tokio::test]
    async fn datagram_from_a_sender_is_logged() {
        let (p, app, _far) = sender_with_stream().await;
        app.send_to(b"hello", p.proxy.local_addr).await.unwrap();
        p.log
            .wait_has(
                "DEBUG",
                &format!("UDP message from {}: 5 bytes", id_of(&app)),
            )
            .await;
    }

    #[tokio::test]
    async fn response_for_a_sender_is_logged() {
        let (p, app, mut far) = sender_with_stream().await;
        far.write_all(&frame(b"abc")).await.unwrap();
        p.log
            .wait_has(
                "DEBUG",
                &format!("UDP response for {}: 3 bytes", id_of(&app)),
            )
            .await;
    }
}

struct Proxy {
    proxy: UdpProxy,
    remotes: Remotes,
    log: LogCapture,
}

/// Proxy on `127.0.0.1:0`; `connect` gives a `duplex`.
async fn setup() -> Proxy {
    let (remotes, connect) = remotes(0);
    let log = LogCapture::default();
    let proxy = create_udp_framed_proxy(("127.0.0.1".to_owned(), 0), connect, log.logger()).await;
    Proxy {
        proxy: proxy.expect("bind on loopback"),
        remotes,
        log,
    }
}

/// Proxy, sender application and a stream already created for it (the first datagram went and was read).
async fn sender_with_stream() -> (Proxy, UdpSocket, tokio::io::DuplexStream) {
    let mut p = setup().await;
    let app = udp_socket().await;
    app.send_to(b"first", p.proxy.local_addr).await.unwrap();
    let mut far = p.remotes.next().await;
    assert_eq!(read_frame(&mut far).await, b"first");
    (p, app, far)
}

fn id_of(app: &UdpSocket) -> String {
    format!("127.0.0.1:{}", app.local_addr().unwrap().port())
}

async fn assert_cut_to_2048(sent_len: usize) {
    let (p, app, mut far) = sender_with_stream().await;
    let data = pattern(sent_len, 6);
    app.send_to(&data, p.proxy.local_addr).await.unwrap();
    assert!(
        read_n(&mut far, 4 + 2048).await == frame(&data[..2048]),
        "{sent_len} bytes: the frame holds the first 2048"
    );
    stays_pending("rest of the datagram after 2048 bytes", read_n(&mut far, 1)).await;
}

/// Sends `data` every 50 ms until `done` holds (the first datagrams may go into a dying stream).
async fn resend_until(app: &UdpSocket, to: SocketAddr, done: impl Fn() -> bool) {
    let sending = async {
        while !done() {
            app.send_to(b"retry", to).await.unwrap();
            sleep(Duration::from_millis(50)).await;
        }
    };
    within("new datagrams did not trigger connect", sending).await;
}

/// Sends `data` every 50 ms until `connect` creates a new stream; returns its far end.
async fn new_stream_after_resend(
    p: &mut Proxy,
    app: &UdpSocket,
    data: &[u8],
) -> tokio::io::DuplexStream {
    let finding = async {
        loop {
            app.send_to(data, p.proxy.local_addr).await.unwrap();
            if let Some(far) = p.remotes.next_within(Duration::from_millis(50)).await {
                return far;
            }
        }
    };
    within("sender did not get a new stream", finding).await
}
