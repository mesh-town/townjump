// JS: dht-rpc/index.js
//! Tests for request timer processing in `DhtNode`: dropping a node on the final timeout,
//! `handle_retry` (first timer of a query), `handle_expired`. Related: `node_responses.rs`.
//! The node is built by `build_node` without a loop; handlers are called directly.

use std::time::Duration;

use tokio::sync::oneshot;

use super::super::DhtError;
use super::super::node_test_support::*;
use super::super::requests::StandaloneRequest;

mod final_timeout_with_node_id {
    use super::*;

    #[tokio::test]
    async fn removes_the_node_of_a_live_query() {
        // given: B in the table, its request belongs to a live query
        let f = setup(Search::Single).await;
        // when: final timeout with node_id = B
        let f = f.when_query_timeout(Some(b_id())).await;
        // then: B dropped (and the table lock released before add_from_table in the query)
        assert!(!f.has_b(), "B must be dropped from the table");
    }

    #[tokio::test]
    async fn removes_the_node_even_if_the_query_is_already_finished() {
        // given: B in the table, the query is already finished and removed (tid cleared); the timeout event is fed by hand
        let f = setup(Search::Finished).await;
        // when
        let f = f.when_query_timeout(Some(b_id())).await;
        // then: B dropped, no panic
        assert!(!f.has_b());
    }

    #[tokio::test]
    async fn none_leaves_the_table_alone() {
        // given: same as in the first test
        let f = setup(Search::Single).await;
        // when: no node_id
        let f = f.when_query_timeout(None).await;
        // then: B stays
        assert!(f.has_b());
    }
}

mod standalone_timeout {
    use super::*;

    #[tokio::test]
    async fn check_removes_its_node() {
        // given: a Check of B (ping_some), no node_id in the event
        let mut f = setup(Search::Absent).await;
        let tid = f.register(StandaloneRequest::Check {
            node_id: b_id(),
            last_seen_tick: 0,
        });
        // when
        let f = f.when_timeout(tid, None).await;
        // then: B is removed
        assert!(!f.has_b());
    }

    #[tokio::test]
    async fn ping_gets_timeout_error_and_table_stays() {
        // given: a user Ping to B
        let mut f = setup(Search::Absent).await;
        let (tx, mut rx) = oneshot::channel();
        let tid = f.register(StandaloneRequest::Ping(tx));
        // when
        let f = f.when_timeout(tid, None).await;
        // then: response Err("timeout"), table intact
        let answer = rx.try_recv().expect("response sent");
        assert!(
            matches!(&answer, Err(DhtError::RequestFailed(m)) if m == "timeout"),
            "{answer:?}"
        );
        assert!(f.has_b());
    }
}

mod retry_event {
    use super::*;

    #[tokio::test]
    async fn gives_the_query_one_more_request_and_keeps_the_tid() {
        // given: a query with concurrency 1 and two nodes in pending; the request to B was sent
        let f = setup(Search::WithSpare).await;
        let tid = f.tid;
        assert_eq!(f.tracked(), 1);
        // when: the first timer of the request to B expired
        let f = f.when_retry(tid).await;
        // then: exactly one request went out (to A1), the tid of the request to B stays
        assert_eq!(f.tracked(), 2);
        assert!(f.node.tid_to_query.contains_key(&tid), "tid must stay");
    }

    #[tokio::test]
    async fn repeated_event_of_the_same_tid_adds_nothing() {
        // given: the retransmission is already handled (a request to A1 went out), the query still has A2
        let f = setup(Search::WithSpare).await;
        let tid = f.tid;
        let f = f.when_retry(tid).await;
        assert_eq!(f.tracked(), 2);
        // when: the same tid expires once more
        let f = f.when_retry(tid).await;
        // then: the address of B is counted once: the slot was not freed, no request to A2
        assert_eq!(f.tracked(), 2);
    }

    #[tokio::test]
    async fn finished_query_has_nothing_to_retry() {
        // given: the query is already finished and removed from active_queries (`on_query_removed` cancelled the request)
        let f = setup(Search::Finished).await;
        let tid = f.tid;
        // when: a late retransmission of this request
        let f = f.when_retry(tid).await;
        // then: no panic, no requests added, the tid is not tracked by a query
        assert_eq!(f.tracked(), 0);
        assert!(
            !f.node.tid_to_query.contains_key(&tid),
            "tid cleared together with the query"
        );
    }

    #[tokio::test]
    async fn commit_tid_is_ignored() {
        // given: the tid of the request to B is tracked by the commit phase, the query has someone to ask
        let mut f = setup(Search::WithSpare).await;
        let tid = f.tid;
        f.node.tid_to_query.insert(tid, (QID, true));
        // when
        let f = f.when_retry(tid).await;
        // then: nothing changed
        assert_eq!(f.tracked(), 1);
        assert!(f.has_b());
    }

    #[tokio::test]
    async fn unknown_tid_is_ignored() {
        // given: a query as above
        let f = setup(Search::WithSpare).await;
        // when: a retransmission of a foreign tid
        let f = f.when_retry(4242).await;
        // then: nothing changed
        assert_eq!(f.tracked(), 1);
        assert!(f.has_b());
    }
}

mod expired_timers {
    use super::*;

    #[tokio::test]
    async fn handle_expired_passes_the_first_expiry_to_the_query() {
        // given: a query with nodes A1, A2 in pending; the request to B was sent (deadline in 1 s)
        let mut f = setup(Search::WithSpare).await;
        assert_eq!(f.tracked(), 1);
        // when: a second passed and the node processed the expired timers
        tokio::time::pause();
        tokio::time::advance(Duration::from_millis(1100)).await;
        f.node.handle_expired();
        // then: the retransmission reached the query, a request to A1 went out
        assert_eq!(f.tracked(), 2);
    }

    #[tokio::test]
    async fn handle_expired_turns_the_fourth_expiry_into_a_final_timeout() {
        // given: B in the table, the only query request to it was sent (3 retransmissions)
        let mut f = setup(Search::Single).await;
        tokio::time::pause();
        // when: three timers expired, the node processed each
        for n in 1..=3 {
            f.when_second_passes().await;
            // then: a retransmission does not remove the node from the table
            assert!(f.has_b(), "expiry {n}: B still in place");
        }
        // when: the fourth expired
        f.when_second_passes().await;
        // then: the final timeout reached handle_timeout: B dropped, tid cleared
        assert!(!f.has_b(), "B must be dropped");
        assert_eq!(f.tracked(), 0, "request tid cleared");
    }
}
