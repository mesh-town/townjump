// JS: dht-rpc/lib/query.js
//! Test for `Query::on_response`: the "slow" mark and `seen` are keyed by the address the
//! request was sent to (`sent_to`), not by the response address (`from`). Related: `query_slow_tests.rs`.

use super::tests::{make_query_with_k, peer};
use super::*;

mod slow_released {
    use super::*;

    #[test]
    fn slow_released_by_reply_from_another_address_of_that_node() {
        // given: a request to A1 became slow
        let (mut q, _rx) = make_query_with_k([0u8; 32], K);
        q.add_nodes(&[("127.0.0.1".to_string(), 5000)]);
        let a1 = peer("127.0.0.1", 5000);
        let sent = q.poll_requests();
        assert_eq!(sent.len(), 1);
        q.on_cycle(&a1);
        assert_eq!(q.slow(), 1);
        // when: a response came from another address A2 of the same node
        let reply = IoResponseData {
            from: peer("127.0.0.1", 5001),
            sent_to: a1,
            to: peer("127.0.0.1", 1),
            from_id: None,
            token: None,
            closer_nodes: vec![],
            error: 0,
            value: None,
            rtt: Duration::from_millis(5),
        };
        q.on_response(reply);
        // then: the mark is cleared, A1 is marked as replied
        assert_eq!(q.slow(), 0);
        assert!(matches!(
            q.seen.get("127.0.0.1:5000"),
            Some(SeenState::Done)
        ));
        assert!(!q.seen.contains_key("127.0.0.1:5001"));
    }
}
