// Tests of the `duplex` module: any `AsyncRead + AsyncWrite + Unpin + Send` is a `Duplex`; bytes, half-close, abort.
// Streams: `tokio::io::duplex` and `TcpStream` over 127.0.0.1 (port 0), always as `Box<dyn Duplex>`.
use super::*;
use std::io;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(5);

mod in_memory_pair {
    use super::*;

    #[tokio::test]
    async fn bytes_written_by_a_arrive_at_b_in_order() {
        let (mut a, mut b) = in_memory_pair();

        a.write_all(b"first,").await.unwrap();
        a.write_all(b"second").await.unwrap();

        assert_eq!(read_n(&mut b, 12).await, b"first,second");
    }

    #[tokio::test]
    async fn bytes_written_by_b_arrive_at_a() {
        let (mut a, mut b) = in_memory_pair();

        b.write_all(b"reply").await.unwrap();

        assert_eq!(read_n(&mut a, 5).await, b"reply");
    }
}

mod half_close {
    use super::*;

    #[tokio::test]
    async fn shutdown_delivers_written_bytes_then_eof_to_the_peer() {
        let (mut a, mut b) = in_memory_pair();

        a.write_all(b"last words").await.unwrap();
        a.shutdown().await.unwrap();

        let received = read_to_eof(&mut b).await;
        assert_eq!(
            received, b"last words",
            "all bytes written before shutdown must reach EOF"
        );
    }

    #[tokio::test]
    async fn shutdown_side_still_receives_what_the_peer_sends() {
        let (mut a, mut b) = in_memory_pair();

        a.shutdown().await.unwrap();
        b.write_all(b"late reply").await.unwrap();

        assert_eq!(read_n(&mut a, 10).await, b"late reply");
    }

    #[tokio::test]
    async fn write_to_a_dropped_peer_fails_instead_of_vanishing() {
        let (mut a, b) = in_memory_pair();

        drop(b);

        let err = a
            .write_all(b"x")
            .await
            .expect_err("writing to a closed stream must fail");
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    }
}

mod tcp_loopback {
    use super::*;

    #[tokio::test]
    async fn bytes_flow_both_ways() {
        let (mut client, mut server) = tcp_pair().await;

        client.write_all(b"request").await.unwrap();
        assert_eq!(read_n(&mut server, 7).await, b"request");
        server.write_all(b"response").await.unwrap();
        assert_eq!(read_n(&mut client, 8).await, b"response");
    }

    #[tokio::test]
    async fn shutdown_gives_peer_eof_and_keeps_reverse_direction_open() {
        let (mut client, mut server) = tcp_pair().await;

        client.shutdown().await.unwrap();
        let received = read_to_eof(&mut server).await;
        server.write_all(b"still open").await.unwrap();

        assert!(
            received.is_empty(),
            "after the client's shutdown the server reads EOF with no bytes"
        );
        assert_eq!(read_n(&mut client, 10).await, b"still open");
    }
}

mod boxed {
    use super::*;

    #[tokio::test]
    async fn boxed_stream_splits_and_its_halves_work_in_other_tasks() {
        let (a, mut b) = in_memory_pair();
        let (mut reader, mut writer) = tokio::io::split(a);

        let writing = tokio::spawn(async move {
            writer.write_all(b"from task").await.unwrap();
            writer.shutdown().await.unwrap();
        });
        let reading = tokio::spawn(async move { read_n(&mut reader, 7).await });
        b.write_all(b"to task").await.unwrap();

        assert_eq!(read_to_eof(&mut b).await, b"from task");
        assert_eq!(timeout(WAIT, reading).await.unwrap().unwrap(), b"to task");
        timeout(WAIT, writing).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn boxed_duplex_is_itself_a_duplex_and_can_be_wrapped_again() {
        let (a, mut b) = in_memory_pair();

        let mut wrapped = rewrap(a);
        wrapped.write_all(b"through wrapper").await.unwrap();

        assert_eq!(read_n(&mut b, 15).await, b"through wrapper");
    }
}

fn in_memory_pair() -> (Box<dyn Duplex>, Box<dyn Duplex>) {
    let (a, b) = tokio::io::duplex(64);
    (Box::new(a), Box::new(b))
}

async fn tcp_pair() -> (Box<dyn Duplex>, Box<dyn Duplex>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let both = timeout(WAIT, async {
        tokio::join!(TcpStream::connect(addr), listener.accept())
    });
    let (client, accepted) = both.await.expect("TCP pair was not set up in time");
    let (server, _) = accepted.unwrap();
    (Box::new(client.unwrap()), Box::new(server))
}

fn rewrap<S: Duplex + 'static>(stream: S) -> Box<dyn Duplex> {
    Box::new(stream)
}

async fn read_n<R: AsyncRead + Unpin>(stream: &mut R, n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    let read = timeout(WAIT, stream.read_exact(&mut buf)).await;
    read.expect("read did not finish in time")
        .expect("read failed");
    buf
}

async fn read_to_eof<R: AsyncRead + Unpin>(stream: &mut R) -> Vec<u8> {
    let mut buf = Vec::new();
    let read = timeout(WAIT, stream.read_to_end(&mut buf)).await;
    read.expect("no EOF in time").expect("read failed");
    buf
}
