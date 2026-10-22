//! Unit test helpers for the stream: StreamInner without a socket and "sent" packets.
#![allow(dead_code)] // with_pending_write and with_queued are called only by some tests

use super::*;
use std::time::Duration;

pub(super) fn stream_inner() -> StreamInner {
    let (read_tx, _read_rx) = mpsc::unbounded_channel();
    StreamInner::new(read_tx, None)
}

/// Registers `count` sent DATA from `first` (mod 2^32): remote_acked = first, next_seq = first+count.
/// Times are deterministic: `time_sent = base + i us`, `base = now − (count + 1) us` — all in the past, strictly ascending.
pub(super) fn with_sent(inner: &mut StreamInner, first: u32, count: u32) {
    let base = Instant::now() - Duration::from_micros(u64::from(count) + 1);
    for i in 0..count {
        let seq = first.wrapping_add(i);
        inner.register_sent(seq, build_data_packet(inner.remote_id, seq, 0, b"x"), false);
        let at = base + Duration::from_micros(u64::from(i));
        inner.outgoing.set_time_sent(Seq(seq), at);
    }
    inner.remote_acked = first;
    inner.next_seq = first.wrapping_add(count);
}

/// Another `count` DATA from `next_seq`, `time_sent = at + i us`; `next_seq` grows, `remote_acked` is untouched.
pub(super) fn send_more_at(inner: &mut StreamInner, count: u32, at: Instant) {
    for i in 0..count {
        let seq = inner.next_seq;
        inner.register_sent(seq, build_data_packet(inner.remote_id, seq, 0, b"m"), false);
        let sent = at + Duration::from_micros(u64::from(i));
        inner.outgoing.set_time_sent(Seq(seq), sent);
        inner.next_seq = seq.wrapping_add(1);
    }
}

/// Fast retransmit of all the lost: copies in ascending number. Tests call the helper, not the retransmission path.
pub(super) fn resend_lost_all(inner: &mut StreamInner) -> Vec<Vec<u8>> {
    let now = Instant::now();
    std::iter::from_fn(|| {
        let seq = inner.outgoing.first_lost(None)?;
        inner.resend_seq(seq, now)
    })
    .collect()
}

/// RTO retransmission without gates: all without SACK are marked lost (as an RTO of long-gone ones), then the lost are retransmitted.
pub(super) fn rto_resend(inner: &mut StreamInner) -> Vec<Vec<u8>> {
    let far = Instant::now() + Duration::from_secs(3600);
    inner.outgoing.mark_rto_lost(Duration::ZERO, far);
    resend_lost_all(inner)
}

/// A pending write with threshold `ack_threshold`; the receiver is for checking the result.
pub(super) fn with_pending_write(
    inner: &mut StreamInner,
    ack_threshold: u32,
) -> oneshot::Receiver<Result<()>> {
    let (tx, rx) = oneshot::channel();
    inner.pending_writes.push(PendingWrite {
        ack_threshold,
        tx: Some(tx),
    });
    rx
}

/// Puts an unsent DATA with `seq` into send_queue (for ZWP and next_send_seq).
pub(super) fn with_queued(inner: &mut StreamInner, seq: u32) {
    let packet = build_data_packet(inner.remote_id, seq, 0, b"q");
    inner.send_queue.push_back(QueuedPacket {
        packet,
        seq,
        remote_addr: "127.0.0.1:9".parse().unwrap(),
    });
}

// ── Real sockets: a stream on UDX sockets against raw UDP peers (they see bytes and the source address) ──

use crate::native::runtime::UdxRuntime;
use crate::native::socket::UdxSocket;

pub(super) const LOCAL_ID: u32 = 1;
pub(super) const REMOTE_ID: u32 = 100;

pub(super) async fn bound_socket(runtime: &UdxRuntime) -> (UdxSocket, SocketAddr) {
    let socket = runtime.create_socket().await.expect("create_socket");
    socket
        .bind("127.0.0.1:0".parse().unwrap())
        .await
        .expect("bind");
    let addr = socket.local_addr().await.expect("local_addr");
    (socket, addr)
}

pub(super) async fn raw_peer() -> (tokio::net::UdpSocket, SocketAddr) {
    let raw = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("raw bind");
    let addr = raw.local_addr().expect("raw addr");
    (raw, addr)
}

/// A stream `LOCAL_ID` connected through `s1` to the raw peer `r1`; `s2` and `r2` are the second route.
pub(super) struct Rig {
    pub(super) runtime: UdxRuntime,
    pub(super) stream: UdxStream,
    pub(super) s1: UdxSocket,
    pub(super) s2: UdxSocket,
    pub(super) addr1: SocketAddr,
    pub(super) addr2: SocketAddr,
    pub(super) r1: tokio::net::UdpSocket,
    pub(super) r2: tokio::net::UdpSocket,
    pub(super) raddr1: SocketAddr,
    pub(super) raddr2: SocketAddr,
}

pub(super) async fn connected_pair_two_sockets() -> Rig {
    let runtime = UdxRuntime::new().expect("runtime");
    let (s1, addr1) = bound_socket(&runtime).await;
    let (s2, addr2) = bound_socket(&runtime).await;
    let (r1, raddr1) = raw_peer().await;
    let (r2, raddr2) = raw_peer().await;
    let stream = runtime.create_stream(LOCAL_ID).await.expect("stream");
    stream
        .connect(&s1, REMOTE_ID, raddr1)
        .await
        .expect("connect");
    Rig {
        runtime,
        stream,
        s1,
        s2,
        addr1,
        addr2,
        r1,
        r2,
        raddr1,
        raddr2,
    }
}

/// One datagram of the raw peer within a second: bytes and the source address.
pub(super) async fn recv_raw(raw: &tokio::net::UdpSocket) -> Option<(Vec<u8>, SocketAddr)> {
    let mut buf = vec![0u8; 2048];
    let got = tokio::time::timeout(Duration::from_secs(1), raw.recv_from(&mut buf)).await;
    let (n, src) = got.ok()?.ok()?;
    buf.truncate(n);
    Some((buf, src))
}

/// A UDX packet for the stream `LOCAL_ID` from the peer: `flags`, `seq`, cumulative `ack`, payload.
pub(super) fn peer_packet(flags: u8, seq: u32, ack: u32, payload: &[u8]) -> Vec<u8> {
    let header = Header {
        type_flags: flags,
        data_offset: 0,
        remote_id: LOCAL_ID,
        recv_window: DEFAULT_RWND,
        seq,
        ack,
    };
    let mut packet = header.encode().to_vec();
    packet.extend_from_slice(payload);
    packet
}

/// Writes `data` and answers it from `raw` with an ACK to the packet's source; returns the packet and its source.
pub(super) async fn write_acked(
    stream: &UdxStream,
    raw: &tokio::net::UdpSocket,
    data: &[u8],
) -> (Vec<u8>, SocketAddr) {
    let answer = async {
        let (packet, src) = recv_raw(raw).await.expect("no packet on the raw peer");
        let seq = u32::from_le_bytes(packet[12..16].try_into().unwrap());
        raw.send_to(&peer_packet(0, 0, seq.wrapping_add(1), b""), src)
            .await
            .unwrap();
        (packet, src)
    };
    let (written, got) = tokio::join!(stream.write(data), answer);
    written.expect("write");
    got
}

/// Polls `fut` once with a no-op waker.
pub(super) fn poll_once<F: std::future::Future + Unpin>(fut: &mut F) -> std::task::Poll<F::Output> {
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    std::pin::Pin::new(fut).poll(&mut cx)
}

pub(super) fn table_len(socket: &UdxSocket) -> usize {
    socket
        .streams_ref()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .len()
}
