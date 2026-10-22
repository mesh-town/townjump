// JS: dht-rpc/index.js Session
//! Tests for `QueryStream`: own session, cancelling the query on drop, `detach` before the query ends.

use std::future::poll_fn;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::time::timeout;

use super::super::command::DhtCommand;
use super::super::session_tests::channel_handle;
use super::super::{DhtError, UserQueryParams};
use super::QueryStream;

fn params() -> UserQueryParams {
    UserQueryParams {
        target: [3u8; 32],
        command: 2,
        value: None,
        commit: false,
        concurrency: None,
    }
}

mod starting {
    use super::*;

    #[tokio::test]
    async fn query_stream_sends_the_query_with_values_in_its_own_session() {
        // given: a session handle
        let (h, mut rx) = channel_handle();
        let s = h.session();
        // when: the stream is requested via the session handle
        let _stream = s.dht().query_stream(params());
        // then: a Query command with a listener and a session different from the handle session
        match rx.try_recv() {
            Ok(DhtCommand::Query {
                session, values, ..
            }) => {
                assert!(values.is_some());
                let id = session.expect("session");
                assert_ne!(id, s.id());
            }
            _ => panic!("expected Query"),
        }
    }

    #[tokio::test]
    async fn a_stream_on_a_closed_node_ends_with_channel_closed() {
        // given: the command receiver was dropped
        let (h, rx) = channel_handle();
        drop(rx);
        let mut stream = h.query_stream(params());
        // when: polling
        let value = poll_fn(|cx| stream.poll_value(cx)).await;
        let done = poll_fn(|cx| stream.poll_done(cx)).await;
        // then: ends without a panic
        assert!(value.is_none());
        assert!(matches!(done, Err(DhtError::ChannelClosed)));
    }
}

mod ending {
    use super::*;

    #[tokio::test]
    async fn dropping_the_stream_destroys_its_session() {
        let (h, mut rx) = channel_handle();
        let stream = h.query_stream(params());
        let id = match rx.try_recv() {
            Ok(DhtCommand::Query { session, .. }) => session.expect("session"),
            _ => panic!("expected Query"),
        };
        drop(stream);
        match rx.try_recv() {
            Ok(DhtCommand::DestroySession { id: got }) => assert_eq!(got, id),
            _ => panic!("expected DestroySession"),
        }
    }

    #[tokio::test]
    async fn a_detached_stream_keeps_its_session_until_the_query_ends() {
        // given: the stream is detached
        let (h, mut rx) = channel_handle();
        let stream = h.query_stream(params());
        let (id, reply_tx) = match rx.try_recv() {
            Ok(DhtCommand::Query {
                session, reply_tx, ..
            }) => (session.expect("session"), reply_tx),
            _ => panic!("expected Query"),
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        stream.detach(move |r| {
            assert!(matches!(r, Ok(ref v) if v.is_empty()));
            seen.fetch_add(1, Ordering::SeqCst);
        });
        // when: no result yet: the session is alive
        assert!(
            timeout(Duration::from_millis(100), rx.recv())
                .await
                .is_err()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        // then: after the result on_done is called once, then DestroySession arrives
        let _ = reply_tx.send(Ok(vec![]));
        match timeout(Duration::from_secs(1), rx.recv()).await {
            Ok(Some(DhtCommand::DestroySession { id: got })) => assert_eq!(got, id),
            _ => panic!("expected DestroySession after the result"),
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn detach_reports_a_failed_query_to_on_done() {
        let (h, mut rx) = channel_handle();
        let stream = h.query_stream(params());
        let reply_tx = match rx.try_recv() {
            Ok(DhtCommand::Query { reply_tx, .. }) => reply_tx,
            _ => panic!("expected Query"),
        };
        let (tx, got) = tokio::sync::oneshot::channel();
        stream.detach(move |r| {
            let _ = tx.send(r);
        });
        let _ = reply_tx.send(Err(DhtError::ChannelClosed));
        let r = timeout(Duration::from_secs(1), got).await.expect("on_done");
        assert!(matches!(r, Ok(Err(DhtError::ChannelClosed))));
    }

    #[tokio::test]
    async fn detach_after_the_result_does_not_call_on_done() {
        // given: the result is already received
        let (mut stream, _values, done_tx) = QueryStream::for_test();
        let _ = done_tx.send(Ok(vec![]));
        let _ = poll_fn(|cx| stream.poll_done(cx)).await;
        // when: detach
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        stream.detach(move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
        });
        // then: on_done not called
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn poll_done_after_the_result_is_channel_closed() {
        let (mut stream, _values, done_tx) = QueryStream::for_test();
        let _ = done_tx.send(Ok(vec![]));
        let first = poll_fn(|cx| stream.poll_done(cx)).await;
        let second = poll_fn(|cx| stream.poll_done(cx)).await;
        assert!(matches!(first, Ok(ref v) if v.is_empty()));
        assert!(matches!(second, Err(DhtError::ChannelClosed)));
    }
}
