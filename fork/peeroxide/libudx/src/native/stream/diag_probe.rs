//! The `ZP` probe (under `UDX_DIAG=z`): once a second a snapshot of the window edge and loss-marking counters, per stream.
//! Does not change sender decisions: counters accumulate by plain additions, the line is printed from `diag_step`.
use super::now_us;
use std::sync::atomic::Ordering;
use portable_atomic::AtomicU64;
use std::time::{Duration, Instant};

/// "Lost" marking counters since stream start and the print clock.
#[derive(Default)]
pub(in crate::native::stream) struct Zp {
    /// RACK marks on the ACK/SACK handling path (`acks.rs`).
    pub(in crate::native::stream) marks_ack: u64,
    /// RACK marks in the reo timer (`on_reo_fire`).
    pub(in crate::native::stream) marks_timer: u64,
    /// Marks in the RACK call that moved Open → Recovery (a subset of `marks_ack`/`marks_timer`).
    pub(in crate::native::stream) marks_enter: u64,
    /// RTO marks.
    pub(in crate::native::stream) marks_rto: u64,
    /// Over the current second: `mark_sacked` lookups and entries walked in `detect_losses_at`.
    pub(in crate::native::stream) sack_lookups: u64,
    pub(in crate::native::stream) rack_visits: u64,
    /// Since stream start: ACK packets processed by the stream (`on_ack_packet`).
    pub(in crate::native::stream) proc_n: u64,
    /// Delay ring "datagram receive → processor parsing", us (allocated once, only under `z`).
    delays: Vec<u32>,
    delays_n: u64,
    q_last: usize,
    q_max: usize,
    /// Over a second: ACK packet handling cost, us, by buckets floor(log2(us+1)); sum and maximum.
    ap: [u32; AP_BUCKETS],
    ap_sum: u64,
    ap_max: u64,
    /// Sample of srtt and cumulative ACKs over a second (`rtt_*`, `cum_n`).
    pub(in crate::native::stream) rtt: rtt::RttProbe,
    start: Option<Instant>,
    last: Option<Instant>,
}

/// Packets received by socket receive loops since process start (the difference from `proc_n` is the time spent in the kernel or queue before the processor).
static RECV_N: AtomicU64 = AtomicU64::new(0);

/// The receive loop took a datagram off the socket (call only under `UDX_DIAG=z`).
pub(in crate::native) fn count_recv() {
    RECV_N.fetch_add(1, Ordering::Relaxed);
}

/// Size of the per-second delay ring.
const RING: usize = 8192;

/// Number of logarithmic `ap` buckets (the last one accumulates everything from 2^24 us).
const AP_BUCKETS: usize = 25;

/// Sender snapshot for the `ZP` line.
pub(in crate::native::stream) struct View {
    pub(in crate::native::stream) remote_acked: u32,
    pub(in crate::native::stream) seq: u32,
    pub(in crate::native::stream) map_len: usize,
    pub(in crate::native::stream) map_min: u32,
    pub(in crate::native::stream) sacked: u32,
    pub(in crate::native::stream) lost: u32,
    pub(in crate::native::stream) inflight: u32,
    /// Left side of `can_send`: `inflight < min(cwnd, rwnd_in_packets)`.
    pub(in crate::native::stream) cwnd_cmp: u32,
    /// Right side of `can_send`: `min(cwnd, rwnd_in_packets)`.
    pub(in crate::native::stream) cwnd_lim: u32,
    pub(in crate::native::stream) rack_rtt_ms: u64,
    pub(in crate::native::stream) reo_wnd_ms: u64,
    pub(in crate::native::stream) min_rtt_ms: Option<u32>,
}

impl Zp {
    /// Time to print: the first call only starts the clock; after that at most once a second.
    pub(in crate::native::stream) fn due(&mut self, now: Instant) -> Option<u64> {
        let start = *self.start.get_or_insert(now);
        let last = *self.last.get_or_insert(now);
        if now.duration_since(last) < Duration::from_secs(1) {
            return None;
        }
        self.last = Some(now);
        Some(now.duration_since(start).as_millis() as u64)
    }

    /// A packet was received: queue delay (us) and the depth of `incoming_rx` after taking it.
    pub(in crate::native::stream) fn note_rx(&mut self, delay_us: u64, depth: usize) {
        if self.delays.len() < RING {
            self.delays.resize(RING, 0);
        }
        self.delays[(self.delays_n % RING as u64) as usize] =
            delay_us.min(u64::from(u32::MAX)) as u32;
        self.delays_n += 1;
        self.rtt.set_dq(delay_us);
        self.q_last = depth;
        self.q_max = self.q_max.max(depth);
    }

    /// An ACK packet was processed: `us` is its cost in the stream task (only under `z`, no allocations).
    pub(in crate::native::stream) fn note_ack_proc(&mut self, us: u64) {
        let k = (63 - (us + 1).leading_zeros() as usize).min(AP_BUCKETS - 1);
        self.ap[k] += 1;
        self.ap_sum += us;
        self.ap_max = self.ap_max.max(us);
    }

    /// The `ap_*` fields over a window and reset. Quantile q is the smallest bucket with cumulative share ≥ q; the printed value is
    /// the upper edge of bucket k, 2^(k+1)−1 us (a bucket edge, not an exact value).
    fn take_ap(&mut self) -> String {
        let n: u64 = self.ap.iter().map(|&c| u64::from(c)).sum();
        let q = |pct: u64| {
            let mut cum = 0;
            for (k, &c) in self.ap.iter().enumerate() {
                cum += u64::from(c);
                if cum * 100 >= pct * n {
                    return (1u64 << (k + 1)) - 1;
                }
            }
            0
        };
        let (p50, p99) = if n == 0 { (0, 0) } else { (q(50), q(99)) };
        let r = format!(
            "ap_n={n} ap_sum_us={} ap_p50_us={p50} ap_p99_us={p99} ap_max_us={}",
            self.ap_sum, self.ap_max
        );
        (self.ap, self.ap_sum, self.ap_max) = ([0; AP_BUCKETS], 0, 0);
        r
    }

    /// p50/p90/max of delays over the window (us; 0 for an empty window) and reset of the window fields.
    fn take_window(&mut self) -> (u32, u32, u32, u64, usize) {
        let n = (self.delays_n as usize).min(RING);
        let w = &mut self.delays[..n];
        w.sort_unstable();
        let at = |q: usize| {
            if n == 0 {
                0
            } else {
                w[(n * q / 100).min(n - 1)]
            }
        };
        let r = (
            at(50),
            at(90),
            at(100),
            self.delays_n,
            self.q_max.max(self.q_last),
        );
        (self.delays_n, self.q_max) = (0, self.q_last);
        r
    }

    /// The `ZP` line (`t` is ms since the stream's first step, `min_rtt=-1` means no measurements yet); window fields are reset.
    pub(in crate::native::stream) fn line(&mut self, t_ms: u64, v: &View) -> String {
        let (p50, p90, max, n, q_max) = self.take_window();
        let q_last = self.q_last;
        let recv_n = RECV_N.load(Ordering::Relaxed);
        let ap = self.take_ap();
        let rtt = self.rtt.take();
        let (lookups, visits) = (
            std::mem::take(&mut self.sack_lookups),
            std::mem::take(&mut self.rack_visits),
        );
        format!(
            "ZP t={t_ms} us={} remote_acked={} seq={} map_len={} map_min={} sacked={} lost={} inflight={} cwnd_cmp={} cwnd_lim={} rack_rtt={} reo_wnd={} min_rtt={} marks_ack={} marks_timer={} marks_enter={} marks_rto={} dq_n={n} dq_p50_us={p50} dq_p90_us={p90} dq_max_us={max} q_last={q_last} q_max={q_max} sack_lookups={lookups} rack_visits={visits} recv_n={recv_n} proc_n={} {ap} {rtt}",
            now_us(),
            v.remote_acked,
            v.seq,
            v.map_len,
            v.map_min,
            v.sacked,
            v.lost,
            v.inflight,
            v.cwnd_cmp,
            v.cwnd_lim,
            v.rack_rtt_ms,
            v.reo_wnd_ms,
            v.min_rtt_ms.map_or(-1, i64::from),
            self.marks_ack,
            self.marks_timer,
            self.marks_enter,
            self.marks_rto,
            self.proc_n
        )
    }
}

#[cfg(test)]
#[path = "diag_probe_ap_tests.rs"]
mod ap_tests;
#[path = "diag_probe_rtt.rs"]
mod rtt;
#[cfg(test)]
#[path = "diag_probe_rtt_tests.rs"]
mod rtt_tests;
#[cfg(test)]
#[path = "diag_probe_tests.rs"]
mod tests;
