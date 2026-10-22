// JS: libudx/src/udx.c
//! Common stream types: incoming packet, events, sent/received packets, write queues, aliases; related — `inner.rs`.

use super::*;

// ── Packet types ─────────────────────────────────────────────────────────────

pub(crate) struct IncomingPacket {
    pub data: Vec<u8>,
    #[allow(dead_code)]
    pub addr: SocketAddr,
    /// Z: moment the datagram was received (only under `UDX_DIAG=z`).
    pub stamp: Option<std::time::Instant>,
}

pub(crate) enum StreamEvent {
    Data(Vec<u8>),
    End,
}

// ── Reliability types ────────────────────────────────────────────────────────

/// A sent packet stored in the outgoing buffer for potential retransmission.
pub(super) struct SentPacket {
    /// Full wire-format packet bytes (header + payload).
    pub(super) packet: Vec<u8>,
    /// When this packet was (last) sent — for RTT measurement on first send.
    pub(super) time_sent: Instant,
    /// Number of times this packet has been retransmitted.
    pub(super) retransmit_count: u32,
    /// Whether this packet has been selectively acknowledged via SACK.
    pub(super) sacked: bool,
    /// Whether this packet was sent as an MTU probe (oversized via data_offset padding).
    pub(super) is_mtu_probe: bool,
    /// Whether this packet has been detected as lost (SACK gap analysis).
    pub(super) lost: bool,
    pub(super) rate_info: Option<super::congestion::rate::PacketRateInfo>,
    /// Route this packet was sent on; `None` — the current `inner.remote_addr`.
    pub(super) remote_addr: Option<SocketAddr>,
}

/// A received out-of-order packet buffered until gaps are filled.
pub(super) struct RecvdPacket {
    pub(super) payload: Vec<u8>,
    pub(super) flags: u8,
}

pub(super) struct QueuedPacket {
    pub(super) packet: Vec<u8>,
    pub(super) seq: u32,
    pub(super) remote_addr: SocketAddr,
}

/// Tracks a pending write awaiting remote acknowledgement.
pub(crate) struct PendingWrite {
    pub(crate) ack_threshold: u32,
    pub(crate) tx: Option<oneshot::Sender<Result<()>>>,
}

/// Notification from the send path to the processor task.
pub(crate) enum StreamNotify {
    DataQueued,
    Shutdown,
}

pub(super) type WriteSenders = Vec<Option<oneshot::Sender<Result<()>>>>;
pub(super) type AckedRateInfo = Vec<(u32, super::congestion::rate::PacketRateInfo)>;
pub(super) type StreamMap =
    Arc<Mutex<std::collections::HashMap<u32, mpsc::UnboundedSender<IncomingPacket>>>>;
pub(super) type FirewallHook =
    Box<dyn FnOnce(&super::socket::UdxSocket, u16, std::net::IpAddr) -> bool + Send>;

// ── WritePrep ────────────────────────────────────────────────────────────────

pub(crate) struct WritePrep {
    pub udp: Arc<tokio::net::UdpSocket>,
    pub remote_addr: SocketAddr,
    pub remote_id: u32,
    pub first_seq: u32,
    pub current_ack: u32,
    pub ack_rx: oneshot::Receiver<Result<()>>,
}
