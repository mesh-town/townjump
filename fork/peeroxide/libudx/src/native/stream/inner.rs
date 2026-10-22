// JS: libudx/src/udx.c
//! `StreamInner` stream state and its base methods; see also `mtu.rs`, `outgoing.rs`, `loss.rs`, `receive.rs`.

use super::path::PathHook;
use super::*;

// ── StreamInner ──────────────────────────────────────────────────────────────

pub(crate) struct StreamInner {
    // ── Connection identity ──
    pub(crate) remote_id: u32,
    pub(crate) remote_addr: Option<SocketAddr>,
    pub(crate) connected: bool,
    /// True once `prepare_end()` has been called (FIN queued).
    pub(super) ended: bool,

    // ── Send state ──
    pub(crate) next_seq: u32,
    pub(crate) pending_writes: Vec<PendingWrite>,
    /// Sent packets awaiting ACK, keyed by sequence number.
    pub(super) outgoing: SentMap,
    /// Sequence number of our END (`reserve_end_seq`); `None` means END was not ordered.
    pub(super) end_seq: Option<u32>,
    /// The peer has already confirmed our END (the reply ACK went out once).
    pub(super) end_acked: bool,

    // ── Receive state ──
    pub(crate) next_remote_seq: u32,
    /// Highest cumulative ACK received from remote.
    pub(super) remote_acked: u32,
    /// Out-of-order receive buffer, keyed by sequence number.
    pub(super) recv_buf: BTreeMap<Seq, RecvdPacket>,
    /// The peer's END was delivered to the reader (JS `UDX_STREAM_ENDED_REMOTE`).
    pub(super) remote_ended: bool,

    // ── RTT estimation (Jacobson/Karels, matching C libudx constants) ──
    pub(super) srtt: u32,
    pub(super) rttvar: u32,
    pub(super) rto: u32,
    pub(super) rto_timeouts: u32,
    /// Diagnostics: RTO firings over the stream's lifetime (for `stats()` only).
    pub(super) rto_fires: u32,
    /// Z diagnostics: loss-marking counters and the `ZP` line clock.
    pub(super) zp: diag::Zp,

    // ── Flow control ──
    /// Remote's last advertised receive window.
    pub(super) send_rwnd: u32,
    /// Zero-window probe (ZWP, D4) is active: window 0 while the queue has data.
    pub(super) zwp_active: bool,

    // ── MTU probing state ──
    pub(crate) mtu_state: MtuState,
    pub(crate) mtu: usize,
    pub(crate) mtu_probe_size: usize,
    pub(crate) mtu_probe_count: u32,
    pub(crate) mtu_probe_wanted: bool,
    pub(crate) mtu_max: usize,

    // ── IO ──
    pub(crate) udp: Option<Arc<tokio::net::UdpSocket>>,
    pub(crate) read_tx: mpsc::UnboundedSender<StreamEvent>,
    pub(super) close_tx: Option<oneshot::Sender<()>>,
    pub(crate) notify_tx: Option<mpsc::UnboundedSender<StreamNotify>>,

    // ── Congestion control (BBR) ──
    pub(crate) congestion: super::congestion::CongestionController,
    pub(super) send_queue: SendQueue,

    // ── Relay ──
    pub(super) relay_target: Option<std::sync::Weak<Mutex<StreamInner>>>,

    // ── Firewall hook (set_firewall_hook path) ──
    /// Single-fire hook invoked on the first incoming packet when the stream
    /// starts in listening mode (via `set_firewall_hook`). On `true` the stream
    /// adopts the packet's source as `remote_addr` and transitions to connected.
    pub(super) firewall_hook: Option<FirewallHook>,
    /// Retained socket clone supplied to `set_firewall_hook`; forwarded to the
    /// hook and cleared after the hook fires.
    pub(super) firewall_socket: Option<super::socket::UdxSocket>,

    // ── Route change and closing ──
    /// A pending `change_remote`; see `upgrade.rs`.
    pub(super) change: Option<RemoteChange>,
    /// Entries in the tables of the sockets the stream moved to; the first socket's entry is the task guard's.
    pub(super) extra_tables: Vec<TableEntry>,
    /// Vets packets from a source other than `remote_addr`; see `path.rs`.
    pub(super) path_hook: Option<PathHook>,
    /// Waiters of `await_direct`: fired by the next DATA or MESSAGE from the current remote.
    pub(super) direct_waiters: Vec<oneshot::Sender<()>>,
    /// `true` once the stream is closed for good (`mark_closed`).
    pub(super) closed: watch::Sender<bool>,

    // ── Write budget ──
    /// The adapter writer is waiting for write headroom (woken in `on_ack_packet` and `terminate`).
    pub(super) writer_waker: Option<std::task::Waker>,
}

impl StreamInner {
    // ── Stream termination ──────────────────────────────────────────────────

    /// Drain all pending writes with `StreamClosed`, mark the stream
    /// disconnected, and drop `notify_tx` so future sends fail fast.
    /// Idempotent — safe to call more than once.
    pub(super) fn terminate(&mut self) {
        self.connected = false;
        self.notify_tx = None;
        self.mark_closed();

        for mut pw in self.pending_writes.drain(..) {
            if let Some(tx) = pw.tx.take() {
                let _ = tx.send(Err(UdxError::StreamClosed));
            }
        }
        // wake() under the lock is safe: tokio only queues the task (the oneshot above is also sent under the lock)
        if let Some(w) = self.writer_waker.take() {
            w.wake();
        }
    }

    // ── MTU ──────────────────────────────────────────────────────────────────

    /// IP + UDP + udx overhead. The `_` branch (IPv6 and address not yet known) mirrors the reference exactly
    /// (udx.c:86: `ss_family == AF_INET ? IPV4 : IPV6`); keep it as is.
    pub(crate) fn ip_overhead(&self) -> usize {
        match self.remote_addr {
            Some(SocketAddr::V4(_)) => IPV4_OVERHEAD,
            _ => IPV6_OVERHEAD,
        }
    }

    /// Payload of one packet; `mtu` is the IP packet size (udx.c:84-87).
    pub(crate) fn max_payload(&self) -> usize {
        self.mtu - self.ip_overhead()
    }

    pub(crate) fn inflight(&self) -> u32 {
        self.outgoing.inflight()
    }

    pub(crate) fn send_idle(&self) -> bool {
        self.outgoing.is_empty() && self.send_queue.is_empty()
    }

    /// Cross-checks the `outgoing` and `send_queue` counters against a walk; debug only.
    pub(super) fn debug_check_counters(&self) {
        self.outgoing.debug_check();
        self.send_queue.debug_check();
    }
}
