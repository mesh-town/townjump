// Tests of writes into the UDP-mode tunnel (server `pipe_udp_framed_server` and client sender `run_sender`):
// datagrams are neither lost nor merged, a write does not wait for an ACK per datagram, the tail leaves without new datagrams.
// The stub transport repeats two properties of the UDX/SecretStream stream: `flush` waits for an ACK, a frame may sit in the transport.
use crate::support::*;
use crate::udp_client::{Sender, run_sender};
use crate::udp_server::pipe_udp_framed_server;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, ready};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf, duplex};
use tokio::net::UdpSocket;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

const BUF: usize = 4 * 1024 * 1024;

/// How the transport's `flush` behaves.
#[derive(Clone, Copy, Debug)]
enum Flush {
    /// Completes at once.
    Instant,
    /// The ACK never comes.
    NeverAcked,
    /// What was written sits in the transport and goes on the wire only with the next write or `flush`.
    HoldsTail,
}

struct Transport {
    wire: DuplexStream,
    held: Vec<u8>,
    flush: Flush,
}

impl Transport {
    fn release(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        while !self.held.is_empty() {
            let n = ready!(Pin::new(&mut self.wire).poll_write(cx, &self.held))?;
            self.held.drain(..n);
        }
        Poll::Ready(Ok(()))
    }
}

impl AsyncRead for Transport {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.wire).poll_read(cx, buf)
    }
}

impl AsyncWrite for Transport {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let Flush::HoldsTail = self.flush else {
            return Pin::new(&mut self.wire).poll_write(cx, buf);
        };
        ready!(self.release(cx))?;
        self.held.extend_from_slice(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if let Flush::NeverAcked = self.flush {
            return Poll::Pending;
        }
        ready!(self.release(cx))?;
        Pin::new(&mut self.wire).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        ready!(self.release(cx))?;
        Pin::new(&mut self.wire).poll_shutdown(cx)
    }
}

fn transport(flush: Flush) -> (Transport, DuplexStream) {
    let (far, wire) = duplex(BUF);
    let held = Vec::new();
    (Transport { wire, held, flush }, far)
}

/// Where the test feeds datagrams from: the server's UDP socket or the client sender's channel.
#[derive(Clone, Copy, Debug)]
enum Side {
    Server,
    Client,
}

enum Entry {
    Server(UdpSocket, SocketAddr),
    Client(UnboundedSender<Vec<u8>>),
}

impl Entry {
    async fn send(&self, datagram: &[u8]) {
        match self {
            Entry::Server(service, to) => {
                service.send_to(datagram, *to).await.unwrap();
            }
            Entry::Client(tx) => tx.send(datagram.to_vec()).unwrap(),
        }
    }
}

type Run = Pin<Box<dyn Future<Output = ()>>>;

/// Starts a side over the transport; returns its future, the stream's far end and the datagram input.
async fn rig(side: Side, flush: Flush) -> (Run, DuplexStream, Entry) {
    let (near, far) = transport(flush);
    match side {
        Side::Server => server_rig(near, far).await,
        Side::Client => client_rig(near, far).await,
    }
}

async fn server_rig(near: Transport, mut far: DuplexStream) -> (Run, DuplexStream, Entry) {
    let service = udp_socket().await;
    let port = service.local_addr().unwrap().port();
    let watch = Watch::new(false);
    let run = pipe_udp_framed_server(
        near,
        ("127.0.0.1".to_owned(), port),
        watch.log.logger(),
        watch.stats.clone(),
    );
    let mut run: Run = Box::pin(until_end(watch.ended.clone(), run));
    let to = drive(&mut run, learn_server_addr(&mut far, &service)).await;
    (run, far, Entry::Server(service, to))
}

/// The server replies to the service from the socket that received its frame: the first frame from the stream opens this address.
async fn learn_server_addr(far: &mut DuplexStream, service: &UdpSocket) -> SocketAddr {
    far.write_all(&frame(b"hello")).await.unwrap();
    let (body, from) = recv(service).await;
    assert_eq!(body, b"hello");
    from
}

async fn client_rig(near: Transport, far: DuplexStream) -> (Run, DuplexStream, Entry) {
    let socket = Arc::new(udp_socket().await);
    let logger = LogCapture::default().logger();
    let sender = Sender {
        id: "test".to_owned(),
        addr: socket.local_addr().unwrap(),
        socket,
        logger,
    };
    let (tx, rx) = unbounded_channel();
    let run: Run = Box::pin(run_sender(
        sender,
        async move { Ok::<_, io::Error>(near) },
        rx,
    ));
    (run, far, Entry::Client(tx))
}

/// Datagrams of different lengths, including empty and maximum (2048).
fn datagrams(count: usize) -> Vec<Vec<u8>> {
    const LENS: [usize; 8] = [0, 1, 2, 3, 77, 500, 1400, 2048];
    (0..count)
        .map(|i| pattern(LENS[i % LENS.len()], i as u32 + 1))
        .collect()
}

/// Sends datagrams and checks that the other end of the stream gets the same frames in the same order.
async fn assert_all_arrive(side: Side, flush: Flush, count: usize) {
    let (run, mut far, entry) = rig(side, flush).await;
    let sent = datagrams(count);
    drive(run, async {
        for d in &sent {
            entry.send(d).await;
        }
        for (i, d) in sent.iter().enumerate() {
            let body = read_frame(&mut far).await;
            assert!(
                body == *d,
                "{side:?} {flush:?}: frame {i} did not match the datagram"
            );
        }
        stays_pending("extra frame after all", read_n(&mut far, 1)).await;
    })
    .await;
}

macro_rules! for_both_sides {
    ($($name:ident: $flush:expr, $count:expr;)*) => {$(
        mod $name {
            use super::*;

            #[tokio::test]
            async fn server() {
                assert_all_arrive(Side::Server, $flush, $count).await;
            }

            #[tokio::test]
            async fn client() {
                assert_all_arrive(Side::Client, $flush, $count).await;
            }
        }
    )*};
}

for_both_sides! {
    boundaries_and_order_are_kept: Flush::Instant, 96;
    burst_passes_although_ack_never_comes: Flush::NeverAcked, 96;
    burst_tail_leaves_the_transport_without_new_writes: Flush::HoldsTail, 96;
    single_datagram_leaves_the_transport_without_new_writes: Flush::HoldsTail, 1;
}
