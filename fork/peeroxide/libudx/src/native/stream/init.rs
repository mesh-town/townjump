//! `StreamInner` constructor: the stream's initial state (shared by UdxStream::new and tests).
use super::*;

impl StreamInner {
    /// A fresh stream: not connected, window `DEFAULT_RWND`, rto `INITIAL_RTO_MS`.
    pub(super) fn new(
        read_tx: mpsc::UnboundedSender<StreamEvent>,
        close_tx: Option<oneshot::Sender<()>>,
    ) -> Self {
        Self {
            remote_id: 0,
            remote_addr: None,
            connected: false,
            ended: false,
            next_seq: 0,
            pending_writes: Vec::new(),
            outgoing: SentMap::default(),
            end_seq: None,
            end_acked: false,
            next_remote_seq: 0,
            remote_acked: 0,
            recv_buf: BTreeMap::new(),
            remote_ended: false,
            srtt: 0,
            rttvar: 0,
            rto: INITIAL_RTO_MS,
            rto_timeouts: 0,
            rto_fires: 0,
            zp: diag::Zp::default(),
            send_rwnd: DEFAULT_RWND,
            zwp_active: false,
            mtu_state: MtuState::Base,
            mtu: MTU_BASE,
            mtu_probe_size: MTU_BASE + MTU_STEP,
            mtu_probe_count: 0,
            mtu_probe_wanted: false,
            mtu_max: MTU_MAX,
            udp: None,
            read_tx,
            close_tx,
            notify_tx: None,
            congestion: crate::native::congestion::CongestionController::new(
                MAX_PAYLOAD as u32,
                INITIAL_RTO_MS,
            ),
            send_queue: SendQueue::default(),
            relay_target: None,
            firewall_hook: None,
            firewall_socket: None,
            writer_waker: None,
            change: None,
            extra_tables: Vec::new(),
            path_hook: None,
            direct_waiters: Vec::new(),
            closed: watch::channel(false).0,
        }
    }
}
