//! A raw UDP peer against a Rust stream: wire tests (JS libudx vectors, timer timings).
//!
//! Layout as in `testvectors/deltas/udx.json`: the Rust stream has id 5 and `remote_id` 7,
//! the peer sends packets to id 5 and sees replies with `remote_id` 7. Packet time is in ms from the peer's "clock".

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use libudx::{UdxRuntime, UdxSocket, UdxStream};
use tokio::net::UdpSocket;

use super::{create_bound_socket, create_runtime};

/// Rust stream number (the destination of the peer's packets).
pub const STREAM_ID: u32 = 5;
/// Peer stream number (`remote_id` in Rust's replies).
pub const PEER_ID: u32 = 7;
/// Default receive window (the fork's `DEFAULT_RWND` and the JS window in the vectors).
pub const OPEN_WINDOW: u32 = 4_194_304;

pub const FLAG_DATA: u8 = 0x01;
pub const FLAG_END: u8 = 0x02;
pub const FLAG_SACK: u8 = 0x04;
pub const FLAG_HEARTBEAT: u8 = 0x20;

/// A packet received by the peer: ms from the peer's clock and the whole bytes.
pub type Received = (u64, Vec<u8>);

/// A raw UDP socket on 127.0.0.1 talking to one Rust stream.
pub struct RawPeer {
    socket: UdpSocket,
    stream_addr: SocketAddr,
    clock: Instant,
}

/// A Rust stream connected to a raw peer. `keep_alive` keeps the runtime and socket alive.
pub struct Link {
    pub peer: RawPeer,
    pub stream: UdxStream,
    pub keep_alive: (UdxRuntime, UdxSocket),
}

impl Link {
    /// Stream `create_stream(5)`, `connect(&socket, 7, peer address)`.
    pub async fn new() -> Self {
        let link = Self::registered().await;
        link.connect_to_peer().await;
        link
    }

    /// Stream `create_stream(5)`, put into the socket table via `register`, without `connect`.
    pub async fn registered() -> Self {
        let runtime = create_runtime();
        let (socket, stream_addr) = create_bound_socket(&runtime).await;
        let udp = UdpSocket::bind("127.0.0.1:0").await.expect("bind raw peer");
        let stream = runtime
            .create_stream(STREAM_ID)
            .await
            .expect("create stream");
        stream.register(&socket).expect("register stream");
        let peer = RawPeer {
            socket: udp,
            stream_addr,
            clock: Instant::now(),
        };
        Self {
            peer,
            stream,
            keep_alive: (runtime, socket),
        }
    }

    /// `connect(&socket, 7, peer address)` on this link's socket.
    pub async fn connect_to_peer(&self) {
        self.stream
            .connect(&self.keep_alive.1, PEER_ID, self.peer.addr())
            .await
            .expect("connect stream to raw peer");
    }
}

impl RawPeer {
    /// Address of the peer's raw socket.
    pub fn addr(&self) -> SocketAddr {
        self.socket.local_addr().expect("raw peer addr")
    }

    /// Send the Rust stream a packet as is.
    pub async fn send(&self, packet: &[u8]) {
        self.socket
            .send_to(packet, self.stream_addr)
            .await
            .expect("send from raw peer");
    }

    /// Restart the time count (the zero point for `Received`).
    pub fn restart_clock(&mut self) {
        self.clock = Instant::now();
    }

    /// Ms since the start of the count.
    pub fn now_ms(&self) -> u64 {
        self.clock.elapsed().as_millis() as u64
    }

    /// The next packet that reached the peer within `timeout`; `None` — silence.
    pub async fn recv_within(&self, timeout: Duration) -> Option<Received> {
        let mut buf = [0u8; 2048];
        let read = tokio::time::timeout(timeout, self.socket.recv_from(&mut buf));
        let (len, _) = read.await.ok()?.expect("recv on raw peer");
        Some((self.now_ms(), buf[..len].to_vec()))
    }

    /// All packets that reached the peer within `duration`, in arrival order.
    pub async fn collect_for(&self, duration: Duration) -> Vec<Received> {
        let deadline = Instant::now() + duration;
        let mut packets = Vec::new();
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match self.recv_within(left).await {
                Some(packet) => packets.push(packet),
                None => break,
            }
        }
        packets
    }
}

/// UDX header, 20 bytes LE: `[ff 01 flags data_offset remote_id window seq ack]`.
pub fn header(
    flags: u8,
    data_offset: u8,
    remote_id: u32,
    window: u32,
    seq: u32,
    ack: u32,
) -> [u8; 20] {
    let mut bytes = [0u8; 20];
    bytes[0] = 0xff;
    bytes[1] = 0x01;
    bytes[2] = flags;
    bytes[3] = data_offset;
    bytes[4..8].copy_from_slice(&remote_id.to_le_bytes());
    bytes[8..12].copy_from_slice(&window.to_le_bytes());
    bytes[12..16].copy_from_slice(&seq.to_le_bytes());
    bytes[16..20].copy_from_slice(&ack.to_le_bytes());
    bytes
}

/// DATA from the peer to the Rust stream: `seq`, payload `payload`, window open.
pub fn data_packet(seq: u32, payload: &[u8]) -> Vec<u8> {
    let mut packet = header(FLAG_DATA, 0, STREAM_ID, OPEN_WINDOW, seq, 0).to_vec();
    packet.extend_from_slice(payload);
    packet
}

/// END without data from the peer, `seq` is the stream end number.
pub fn end_packet(seq: u32) -> Vec<u8> {
    header(FLAG_END, 0, STREAM_ID, OPEN_WINDOW, seq, 0).to_vec()
}

/// ACK from the peer in the JS format: `data_offset` 0, SACK blocks right after the header.
pub fn ack_packet(ack: u32, window: u32, blocks: &[(u32, u32)]) -> Vec<u8> {
    let flags = if blocks.is_empty() { 0 } else { FLAG_SACK };
    let mut packet = header(flags, 0, STREAM_ID, window, 0, ack).to_vec();
    for &(start, end) in blocks {
        packet.extend_from_slice(&start.to_le_bytes());
        packet.extend_from_slice(&end.to_le_bytes());
    }
    packet
}

/// Packet type flags (byte 2).
pub fn flags_of(packet: &[u8]) -> u8 {
    packet[2]
}

/// Packet `seq` number (bytes 12..16).
pub fn seq_of(packet: &[u8]) -> u32 {
    u32::from_le_bytes(packet[12..16].try_into().expect("packet has a header"))
}

/// Packet `ack` number (bytes 16..20).
pub fn ack_of(packet: &[u8]) -> u32 {
    u32::from_le_bytes(packet[16..20].try_into().expect("packet has a header"))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn from_hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex digit"))
        .collect()
}
