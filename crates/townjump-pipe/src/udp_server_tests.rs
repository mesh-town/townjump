// Tests of `pipe_udp_framed_server`: stream frames ↔ datagrams of its own socket per connection (pipe.md, `pipeUdpFramedServer`).
// The target service is a UDP socket on 127.0.0.1:0; `attach` and helpers are at the bottom of the file.
use super::*;
use crate::support::*;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncWriteExt, DuplexStream, duplex};
use tokio::net::UdpSocket;
use tokio::time::sleep;

mod frame_to_service {
    use super::*;

    #[tokio::test]
    async fn frame_becomes_a_datagram_at_local() {
        let (service, port) = service().await;
        let watch = Watch::new(false);
        let (mut far, server) = attach(&watch, port);
        drive(server, async {
            far.write_all(&[0, 0, 0, 5, b'h', b'e', b'l', b'l', b'o'])
                .await
                .unwrap();
            assert_eq!(recv(&service).await.0, b"hello");
        })
        .await;
    }

    #[tokio::test]
    async fn zero_length_frame_becomes_an_empty_datagram() {
        let (service, port) = service().await;
        let watch = Watch::new(false);
        let (mut far, server) = attach(&watch, port);
        drive(server, async {
            far.write_all(&[0, 0, 0, 0]).await.unwrap();
            assert_eq!(recv(&service).await.0, Vec::<u8>::new());
        })
        .await;
    }

    #[tokio::test]
    async fn two_frames_in_one_write_become_two_datagrams_in_order() {
        let (service, port) = service().await;
        let watch = Watch::new(false);
        let (mut far, server) = attach(&watch, port);
        drive(server, async {
            far.write_all(&[0, 0, 0, 1, b'1', 0, 0, 0, 2, b'2', b'2'])
                .await
                .unwrap();
            assert_eq!(recv(&service).await.0, b"1");
            assert_eq!(recv(&service).await.0, b"22");
        })
        .await;
    }

    #[tokio::test]
    async fn frame_written_in_pieces_becomes_one_datagram() {
        let (service, port) = service().await;
        let watch = Watch::new(false);
        let (mut far, server) = attach(&watch, port);
        drive(server, async {
            for piece in [&[0u8, 0][..], &[0, 3, b'a'], b"bc"] {
                far.write_all(piece).await.unwrap();
                far.flush().await.unwrap();
                sleep(Duration::from_millis(20)).await;
            }
            assert_eq!(recv(&service).await.0, b"abc");
            stays_pending("second datagram from one frame", recv(&service)).await;
        })
        .await;
    }

    #[tokio::test]
    async fn frames_keep_their_order() {
        let (service, port) = service().await;
        let watch = Watch::new(false);
        let (mut far, server) = attach(&watch, port);
        let sent: Vec<Vec<u8>> = (0..30).map(|i| pattern(10 + i, i as u32 + 1)).collect();
        drive(server, async {
            for datagram in &sent {
                far.write_all(&frame(datagram)).await.unwrap();
            }
            for datagram in &sent {
                assert!(
                    recv(&service).await.0 == *datagram,
                    "datagrams arrived out of frame order"
                );
            }
        })
        .await;
    }

    #[tokio::test]
    async fn frame_of_2048_bytes_passes_whole() {
        let (service, port) = service().await;
        let watch = Watch::new(false);
        let (mut far, server) = attach(&watch, port);
        let data = pattern(2048, 8);
        drive(server, async {
            far.write_all(&frame(&data)).await.unwrap();
            assert!(
                recv(&service).await.0 == data,
                "2048-byte datagram did not arrive whole"
            );
        })
        .await;
    }
}

mod service_to_frame {
    use super::*;

    #[tokio::test]
    async fn datagram_becomes_a_frame_with_a_length_prefix() {
        let (service, port) = service().await;
        let watch = Watch::new(false);
        let (mut far, server) = attach(&watch, port);
        drive(server, async {
            let to_server = first_datagram(&mut far, &service).await;
            service.send_to(b"pong", to_server).await.unwrap();
            assert_eq!(
                read_n(&mut far, 8).await,
                [0, 0, 0, 4, b'p', b'o', b'n', b'g']
            );
        })
        .await;
    }

    #[tokio::test]
    async fn empty_datagram_becomes_a_zero_length_frame() {
        let (service, port) = service().await;
        let watch = Watch::new(false);
        let (mut far, server) = attach(&watch, port);
        drive(server, async {
            let to_server = first_datagram(&mut far, &service).await;
            service.send_to(b"", to_server).await.unwrap();
            assert_eq!(read_n(&mut far, 4).await, [0, 0, 0, 0]);
        })
        .await;
    }

    #[tokio::test]
    async fn datagram_of_2048_bytes_passes_whole() {
        let (service, port) = service().await;
        let watch = Watch::new(false);
        let (mut far, server) = attach(&watch, port);
        let data = pattern(2048, 5);
        drive(server, async {
            let to_server = first_datagram(&mut far, &service).await;
            service.send_to(&data, to_server).await.unwrap();
            assert!(
                read_n(&mut far, 4 + 2048).await == frame(&data),
                "2048-byte frame: [0,0,8,0] and the whole body"
            );
        })
        .await;
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
        let (service, port) = service().await;
        let watch = Watch::new(false);
        let (mut far, server) = attach(&watch, port);
        let sent: Vec<Vec<u8>> = (0..30).map(|i| pattern(10 + i, i as u32 + 1)).collect();
        drive(server, async {
            let to_server = first_datagram(&mut far, &service).await;
            for datagram in &sent {
                service.send_to(datagram, to_server).await.unwrap();
            }
            for datagram in &sent {
                assert!(
                    read_frame(&mut far).await == *datagram,
                    "frames arrived out of datagram order"
                );
            }
        })
        .await;
    }

    #[tokio::test]
    async fn datagram_from_any_other_sender_is_forwarded_too() {
        let (service, port) = service().await;
        let watch = Watch::new(false);
        let (mut far, server) = attach(&watch, port);
        drive(server, async {
            let to_server = first_datagram(&mut far, &service).await;
            let stranger = udp_socket().await;
            stranger.send_to(b"stranger", to_server).await.unwrap();
            assert_eq!(
                read_frame(&mut far).await,
                b"stranger",
                "no filter by sender"
            );
        })
        .await;
    }
}

mod own_socket_per_connection {
    use super::*;

    #[tokio::test]
    async fn two_connections_send_from_different_ports_and_get_their_own_answers() {
        let (service, port) = service().await;
        let (first, second) = (Watch::new(false), Watch::new(false));
        let (mut far1, server1) = attach(&first, port);
        let (mut far2, server2) = attach(&second, port);
        drive(
            async {
                tokio::join!(server1, server2);
            },
            async {
                let from1 = first_datagram(&mut far1, &service).await;
                let from2 = first_datagram(&mut far2, &service).await;
                assert_ne!(from1, from2, "each connection has its own socket");
                service.send_to(b"to-2", from2).await.unwrap();
                assert_eq!(read_frame(&mut far2).await, b"to-2");
                stays_pending("2nd reply reached the 1st", read_n(&mut far1, 1)).await;
            },
        )
        .await;
    }

    #[tokio::test]
    async fn the_source_port_stays_the_same_within_one_connection() {
        let (service, port) = service().await;
        let watch = Watch::new(false);
        let (mut far, server) = attach(&watch, port);
        drive(server, async {
            let first = first_datagram(&mut far, &service).await;
            far.write_all(&frame(b"again")).await.unwrap();
            let (body, second) = recv(&service).await;
            assert_eq!((body.as_slice(), second), (&b"again"[..], first));
        })
        .await;
    }
}

/// Target service: a UDP socket on loopback and its port.
pub(super) async fn service() -> (UdpSocket, u16) {
    let socket = udp_socket().await;
    let port = socket.local_addr().expect("local_addr").port();
    (socket, port)
}

/// `pipe_udp_framed_server` for a new incoming stream; returns its far end and the server's future.
pub(super) fn attach(watch: &Watch, port: u16) -> (DuplexStream, impl Future<Output = ()> + use<>) {
    let (far, near) = duplex(PIPE_BUF);
    let local = ("127.0.0.1".to_owned(), port);
    let server = pipe_udp_framed_server(near, local, watch.log.logger(), watch.stats.clone());
    (far, until_end(watch.ended.clone(), server))
}

/// Sends a frame, waits for the datagram at the service and returns the server socket's address: the service replies there.
pub(super) async fn first_datagram(far: &mut DuplexStream, service: &UdpSocket) -> SocketAddr {
    far.write_all(&frame(b"hello")).await.expect("frame write");
    let (body, from) = recv(service).await;
    assert_eq!(body, b"hello");
    from
}

async fn assert_cut_to_2048(sent_len: usize) {
    let (service, port) = service().await;
    let watch = Watch::new(false);
    let (mut far, server) = attach(&watch, port);
    let data = pattern(sent_len, 9);
    drive(server, async {
        let to_server = first_datagram(&mut far, &service).await;
        service.send_to(&data, to_server).await.unwrap();
        assert!(
            read_n(&mut far, 4 + 2048).await == frame(&data[..2048]),
            "{sent_len} bytes: the frame holds the first 2048"
        );
        stays_pending("rest of the datagram after 2048 bytes", read_n(&mut far, 1)).await;
    })
    .await;
}
