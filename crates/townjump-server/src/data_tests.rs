// Tests for the data path: incoming DHT connection -> local TCP or UDP service (frames `[u32 BE length][body]`).
// Source: server.md «Incoming connection (`onConnection(c)`)», pipe.md; `udp='false'` selects the UDP branch.
use super::fixture::*;
use std::net::SocketAddr;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::time::timeout;
use townjump_dht::DhtConnection;
use townjump_jsval::JsValue;

mod tcp {
    use super::*;

    #[tokio::test]
    async fn client_bytes_reach_the_service() {
        let mut t = tunnel().await;
        send(&mut t.conn.stream, b"ping").await;
        assert_eq!(read_n(&mut t.local, 4).await, b"ping");
    }

    #[tokio::test]
    async fn service_bytes_reach_the_client() {
        let mut t = tunnel().await;
        send(&mut t.local, b"pong").await;
        assert_eq!(read_n(&mut t.conn.stream, 4).await, b"pong");
    }

    #[tokio::test]
    async fn two_connections_carry_their_own_bytes() {
        let mut t = tunnel().await;
        let (mut second, mut second_local) = extra(&t).await;
        send(&mut t.conn.stream, b"one").await;
        send(&mut second.stream, b"two").await;
        assert_eq!(read_n(&mut t.local, 3).await, b"one");
        assert_eq!(read_n(&mut second_local, 3).await, b"two");
    }

    mod closing {
        use super::*;

        #[tokio::test]
        async fn client_side_closes_only_its_own_service_socket() {
            let mut t = tunnel().await;
            let (mut second, mut second_local) = extra(&t).await;
            drop(t.conn);
            assert!(
                is_closed(&mut t.local).await,
                "local socket of the closed client is alive"
            );
            send(&mut second.stream, b"alive").await;
            assert_eq!(read_n(&mut second_local, 5).await, b"alive");
        }

        #[tokio::test]
        async fn service_side_closes_the_client_stream() {
            let mut t = tunnel().await;
            drop(t.local);
            assert!(
                is_closed(&mut t.conn.stream).await,
                "client stream is not closed"
            );
        }
    }

    mod local_port_refuses {
        use super::*;

        #[tokio::test]
        async fn closes_the_client_stream_and_keeps_the_server_alive() {
            let fx = Fixture::started(Setup {
                port: free_port().await,
                ..Setup::default()
            })
            .await;
            let mut first = fx.connect_as(server_pair()).await.expect("connect");
            assert!(
                is_closed(&mut first.stream).await,
                "stream is not closed on port rejection"
            );
            fx.connect_as(server_pair())
                .await
                .expect("server stopped accepting");
        }

        #[tokio::test]
        async fn is_not_fatal() {
            let mut fx = Fixture::started(Setup {
                port: free_port().await,
                ..Setup::default()
            })
            .await;
            let mut conn = fx.connect_as(server_pair()).await.expect("connect");
            assert!(is_closed(&mut conn.stream).await);
            assert!(
                timeout(SILENCE, fx.fatal.recv()).await.is_err(),
                "port rejection went to Fatal"
            );
        }
    }

    mod log {
        use super::*;

        #[tokio::test]
        async fn writes_incoming_handling_and_piped_lines_in_order() {
            let t = tunnel().await;
            t.fx.wait_log("DEBUG", "TCP connection piped").await;
            let debug = t.fx.messages("DEBUG");
            let tail = &debug[debug.len() - 3..];
            let expected = [
                incoming(),
                "Handling TCP connection".into(),
                "TCP connection piped".into(),
            ];
            assert_eq!(tail, expected);
        }

        #[tokio::test]
        async fn disconnect_line_waits_for_the_last_connection_of_the_key() {
            let t = tunnel().await;
            let (second, _second_local) = extra(&t).await;
            let Tunnel {
                fx,
                conn,
                mut local,
                ..
            } = t;
            drop(conn);
            assert!(is_closed(&mut local).await);
            tokio::time::sleep(SILENCE).await;
            assert!(
                !fx.has("DEBUG", &disconnected()),
                "Disconnected with a live second connection"
            );
            drop(second);
            fx.wait_log("DEBUG", &disconnected()).await;
        }
    }

    mod branch {
        use super::*;

        #[tokio::test]
        async fn udp_false_takes_the_tcp_branch() {
            let fx = branch_server(JsValue::Bool(false)).await;
            fx.connect_as(server_pair()).await.expect("connect");
            fx.wait_log("DEBUG", "Handling TCP connection").await;
        }

        #[tokio::test]
        async fn q6_udp_string_false_takes_the_udp_branch() {
            let fx = branch_server(JsValue::String("false".to_string())).await;
            fx.connect_as(server_pair()).await.expect("connect");
            fx.wait_log("DEBUG", "Handling UDP connection").await;
        }
    }
}

mod udp {
    use super::*;

    #[tokio::test]
    async fn frame_becomes_one_datagram() {
        let mut u = udp_tunnel().await;
        send(&mut u.conn.stream, &frame(b"hello")).await;
        assert_eq!(u.service.recv().await.0, b"hello");
    }

    #[tokio::test]
    async fn datagram_becomes_one_frame() {
        let mut u = udp_tunnel().await;
        send(&mut u.conn.stream, &frame(b"hi")).await;
        let (_, from) = u.service.recv().await;
        u.service.0.send_to(b"reply", from).await.expect("send_to");
        assert_eq!(read_n(&mut u.conn.stream, 9).await, frame(b"reply"));
    }

    #[tokio::test]
    async fn two_frames_in_one_write_are_two_datagrams() {
        let mut u = udp_tunnel().await;
        send(&mut u.conn.stream, &[frame(b"a"), frame(b"bc")].concat()).await;
        assert_eq!(u.service.recv().await.0, b"a");
        assert_eq!(u.service.recv().await.0, b"bc");
    }

    #[tokio::test]
    async fn frame_split_across_writes_is_one_datagram() {
        let mut u = udp_tunnel().await;
        let whole = frame(b"split");
        send(&mut u.conn.stream, &whole[..2]).await;
        send(&mut u.conn.stream, &whole[2..]).await;
        assert_eq!(u.service.recv().await.0, b"split");
    }

    #[tokio::test]
    async fn each_dht_connection_gets_its_own_local_socket() {
        let mut u = udp_tunnel().await;
        let mut second = u.fx.connect_as(server_pair()).await.expect("connect");
        send(&mut u.conn.stream, &frame(b"1")).await;
        send(&mut second.stream, &frame(b"2")).await;
        let (first_from, second_from) = (u.service.recv().await.1, u.service.recv().await.1);
        assert_ne!(
            first_from, second_from,
            "one address pair for two connections"
        );
    }

    #[tokio::test]
    async fn logs_handling_and_framed_and_piped() {
        let u = udp_tunnel().await;
        u.fx.wait_log("DEBUG", "UDP connection framed and piped")
            .await;
        assert!(u.fx.has("DEBUG", "Handling UDP connection"));
    }
}

/// TCP path: server, service, client and the socket the service accepted.
struct Tunnel {
    fx: Fixture,
    service: Backend,
    conn: DhtConnection,
    local: TcpStream,
}

async fn tunnel() -> Tunnel {
    let service = backend().await;
    let opts = Setup {
        port: service.port(),
        ..Setup::default()
    };
    let fx = Fixture::started(opts).await;
    let conn = fx
        .connect_as(server_pair())
        .await
        .expect("client did not connect");
    let local = service.accept().await;
    Tunnel {
        fx,
        service,
        conn,
        local,
    }
}

/// Second connection of the same client and the socket accepted for it.
async fn extra(t: &Tunnel) -> (DhtConnection, TcpStream) {
    let conn =
        t.fx.connect_as(server_pair())
            .await
            .expect("second connection");
    (conn, t.service.accept().await)
}

async fn branch_server(udp: JsValue) -> Fixture {
    Fixture::started(Setup {
        port: free_port().await,
        udp: Some(udp),
        ..Setup::default()
    })
    .await
}

fn incoming() -> String {
    format!(
        "Incoming connection received from {}",
        z32(&server_pair().public_key)
    )
}

fn disconnected() -> String {
    format!("Disconnected from {}", z32(&server_pair().public_key))
}

async fn send<W: AsyncWrite + Unpin>(stream: &mut W, bytes: &[u8]) {
    let sent = timeout(WAIT, async {
        stream.write_all(bytes).await?;
        stream.flush().await
    })
    .await;
    sent.expect("write did not finish within 5 s")
        .expect("write error");
}

/// `[u32 BE length][body]`.
fn frame(body: &[u8]) -> Vec<u8> {
    let mut framed = u32::try_from(body.len())
        .expect("length")
        .to_be_bytes()
        .to_vec();
    framed.extend_from_slice(body);
    framed
}

/// UDP path: a server with `udp = true`, a local UDP service and one client.
struct UdpTunnel {
    fx: Fixture,
    service: UdpService,
    conn: DhtConnection,
}

struct UdpService(UdpSocket);

impl UdpService {
    /// Next datagram and the sender address (5 s).
    async fn recv(&self) -> (Vec<u8>, SocketAddr) {
        let mut buf = vec![0u8; 2048];
        let got = timeout(WAIT, self.0.recv_from(&mut buf)).await;
        let (n, from) = got
            .expect("datagram did not arrive within 5 s")
            .expect("recv_from");
        buf.truncate(n);
        (buf, from)
    }
}

async fn udp_tunnel() -> UdpTunnel {
    let socket = UdpSocket::bind("127.0.0.1:0").await.expect("bind udp");
    let port = f64::from(socket.local_addr().expect("addr").port());
    let opts = Setup {
        port: JsValue::Number(port),
        udp: Some(JsValue::Bool(true)),
        ..Setup::default()
    };
    let fx = Fixture::started(opts).await;
    let conn = fx
        .connect_as(server_pair())
        .await
        .expect("client did not connect");
    UdpTunnel {
        fx,
        service: UdpService(socket),
        conn,
    }
}
