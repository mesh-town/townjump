// JS: @holesail/hyper-cmd-lib-net/index.js:pipeUdpFramedServer (its own implicitly bound udp4 socket per connection).
// Related: udp_frame.rs (frames), udp_proxy.rs (client side).
use crate::stats::PipeStats;
use crate::udp_frame::{FrameDecoder, encode_frame};
use crate::udp_recv::{RECV_BUF, recv_next};
use std::io;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::UdpSocket;
use townjump_logger::Logger;

/// Frames from `remote` go to `local` as datagrams; any datagram on the socket is packed back into a frame.
/// Returns when the stream ended or broke (socket closed, stream closed).
pub async fn pipe_udp_framed_server<S>(
    remote: S,
    local: (String, u16),
    logger: Logger,
    stats: Arc<PipeStats>,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let _ = stats; // JS: not used for UDP
    let Ok(socket) = UdpSocket::bind(("0.0.0.0", 0)).await else {
        return;
    };
    let (mut rd, mut wr) = tokio::io::split(remote);
    serve(&mut rd, &mut wr, &socket, &local, &logger).await;
    let _ = wr.shutdown().await;
}

/// What happened first: bytes arrived from the stream or a datagram on the socket.
enum Event {
    Stream(io::Result<usize>),
    Socket(io::Result<usize>),
}

async fn next_event<R: AsyncRead + Unpin>(
    rd: &mut R,
    socket: &UdpSocket,
    chunk: &mut [u8],
    dgram: &mut [u8],
    logger: &Logger,
) -> Event {
    tokio::select! {
        n = rd.read(chunk) => Event::Stream(n),
        got = recv_next(socket, dgram, logger) => Event::Socket(got.map(|(n, _)| n)),
    }
}

/// A stream write does not wait for an ACK per datagram (in JS `remoteStream.write` without waiting): `flush` waits for an ACK of everything
/// sent and would hold both sides. It runs only when idle, so that a tail left by the transport with a full window
/// does not sit until the next datagram (as in `pump::next_chunk`).
async fn serve<R, W>(
    rd: &mut R,
    wr: &mut W,
    socket: &UdpSocket,
    local: &(String, u16),
    logger: &Logger,
) where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut decoder = FrameDecoder::default();
    let (mut chunk, mut dgram) = (vec![0u8; 16 * 1024], vec![0u8; RECV_BUF]);
    let mut unflushed = false;
    loop {
        let event = tokio::select! {
            biased;
            event = next_event(rd, socket, &mut chunk, &mut dgram, logger) => event,
            flushed = wr.flush(), if unflushed => {
                unflushed = false;
                let Ok(()) = flushed else { return };
                continue;
            }
        };
        let step = match event {
            Event::Stream(Ok(n)) if n > 0 => {
                from_stream(socket, local, logger, &mut decoder, &chunk[..n]).await
            }
            Event::Socket(Ok(n)) => {
                unflushed = true;
                to_stream(wr, logger, &dgram[..n]).await
            }
            Event::Socket(Err(e)) => {
                logger.error(&format!("Local UDP socket error: {e}"));
                return;
            }
            Event::Stream(_) => return,
        };
        if step.is_err() {
            return;
        }
    }
}

async fn from_stream(
    socket: &UdpSocket,
    local: &(String, u16),
    logger: &Logger,
    decoder: &mut FrameDecoder,
    chunk: &[u8],
) -> io::Result<()> {
    logger.debug(&format!("Data from remote to local: {} bytes", chunk.len()));
    for body in decoder.push(chunk) {
        if let Err(e) = socket.send_to(&body, (local.0.as_str(), local.1)).await {
            logger.error(&format!("Send error to local: {e}"));
        }
    }
    Ok(())
}

async fn to_stream<S: AsyncWrite + Unpin>(
    remote: &mut S,
    logger: &Logger,
    datagram: &[u8],
) -> io::Result<()> {
    logger.debug(&format!(
        "Data from local to remote: {} bytes",
        datagram.len()
    ));
    remote.write_all(&encode_frame(datagram)).await
}

#[cfg(test)]
#[path = "udp_server_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "udp_server_close_tests.rs"]
mod close_tests;

#[cfg(test)]
#[path = "udp_flush_tests.rs"]
mod flush_tests;
