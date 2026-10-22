// JS: dht-rpc/lib/io.js Request.destroy
//! Tests for `Io::cancel_owner`: cancelling requests by owner, window and `stats` accounting, no
//! retransmissions, timeout or event for a late response. Fixture: `io_retry_tests`.

use libudx::Datagram;

use super::retry_tests::{DEAD_PORT, Fixture, peer, setup_empty};
use super::*;
use crate::messages::{self, Response};

const Q1: Owner = Owner::Query(1);
const S2: Owner = Owner::Session(2);

/// A request to loopback port `port` (nobody answers there) with owner `owner`; returns its tid.
fn add_owned(f: &mut Fixture, owner: Option<Owner>, port: u16) -> u16 {
    let params = RequestParams {
        to: peer(port),
        internal: true,
        command: 1,
        ..RequestParams::default()
    };
    f.io.create_request_owned(params, owner)
        .expect("create_request_owned")
}

mod by_owner {
    use super::*;

    #[tokio::test]
    async fn removes_only_that_owners_entries() {
        // given: two Q(1) entries, one S(2), one without an owner
        let mut f = setup_empty().await;
        let a = add_owned(&mut f, Some(Q1), DEAD_PORT);
        let b = add_owned(&mut f, Some(Q1), DEAD_PORT + 1);
        let c = add_owned(&mut f, Some(S2), DEAD_PORT + 2);
        let d = add_owned(&mut f, None, DEAD_PORT + 3);
        // when
        let mut gone = f.io.cancel_owner(Q1);
        gone.sort_unstable();
        // then: both Q(1) tids are gone, the rest stay
        assert_eq!(gone, vec![a, b]);
        let mut left = vec![c, d];
        left.sort_unstable();
        assert_eq!(f.inflight_tids(), left);
    }

    #[tokio::test]
    async fn unknown_owner_is_a_noop() {
        // given: two entries
        let mut f = setup_empty().await;
        add_owned(&mut f, Some(Q1), DEAD_PORT);
        add_owned(&mut f, None, DEAD_PORT + 1);
        let before = f.io.stats.clone();
        let tids = f.inflight_tids();
        // when: cancelling an owner that does not exist
        let gone = f.io.cancel_owner(Owner::Session(9));
        // then: nothing changed
        assert!(gone.is_empty());
        assert_eq!(f.inflight_tids(), tids);
        let after = &f.io.stats;
        assert_eq!(
            (after.active, after.total, after.cancelled),
            (before.active, before.total, before.cancelled)
        );
    }
}

mod accounting {
    use super::*;

    #[tokio::test]
    async fn releases_active_and_counts_cancelled() {
        // given: three Q(1) entries
        let mut f = setup_empty().await;
        for i in 0..3 {
            add_owned(&mut f, Some(Q1), DEAD_PORT + i);
        }
        let active = f.io.stats.active;
        // when
        f.io.cancel_owner(Q1);
        // then: active −3, cancelled 3, no timeouts
        assert_eq!(f.io.stats.active, active - 3);
        assert_eq!(f.io.stats.cancelled, 3);
        assert_eq!(f.io.stats.timeouts, 0);
    }

    #[tokio::test]
    async fn releases_the_congestion_window() {
        // given: window before sends = T, two sent Q(1) entries
        let mut f = setup_empty().await;
        let total = f.io.congestion.total;
        add_owned(&mut f, Some(Q1), DEAD_PORT);
        add_owned(&mut f, Some(Q1), DEAD_PORT + 1);
        assert_eq!(
            f.io.congestion.total,
            total + 2,
            "two sends took the window"
        );
        // when
        f.io.cancel_owner(Q1);
        // then: the window is back
        assert_eq!(f.io.congestion.total, total);
    }

    #[tokio::test]
    async fn unsent_entry_leaves_the_send_queue_without_touching_the_window() {
        // given: window full, a Q(1) entry waits in `pending` (never sent)
        let mut f = setup_empty().await;
        f.io.congestion.total = 1000;
        let tid = add_owned(&mut f, Some(Q1), DEAD_PORT);
        assert_eq!(f.sent_of(tid), 0);
        assert!(f.io.pending.iter().any(|p| p.tid == tid));
        // when
        f.io.cancel_owner(Q1);
        // then: removed from the queue, window untouched
        assert!(!f.io.pending.iter().any(|p| p.tid == tid));
        assert_eq!(f.io.congestion.total, 1000);
    }
}

mod afterwards {
    use super::*;

    #[tokio::test]
    async fn cancelled_entry_gets_no_retry_and_no_timeout() {
        // given: a Q(1) entry and an entry without an owner are overdue
        let mut f = setup_empty().await;
        let owned = add_owned(&mut f, Some(Q1), DEAD_PORT);
        let free = add_owned(&mut f, None, DEAD_PORT + 1);
        f.expire(owned);
        f.expire(free);
        let retries = f.io.stats.retries;
        // when: Q(1) is cancelled, then the timers are processed
        f.io.cancel_owner(Q1);
        let (retry_events, timeouts) = f.io.check_timeouts_and_retries();
        // then: retransmission only for the entry without an owner
        let tids: Vec<u16> = retry_events.iter().map(|e| e.tid).collect();
        assert_eq!(tids, vec![free]);
        assert!(timeouts.is_empty());
        assert_eq!(f.io.stats.retries, retries + 1);
        assert_eq!(f.io.stats.timeouts, 0);
    }

    #[tokio::test]
    async fn late_reply_to_a_cancelled_tid_is_ignored() {
        // given: tid cancelled
        let mut f = setup_empty().await;
        let tid = add_owned(&mut f, Some(Q1), DEAD_PORT);
        f.io.cancel_owner(Q1);
        let before = f.io.stats.clone();
        // when: a response with this tid arrives
        let reply = Response {
            tid,
            to: peer(1),
            id: None,
            token: None,
            closer_nodes: vec![],
            error: 0,
            value: None,
        };
        let data = messages::encode_response_to_bytes(&reply).expect("encode");
        let addr = format!("127.0.0.1:{DEAD_PORT}").parse().expect("addr");
        let event =
            f.io.process_datagram(Datagram { data, addr }, SocketKind::Server);
        // then: no event, accounting unchanged
        assert!(event.is_none());
        let after = &f.io.stats;
        assert_eq!(
            (after.active, after.responses, after.cancelled),
            (before.active, before.responses, before.cancelled)
        );
    }
}
