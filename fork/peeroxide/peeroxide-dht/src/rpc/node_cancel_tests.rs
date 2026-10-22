// JS: dht-rpc/index.js, dht-rpc/lib/io.js
//! Tests for cancelling requests by owner in `DhtNode`: the `Owner::Query` mark on query requests,
//! cancelling requests together with the query, and sessions (`Owner::Session`): the mark, `destroy_session`,
//! requests after it. Fixture: `node_test_support`.

use super::super::command::DhtCommand;
use super::super::node_test_support::*;
use super::super::types::{DhtError, ResponseData, UserRequestParams};
use crate::io::{Owner, RequestParams};
use crate::query::{QueryReply, QueryRequest};
use tokio::sync::oneshot;

mod query_owner {
    use super::*;

    #[tokio::test]
    async fn search_requests_carry_the_query_owner() {
        // given: a query with one request to B
        let f = setup(Search::Single).await;
        // then: the request belongs to the query
        assert_eq!(f.node.io.inflight_of(Owner::Query(QID)), vec![f.tid]);
    }

    #[tokio::test]
    async fn commit_and_down_hint_carry_the_query_owner() {
        // given: a query with a request to B
        let mut f = setup(Search::Single).await;
        let before = f.node.io.inflight_of(Owner::Query(QID)).len();
        // when: the query sends commit and DOWN_HINT
        let commit = RequestParams {
            to: spot(PORT_B),
            internal: true,
            command: 1,
            ..RequestParams::default()
        };
        f.node.dispatch_query_requests(
            QID,
            vec![
                QueryRequest::Commit(commit.clone()),
                QueryRequest::DownHint(commit),
            ],
        );
        // then: both requests are marked too
        assert_eq!(f.node.io.inflight_of(Owner::Query(QID)).len(), before + 2);
    }
}

mod query_removal {
    use super::*;

    #[tokio::test]
    async fn removed_query_leaves_no_inflight_and_no_tracked_tid() {
        // given: a query with spare nodes, a request to B in flight
        let mut f = setup(Search::WithSpare).await;
        tokio::time::pause();
        let stats = f.node.io.stats.clone();
        // when: the query is removed from the active set and cancelled
        f.node.active_queries.remove(&QID);
        f.node.on_query_removed(QID);
        for _ in 0..4 {
            f.when_second_passes().await;
        }
        // then: no requests or tids, no retransmissions or timeouts added, B stays in the table
        assert_eq!(f.tracked(), 0);
        assert!(f.node.io.inflight_of(Owner::Query(QID)).is_empty());
        assert_eq!(f.node.io.stats.retries, stats.retries);
        assert_eq!(f.node.io.stats.timeouts, stats.timeouts);
        assert!(f.has_b());
    }
}

mod session_destroy {
    use super::*;

    type RequestRx = oneshot::Receiver<Result<ResponseData, DhtError>>;

    fn request(f: &mut Fixture, session: Option<u64>, port: u16) -> RequestRx {
        let (tx, rx) = oneshot::channel();
        f.node.handle_command(DhtCommand::Request {
            params: UserRequestParams::default(),
            host: HOST.into(),
            port,
            session,
            via: None,
            reply_tx: tx,
        });
        rx
    }

    fn find_node(
        f: &mut Fixture,
        session: u64,
    ) -> oneshot::Receiver<Result<Vec<QueryReply>, DhtError>> {
        let (tx, rx) = oneshot::channel();
        f.node.handle_command(DhtCommand::FindNode {
            target: [0u8; 32],
            session: Some(session),
            reply_tx: tx,
        });
        rx
    }

    #[tokio::test]
    async fn session_destroy_fails_its_standalone_caller() {
        // given: a request of session 7 in flight
        let mut f = setup(Search::Absent).await;
        let mut rx = request(&mut f, Some(7), 9100);
        assert_eq!(f.node.standalone_tids.len(), 1);
        // when: the session is destroyed
        f.node.destroy_session(7);
        // then: the caller got destroyed, no entry
        match rx.try_recv() {
            Ok(Err(DhtError::RequestFailed(m))) => assert_eq!(m, "destroyed"),
            other => panic!("expected RequestFailed(destroyed), got {other:?}"),
        }
        assert!(f.node.standalone_tids.is_empty());
    }

    #[tokio::test]
    async fn session_destroy_aborts_its_active_query() {
        // given: find_node of session 7, B in the table
        let mut f = setup(Search::Absent).await;
        let mut rx = find_node(&mut f, 7);
        assert_eq!(f.node.active_queries.len(), 1);
        assert_eq!(f.tracked(), 1);
        // when: the session is destroyed
        f.node.destroy_session(7);
        // then: no query, tids not tracked, the caller got an error
        assert!(f.node.active_queries.is_empty());
        assert_eq!(f.tracked(), 0);
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;
        assert!(matches!(rx.try_recv(), Ok(Err(_))));
    }

    #[tokio::test]
    async fn session_destroy_leaves_other_sessions_and_node_requests_alone() {
        // given: requests of sessions 7 and 8, a query request, an unmarked request
        let mut f = setup(Search::Single).await;
        let _r7 = request(&mut f, Some(7), 9100);
        let _r8 = request(&mut f, Some(8), 9101);
        let _none = f
            .node
            .io
            .create_request(RequestParams {
                to: spot(9102),
                internal: true,
                command: 1,
                ..RequestParams::default()
            })
            .expect("tid");
        let active = f.node.io.stats.active;
        // when: session 7 is destroyed
        f.node.destroy_session(7);
        // then: session 8, the query and the unowned one are alive (exactly one request cancelled)
        assert_eq!(f.node.io.stats.active, active - 1);
        assert_eq!(f.node.io.inflight_of(Owner::Session(7)), Vec::<u16>::new());
        assert_eq!(f.node.io.inflight_of(Owner::Session(8)).len(), 1);
        assert_eq!(f.node.io.inflight_of(Owner::Query(QID)), vec![f.tid]);
    }

    #[tokio::test]
    async fn session_destroy_never_removes_table_nodes() {
        // given: find_node of session 7 to B
        let mut f = setup(Search::Absent).await;
        tokio::time::pause();
        let _rx = find_node(&mut f, 7);
        let timeouts = f.node.io.stats.timeouts;
        // when: the session is destroyed, time passes
        f.node.destroy_session(7);
        for _ in 0..4 {
            f.when_second_passes().await;
        }
        // then: B stays in the table, no timeouts
        assert!(f.has_b());
        assert_eq!(f.node.io.stats.timeouts, timeouts);
    }

    #[tokio::test]
    async fn unknown_session_is_a_noop() {
        // given: requests in flight
        let mut f = setup(Search::Single).await;
        let _rx = request(&mut f, Some(7), 9100);
        let before = f.node.io.stats.clone();
        // when: an unknown session is destroyed
        f.node.destroy_session(99);
        // then: everything stays in place
        assert_eq!(f.node.io.inflight_of(Owner::Session(7)).len(), 1);
        assert_eq!(f.node.io.inflight_of(Owner::Query(QID)), vec![f.tid]);
        assert_eq!(f.node.standalone_tids.len(), 1);
        assert_eq!(f.node.io.stats.cancelled, before.cancelled);
    }
}
