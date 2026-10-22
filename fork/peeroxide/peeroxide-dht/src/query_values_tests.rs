// JS: dht-rpc/lib/query.js _onvisit
//! Tests for the `Query` value listener (`query_values.rs`).

use std::time::Duration;

use tokio::sync::mpsc::{self, error::TryRecvError};

use super::super::tests::{make_query_with_k, peer};
use super::super::{IoResponseData, Query, QueryResult};
use tokio::sync::oneshot;

fn response(error: u64, value: Option<Vec<u8>>) -> IoResponseData {
    let mut id = [0u8; 32];
    id[0] = 1;
    IoResponseData {
        from: peer("127.0.0.1", 4001),
        sent_to: peer("127.0.0.1", 4001),
        to: peer("127.0.0.1", 5000),
        from_id: Some(id),
        token: None,
        closer_nodes: vec![],
        error,
        value,
        rtt: Duration::from_millis(5),
    }
}

/// Query with a listener; two requests "in flight" so one response does not finish it.
fn listened() -> (
    Query,
    oneshot::Receiver<QueryResult>,
    mpsc::UnboundedReceiver<super::super::QueryReply>,
) {
    let (mut q, rx) = make_query_with_k([0u8; 32], 20);
    let (tx, values) = mpsc::unbounded_channel();
    q.values_tx = Some(tx);
    q.inflight = 2;
    (q, rx, values)
}

mod a_response {
    use super::*;

    #[test]
    fn a_reply_with_a_value_reaches_the_listener_before_the_query_ends() {
        let (mut q, mut result_rx, mut values) = listened();
        q.on_response(response(0, Some(vec![7, 7])));
        let got = values.try_recv().expect("value");
        assert_eq!(got.from, peer("127.0.0.1", 4001));
        assert_eq!(got.value, Some(vec![7, 7]));
        assert!(
            result_rx.try_recv().is_err(),
            "result must not be ready yet"
        );
    }

    #[test]
    fn a_reply_without_a_value_is_not_sent() {
        let (mut q, _rx, mut values) = listened();
        q.on_response(response(0, None));
        assert!(matches!(values.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn an_error_reply_is_not_sent() {
        let (mut q, _rx, mut values) = listened();
        q.on_response(response(1, Some(vec![1])));
        assert!(matches!(values.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn a_reply_during_commit_is_not_sent() {
        let (mut q, _rx, mut values) = listened();
        q.committing = true;
        q.on_response(response(0, Some(vec![1])));
        assert!(matches!(values.try_recv(), Err(TryRecvError::Empty)));
    }
}

mod the_listener {
    use super::*;

    #[test]
    fn finishing_the_query_closes_the_values_channel() {
        let (mut q, _rx, mut values) = listened();
        q.finish_internal();
        assert!(matches!(values.try_recv(), Err(TryRecvError::Disconnected)));
    }

    #[test]
    fn a_dropped_listener_does_not_break_the_reply() {
        let (mut q, _rx, values) = listened();
        drop(values);
        q.on_response(response(0, Some(vec![1])));
        assert_eq!(q.closest_replies.len(), 1);
    }
}
