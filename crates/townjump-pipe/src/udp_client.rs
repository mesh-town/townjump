// JS: hyper-cmd-lib-net/index.js:createUdpFramedProxy (a `clients` map entry: the DHT stream of one local sender).
// Related: udp_proxy.rs (socket and sender map), udp_frame.rs (frames).
use crate::udp_frame::{FrameDecoder, encode_frame};
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::net::UdpSocket;
use tokio::sync::mpsc::UnboundedReceiver;
use townjump_logger::Logger;

/// Everything the sender's task knows about it: id `address:port`, the proxy's shared socket, log.
pub(crate) struct Sender {
    pub id: String,
    pub addr: SocketAddr,
    pub socket: Arc<UdpSocket>,
    pub logger: Logger,
}

/// Drives one sender's DHT stream: datagrams from `rx` go into the stream as frames, frames from the stream go to it as datagrams.
/// The sender's read end (FIN) does not remove it: the task stays alive until the stream is closed entirely —
/// read error, write rejection or channel close.
pub(crate) async fn run_sender<S, Fut>(
    sender: Sender,
    connect: Fut,
    mut rx: UnboundedReceiver<Vec<u8>>,
) where
    S: AsyncRead + AsyncWrite + Unpin,
    Fut: Future<Output = io::Result<S>>,
{
    let stream = match connect.await {
        Ok(stream) => stream,
        Err(e) => return sender.remote_error(&e),
    };
    let (mut rd, mut wr) = tokio::io::split(stream);
    let mut decoder = FrameDecoder::default();
    let end = pump_sender(&sender, &mut rd, &mut wr, &mut decoder, &mut rx).await;
    end.err().iter().for_each(|e| sender.remote_error(e));
    sender
        .logger
        .debug(&format!("Remote close for {}", sender.id));
}

/// What happened first: bytes arrived from the stream or a datagram from the local sender.
enum Event {
    Stream(io::Result<usize>),
    Datagram(Option<Vec<u8>>),
}

async fn next_event<R: AsyncRead + Unpin>(
    rd: &mut R,
    rx: &mut UnboundedReceiver<Vec<u8>>,
    buf: &mut [u8],
    eof: bool,
) -> Event {
    tokio::select! {
        n = rd.read(buf), if !eof => Event::Stream(n),
        dgram = rx.recv() => Event::Datagram(dgram),
    }
}

/// A stream write does not wait for an ACK per datagram (in JS `remoteStream.write` without waiting): `flush` waits for an ACK of everything
/// sent and would hold both sides. It runs only when idle, so that a tail left by the transport with a full window
/// does not sit until the next datagram (as in `pump::next_chunk`).
async fn pump_sender<S: AsyncRead + AsyncWrite>(
    sender: &Sender,
    rd: &mut ReadHalf<S>,
    wr: &mut WriteHalf<S>,
    decoder: &mut FrameDecoder,
    rx: &mut UnboundedReceiver<Vec<u8>>,
) -> io::Result<()> {
    let (mut buf, mut eof, mut unflushed) = (vec![0u8; 16 * 1024], false, false);
    loop {
        let event = tokio::select! {
            biased;
            event = next_event(rd, rx, &mut buf, eof) => event,
            flushed = wr.flush(), if unflushed => {
                unflushed = false;
                flushed.map_err(closed)?;
                continue;
            }
        };
        match event {
            Event::Stream(n) => match n? {
                0 => eof = true,
                n => sender.deliver(decoder, &buf[..n]).await,
            },
            Event::Datagram(Some(d)) => {
                write_frame(wr, &d).await?;
                unflushed = true;
            }
            Event::Datagram(None) => return Ok(()),
        }
    }
}

/// A write rejection is a close of the whole stream; it is not logged as an error (in JS this is `close`).
async fn write_frame<W: AsyncWrite + Unpin>(wr: &mut W, datagram: &[u8]) -> io::Result<()> {
    wr.write_all(&encode_frame(datagram)).await.map_err(closed)
}

fn closed(_: io::Error) -> io::Error {
    io::ErrorKind::BrokenPipe.into()
}

impl Sender {
    fn remote_error(&self, e: &io::Error) {
        self.logger
            .error(&format!("Remote error for {}: {e}", self.id));
    }

    async fn deliver(&self, decoder: &mut FrameDecoder, chunk: &[u8]) {
        for body in decoder.push(chunk) {
            self.logger.debug(&format!(
                "UDP response for {}: {} bytes",
                self.id,
                body.len()
            ));
            if let Err(e) = self.socket.send_to(&body, self.addr).await {
                self.logger
                    .error(&format!("Send error to {}: {e}", self.id));
            }
        }
    }
}
