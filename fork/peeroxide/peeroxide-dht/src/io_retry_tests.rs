// JS: dht-rpc/lib/io.js
//! Tests for request retransmissions in IO: `RetryEvent` on intermediate expiries, `TimeoutEvent` on
//! the final one and `node_id` in it (created via `create_request` and `send_request_via_socket`),
//! several entries in one call. Related: `io_tests.rs` (window, tid, tokens).

use std::sync::{Arc, Mutex};

use libudx::UdxRuntime;
use tokio::time::Instant;

use super::*;
use crate::messages::Ipv4Peer;
use crate::peer::NodeId;
use crate::routing_table::RoutingTable;

pub(super) const NODE_ID: NodeId = [7u8; 32];
pub(super) const NODE_ID_A: NodeId = [0xA; 32];
pub(super) const NODE_ID_C: NodeId = [0xC; 32];
pub(super) const DEAD_PORT: u16 = 9;

mod request_with_node_id {
    use super::*;

    #[tokio::test]
    async fn first_three_expiries_give_one_retry_each_and_no_timeout() {
        // given: a request with node_id (DEFAULT_RETRIES = 3)
        let mut f = setup(Some(NODE_ID)).await;
        for n in 1..=3 {
            // when: the timer expires
            let (retries, timeouts) = f.when_expired();
            // then: one retransmission of this request, no final event
            assert_eq!(retries.len(), 1, "expiry {n}");
            assert_eq!(retries[0].tid, f.tid, "expiry {n}");
            assert_eq!(retries[0].to, dead_peer(), "expiry {n}");
            assert!(timeouts.is_empty(), "expiry {n}: no final event yet");
        }
    }

    #[tokio::test]
    async fn fourth_expiry_gives_final_timeout_with_node_id() {
        // given: three retransmissions behind
        let mut f = setup(Some(NODE_ID)).await;
        for _ in 0..3 {
            f.when_expired();
        }
        // when: the fourth timer expires
        let (retries, timeouts) = f.when_expired();
        // then: no retransmissions, one final event with the node id, entry removed
        assert!(retries.is_empty());
        assert_eq!(timeouts.len(), 1);
        assert_eq!(timeouts[0].tid, f.tid);
        assert_eq!(timeouts[0].node_id, Some(NODE_ID));
        assert!(f.io.inflight.is_empty(), "entry removed from inflight");
    }
}

mod request_without_node_id {
    use super::*;

    #[tokio::test]
    async fn final_timeout_carries_none() {
        // given: a request without node_id
        let mut f = setup(None).await;
        // when: four expiries
        let events: Vec<_> = (0..4).map(|_| f.when_expired()).collect();
        // then: the final event exists and has no id
        let (_, timeouts) = &events[3];
        assert_eq!(timeouts.len(), 1);
        assert_eq!(timeouts[0].node_id, None);
    }
}

mod request_not_yet_sent {
    use super::*;

    #[tokio::test]
    async fn expiry_sends_it_without_retry_event() {
        // given: the request is still in the congestion queue (sent == 0)
        let mut f = setup(Some(NODE_ID)).await;
        f.io.inflight[0].sent = 0;
        // when: the timer expires
        let (retries, timeouts) = f.when_expired();
        // then: sent for the first time, no event
        assert!(
            retries.is_empty(),
            "unsent request is not slow: {retries:?}"
        );
        assert!(timeouts.is_empty());
        assert_eq!(f.io.inflight[0].sent, 1);
    }
}

mod check_timeouts_wrapper {
    use super::*;

    #[tokio::test]
    async fn hides_retries_and_returns_only_the_final_timeout() {
        // given: a request with node_id
        let mut f = setup(Some(NODE_ID)).await;
        // when: expiries 1–3
        for n in 1..=3 {
            f.expire(f.tid);
            // then: no events
            assert!(f.io.check_timeouts().is_empty(), "expiry {n}");
        }
        assert_eq!(f.io.inflight[0].sent, 4, "retransmissions done silently");
        // when: expiry 4
        f.expire(f.tid);
        let timeouts = f.io.check_timeouts();
        // then: one final event
        assert_eq!(timeouts.len(), 1);
        assert_eq!(timeouts[0].node_id, Some(NODE_ID));
    }
}

mod request_via_socket {
    use super::*;

    #[tokio::test]
    async fn first_expiry_is_the_final_timeout_with_node_id() {
        // given: the request went through a foreign socket (no retransmissions) and carries node_id
        let mut f = setup_empty().await;
        let socket = f.io.server_socket();
        let params = RequestParams {
            to: dead_peer(),
            internal: true,
            command: 1,
            node_id: Some(NODE_ID),
            ..RequestParams::default()
        };
        let tid = f.io.send_request_via_socket(params, &socket).expect("send");
        // when: the first timer expires
        f.expire(tid);
        let (retries, timeouts) = f.io.check_timeouts_and_retries();
        // then: final event at once with the node id, no retransmission, entry removed
        assert!(retries.is_empty(), "{retries:?}");
        assert_eq!(timeouts.len(), 1);
        assert_eq!(timeouts[0].tid, tid);
        assert_eq!(timeouts[0].node_id, Some(NODE_ID));
        assert!(f.io.inflight.is_empty());
    }
}

mod several_entries_in_one_call {
    use super::*;

    #[tokio::test]
    async fn retry_and_final_of_different_requests_keep_their_own_data() {
        // given: A used up its retransmissions and is first, B is at the first expiry; both overdue
        let mut f = setup_empty().await;
        let a = f.add_request(Some(NODE_ID_A), 11);
        let b = f.add_request(None, 12);
        f.set_sent(a, 4);
        f.expire(a);
        f.expire(b);
        // when: one timer check call
        let (retries, timeouts) = f.io.check_timeouts_and_retries();
        // then: retransmission for B to its address; final for A with its id; B remains in inflight
        assert_eq!(retries.len(), 1, "{retries:?}");
        assert_eq!(retries[0].tid, b);
        assert_eq!(retries[0].to, peer(12));
        assert_eq!(timeouts.len(), 1, "{timeouts:?}");
        assert_eq!(timeouts[0].tid, a);
        assert_eq!(timeouts[0].to, peer(11));
        assert_eq!(timeouts[0].node_id, Some(NODE_ID_A));
        assert_eq!(f.inflight_tids(), [b]);
    }

    #[tokio::test]
    async fn two_finals_around_a_survivor_remove_exactly_the_finals() {
        // given: inflight order A, B, D, C; A and C (last) used up retransmissions, B at the
        // first expiry, D not overdue yet
        let mut f = setup_empty().await;
        let a = f.add_request(Some(NODE_ID_A), 11);
        let b = f.add_request(None, 12);
        let d = f.add_request(None, 14);
        let c = f.add_request(Some(NODE_ID_C), 13);
        f.set_sent(a, 4);
        f.set_sent(c, 4);
        [a, b, c].iter().for_each(|&tid| f.expire(tid));
        // when
        let (retries, timeouts) = f.io.check_timeouts_and_retries();
        // then: finals for A and C, each with its own id, one retransmission (B), D untouched
        let mut finals: Vec<_> = timeouts.iter().map(|t| (t.tid, t.node_id)).collect();
        finals.sort_unstable();
        let mut expected = vec![(a, Some(NODE_ID_A)), (c, Some(NODE_ID_C))];
        expected.sort_unstable();
        assert_eq!(finals, expected);
        assert_eq!(retries.iter().map(|r| r.tid).collect::<Vec<_>>(), [b]);
        let mut survivors = vec![b, d];
        survivors.sort_unstable();
        assert_eq!(f.inflight_tids(), survivors);
        assert_eq!(f.sent_of(b), 2, "B retransmitted");
        assert_eq!(f.sent_of(d), 1, "D untouched");
    }
}

// ── setup and helpers ─────────────────────────────────────────────────────────

pub(super) struct Fixture {
    pub(super) io: Io,
    /// tid of the request created in `setup` (0 in `setup_empty`).
    pub(super) tid: u16,
    /// The runtime is kept for the whole test; the field follows `io` so the sockets close first.
    _rt: UdxRuntime,
}

impl Fixture {
    /// The deadline of request `tid` has passed.
    pub(super) fn expire(&mut self, tid: u16) {
        let i = self.index_of(tid);
        self.io.inflight[i].deadline = Instant::now();
    }

    /// Request `tid` has already been sent `sent` times.
    pub(super) fn set_sent(&mut self, tid: u16, sent: u32) {
        let i = self.index_of(tid);
        self.io.inflight[i].sent = sent;
    }

    pub(super) fn sent_of(&self, tid: u16) -> u32 {
        self.io.inflight[self.index_of(tid)].sent
    }

    /// tids of the inflight entries in ascending order.
    pub(super) fn inflight_tids(&self) -> Vec<u16> {
        let mut tids: Vec<u16> = self.io.inflight.iter().map(|e| e.tid).collect();
        tids.sort_unstable();
        tids
    }

    /// The timer of the request from `setup` expired and IO checked the timeouts.
    pub(super) fn when_expired(&mut self) -> (Vec<RetryEvent>, Vec<TimeoutEvent>) {
        self.expire(self.tid);
        self.io.check_timeouts_and_retries()
    }

    /// Another request to loopback port `port` (nobody answers there); returns its tid.
    pub(super) fn add_request(&mut self, node_id: Option<NodeId>, port: u16) -> u16 {
        let params = RequestParams {
            to: peer(port),
            internal: true,
            command: 1,
            node_id,
            ..RequestParams::default()
        };
        self.io.create_request(params).expect("create_request")
    }

    pub(super) fn index_of(&self, tid: u16) -> usize {
        let found = self.io.inflight.iter().position(|e| e.tid == tid);
        found.expect("an entry with this tid exists")
    }
}

/// IO on loopback without requests.
pub(super) async fn setup_empty() -> Fixture {
    let rt = UdxRuntime::new().expect("runtime");
    let table = Arc::new(Mutex::new(RoutingTable::new([0u8; 32])));
    let config = IoConfig {
        host: "127.0.0.1".into(),
        port: 0,
        ..IoConfig::default()
    };
    let io = Io::bind(&rt, table, config).await.expect("bind");
    Fixture {
        io,
        tid: 0,
        _rt: rt,
    }
}

/// IO on loopback and one request to `dead_peer()`.
pub(super) async fn setup(node_id: Option<NodeId>) -> Fixture {
    let mut f = setup_empty().await;
    f.tid = f.add_request(node_id, DEAD_PORT);
    f
}

pub(super) fn peer(port: u16) -> Ipv4Peer {
    Ipv4Peer {
        host: "127.0.0.1".into(),
        port,
    }
}

/// A port nobody listens on.
pub(super) fn dead_peer() -> Ipv4Peer {
    peer(DEAD_PORT)
}
