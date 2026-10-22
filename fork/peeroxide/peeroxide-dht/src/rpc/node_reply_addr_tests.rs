// JS: dht-rpc/index.js
//! Test for `handle_io_event`: the response comes from a different address than the request went to, and the "slow"
//! mark of the node in the query is still cleared (by `request.to`). Fixture: `node_test_support`.

use super::super::node_test_support::*;
use crate::io::{IoEvent, ResolvedRequest};

#[tokio::test]
async fn reply_carries_the_address_the_request_went_to() {
    // given: a query with spare nodes, the request to B became slow
    let mut f = setup(Search::WithSpare).await;
    let b = spot(PORT_B);
    let slow_before = {
        let q = f.node.active_queries.get_mut(&QID).expect("query");
        q.on_cycle(&b);
        q.slow()
    };
    assert_eq!(slow_before, 1);
    // when: the response to the request to B came from another port
    f.node.handle_io_event(IoEvent::Response {
        server_arrival: true,
        tid: f.tid,
        from: spot(PORT_B + 100),
        to: spot(1),
        id: None,
        token: None,
        closer_nodes: vec![],
        error: 0,
        value: None,
        rtt: std::time::Duration::from_millis(5),
        request: ResolvedRequest {
            tid: f.tid,
            to: b,
            command: 1,
            internal: true,
            target: None,
        },
    });
    // then: the mark of B is cleared
    let q = f
        .node
        .active_queries
        .get(&QID)
        .expect("query still running");
    assert_eq!(q.slow(), 0);
}
