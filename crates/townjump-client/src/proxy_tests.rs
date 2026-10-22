// Tests for client data: local TCP/UDP entry point -> new DHT connection to the mock server (client.md «TCP (`handleTCP`)», «UDP (`handleUDP`)», «The rest of the API»).
// The server is `Remote` from `crate::support`; UDP frames are `[u32 BE][body]`; `ev_connect` and helpers are at the bottom.
use crate::support::*;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

mod tcp {
    use super::*;

    #[tokio::test]
    async fn local_client_bytes_reach_the_server_over_a_dht_connection() {
        let mut f = setup(Opts::default()).await;
        let port = f.start().await;
        let mut server = remote(&f.factory, PUBLIC_PK).await;
        let mut client = tcp_connect(port).await;
        client.write_all(b"ping").await.expect("write");
        let mut conn = server.next().await;
        assert_eq!(read_n(&mut conn.stream, 4).await, b"ping");
        assert_eq!(f.named("connect"), vec![ev_connect(PUBLIC_PK, true)]);
    }

    #[tokio::test]
    async fn server_answer_goes_back_to_the_local_client() {
        let mut f = setup(Opts::default()).await;
        let port = f.start().await;
        let mut server = remote(&f.factory, PUBLIC_PK).await;
        let mut client = tcp_connect(port).await;
        let mut conn = server.next().await;
        conn.stream.write_all(b"pong").await.expect("write");
        assert_eq!(read_n(&mut client, 4).await, b"pong");
    }

    #[tokio::test]
    async fn every_local_connection_gets_its_own_dht_connection() {
        let mut f = setup(Opts::default()).await;
        let port = f.start().await;
        let mut server = remote(&f.factory, PUBLIC_PK).await;
        let (_first, _second) = (tcp_connect(port).await, tcp_connect(port).await);
        let (_a, _b) = (server.next().await, server.next().await);
        assert_eq!(f.named("connect").len(), 2);
    }

    #[tokio::test]
    async fn no_dht_connection_is_opened_before_a_local_client_arrives() {
        let mut f = setup(Opts::default()).await;
        f.start().await;
        let mut server = remote(&f.factory, PUBLIC_PK).await;
        server.none_new().await;
        assert!(
            f.named("connect").is_empty(),
            "connect without a client: {:?}",
            f.named("connect")
        );
    }

    #[tokio::test]
    async fn private_client_connects_to_the_derived_public_key() {
        let mut f = setup(Opts::private()).await;
        let port = f.start().await;
        let mut server = remote(&f.factory, PRIVATE_PK).await;
        let _client = tcp_connect(port).await;
        let _conn = server.next().await;
        assert_eq!(f.named("connect"), vec![ev_connect(PRIVATE_PK, true)]);
    }

    #[tokio::test]
    async fn missing_server_closes_only_that_local_connection_and_the_client_keeps_listening() {
        let mut f = setup(Opts::default()).await;
        let port = f.start().await;
        let mut lost = tcp_connect(port).await;
        assert!(
            is_closed(&mut lost).await,
            "no server - the local connection is closed"
        );
        assert_eq!(f.info().state, Some("listening"));
        let mut server = remote(&f.factory, PUBLIC_PK).await;
        let _again = tcp_connect(port).await;
        let _conn = server.next().await;
    }
}

mod udp {
    use super::*;

    #[tokio::test]
    async fn datagram_reaches_the_server_as_a_length_prefixed_frame_on_a_plain_connect() {
        let mut f = setup(Opts::default()).await;
        let port = f.start_udp().await;
        let mut server = remote(&f.factory, PUBLIC_PK).await;
        let socket = udp_socket().await;
        socket
            .send_to(b"hello", ("127.0.0.1", port))
            .await
            .expect("send_to");
        let mut conn = server.next().await;
        assert_eq!(read_n(&mut conn.stream, 9).await, b"\0\0\0\x05hello");
        assert_eq!(
            f.named("connect"),
            vec![ev_connect(PUBLIC_PK, false)],
            "UDP - without reusableSocket"
        );
    }

    #[tokio::test]
    async fn answer_frame_goes_back_to_the_source_socket() {
        let mut f = setup(Opts::default()).await;
        let port = f.start_udp().await;
        let mut server = remote(&f.factory, PUBLIC_PK).await;
        let socket = udp_socket().await;
        socket
            .send_to(b"hello", ("127.0.0.1", port))
            .await
            .expect("send_to");
        let mut conn = server.next().await;
        conn.stream
            .write_all(b"\0\0\0\x03abc")
            .await
            .expect("write");
        let mut buf = [0u8; 16];
        let (n, _from) = within("reply datagram", socket.recv_from(&mut buf))
            .await
            .expect("recv_from");
        assert_eq!(&buf[..n], b"abc");
    }

    #[tokio::test]
    async fn destroy_closes_the_stream_of_every_udp_client() {
        let mut f = setup(Opts::default()).await;
        let port = f.start_udp().await;
        let mut server = remote(&f.factory, PUBLIC_PK).await;
        let socket = udp_socket().await;
        socket
            .send_to(b"hi", ("127.0.0.1", port))
            .await
            .expect("send_to");
        let mut conn = server.next().await;
        // The `hi` frame already sits in the stream buffer and is read through to EOF (`duplex` hands out what was written before closing).
        assert_eq!(read_n(&mut conn.stream, 6).await, b"\0\0\0\x02hi");
        within("destroy", f.client.destroy())
            .await
            .expect("destroy");
        assert!(
            is_closed(&mut conn.stream).await,
            "remoteStream.destroy() for each client"
        );
    }
}

/// `connect` trace event: TCP - with `reusableSocket`, UDP - without options.
fn ev_connect(pk: &str, tcp: bool) -> Value {
    if tcp {
        return json!({ "ev": "connect", "publicKey": pk, "opts": { "reusableSocket": true } });
    }
    json!({ "ev": "connect", "publicKey": pk })
}
