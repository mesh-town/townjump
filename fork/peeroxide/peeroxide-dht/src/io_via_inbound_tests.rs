// JS: dht-rpc/lib/io.js
//! Tests for IO through the external socket E, part 2: a response via E removes the entry, an
//! incoming request via E gets all responses from E, the ordinary path is unchanged. Fixture and
//! helpers are in the parent (`io_via_tests.rs`), included from there.

use super::*;

mod response_via_e {
    use super::*;

    #[tokio::test]
    async fn resolves_the_request_by_tid_with_empty_to() {
        // given: a request via E went to R
        let mut f = setup(Opts::default()).await;
        let tid = f.when_request_via(None);
        // when: a response with the same tid and a non-empty `to` is accepted via E
        let event = f.when_response_arrives_via_e(tid, peer(4242));
        // then: Response of this tid; `to` is empty (the address R sees us at is the address of E)
        let Some(IoEvent::Response {
            tid: got,
            from,
            to,
            request,
            ..
        }) = event
        else {
            panic!("expected IoEvent::Response");
        };
        assert_eq!(got, tid);
        assert_eq!(
            to,
            Ipv4Peer::default(),
            "`to` belongs to E, it does not go into the node's NAT"
        );
        assert_eq!(from, peer(f.r_addr.port()));
        assert_eq!(request.command, COMMAND);
    }

    #[tokio::test]
    async fn removes_the_inflight_entry() {
        // given: a request via E went to R
        let mut f = setup(Opts::default()).await;
        let tid = f.when_request_via(None);
        // when: the response is accepted via E
        f.when_response_arrives_via_e(tid, peer(4242));
        // then: the entry is removed, one response counted
        assert!(f.io.inflight.is_empty(), "entry removed from inflight");
        assert_eq!((f.io.stats.active, f.io.stats.responses), (0, 1));
    }

    #[tokio::test]
    async fn received_bytes_are_counted() {
        let mut f = setup(Opts::default()).await;
        let tid = f.when_request_via(None);
        // when: the response is accepted via E
        let data = response_bytes(tid, peer(4242));
        f.arrive_via_e(f.r_addr, data.clone());
        // then: bytes_received equals the size of the received datagram
        assert_eq!(f.io.wire.snapshot().1, data.len() as u64);
    }
}

mod incoming_request_via_e {
    use super::*;

    #[tokio::test]
    async fn arrival_socket_is_e() {
        // given: IO
        let mut f = setup(Opts::default()).await;
        // when: the request is accepted via E
        let req = incoming(f.when_request_arrives_via_e(None));
        // then: a request from R, arrival socket is E
        assert_eq!(req.tid, REQUEST_TID);
        assert_eq!(req.from, peer(f.r_addr.port()));
        let arrival = f.io.arrival_socket(&req.reply_ctx);
        assert_eq!(addr_of(&arrival).await, f.e_addr);
    }

    #[tokio::test]
    async fn reply_leaves_from_e_without_node_id() {
        // given: a request accepted via E
        let mut f = setup(Opts::default()).await;
        let req = incoming(f.when_request_arrives_via_e(None));
        // when: it is answered
        f.io.send_reply(&req, 0, Some(b"pong"));
        // then: R got a response from the address of E, same tid, no node id
        let (data, from) = f.r_receives().await;
        assert_eq!(from, f.e_addr, "response did not go out from E");
        let res = decode_response(&data);
        assert_eq!(res.tid, REQUEST_TID);
        assert_eq!(res.value, Some(b"pong".to_vec()));
        assert_eq!(res.id, None, "node id is not sent via E");
    }

    #[tokio::test]
    async fn deferred_reply_leaves_from_e_without_node_id() {
        // given: a request accepted via E
        let mut f = setup(Opts::default()).await;
        let req = incoming(f.when_request_arrives_via_e(None));
        // when: it is answered later, by the saved context (the `UserRequest::reply` path)
        let ctx = req.reply_ctx.clone();
        f.io.send_reply_deferred(&req.from, ctx, req.tid, req.target, 0, Some(b"late"));
        // then: R got a response from the address of E, same tid, no node id
        let (data, from) = f.r_receives().await;
        assert_eq!(from, f.e_addr, "deferred response did not go out from E");
        let res = decode_response(&data);
        assert_eq!(res.tid, REQUEST_TID);
        assert_eq!(res.value, Some(b"late".to_vec()));
        assert_eq!(res.id, None, "node id is not sent via E");
    }
}

mod bad_token_via_e {
    use super::*;

    #[tokio::test]
    async fn is_answered_with_error_2_from_e() {
        // given: IO
        let mut f = setup(Opts::default()).await;
        // when: a request with a foreign token came via E
        let event = f.when_request_arrives_via_e(Some([0xEE; 32]));
        // then: nothing upward, R got an error 2 response from the address of E, without a node id
        assert!(
            event.is_none(),
            "request with a bad token does not go upward"
        );
        let (data, from) = f.r_receives().await;
        assert_eq!(
            from, f.e_addr,
            "\"bad token\" response did not go out from E"
        );
        let res = decode_response(&data);
        assert_eq!(res.tid, REQUEST_TID);
        assert_eq!(res.error, ERROR_INVALID_TOKEN);
        assert_eq!(res.id, None, "node id is not sent via E");
    }
}

mod ignored_arrival_via_e {
    use super::*;

    #[tokio::test]
    async fn while_suspended() {
        let mut f = setup(Opts::default()).await;
        f.io.suspend();
        assert!(f.when_request_arrives_via_e(None).is_none());
    }

    #[tokio::test]
    async fn shorter_than_two_bytes() {
        let mut f = setup(Opts::default()).await;
        assert!(f.arrive_via_e(f.r_addr, vec![0x03]).is_none());
    }

    #[tokio::test]
    async fn from_an_ipv6_address() {
        let mut f = setup(Opts::default()).await;
        let from: SocketAddr = "[::1]:5555".parse().expect("addr");
        assert!(f.arrive_via_e(from, request_bytes(None)).is_none());
    }

    #[tokio::test]
    async fn from_port_zero() {
        let mut f = setup(Opts::default()).await;
        let from: SocketAddr = "127.0.0.1:0".parse().expect("addr");
        assert!(f.arrive_via_e(from, request_bytes(None)).is_none());
    }
}

mod plain_path {
    use super::*;

    #[tokio::test]
    async fn server_socket_request_is_answered_from_it_with_node_id() {
        // given: a node with an id; the request was accepted on the server socket (not via E)
        let mut f = setup(Opts::default()).await;
        let req = incoming(f.when_request_arrives_on(SocketKind::Server));
        // when: it is answered
        f.io.send_reply(&req, 0, Some(b"pong"));
        // then: arrival socket and response sender are the IO server socket, the response has the node id
        let server = f.io.server_local_addr().await.expect("server addr");
        assert!(req.reply_ctx.via.is_none(), "no external socket");
        assert_eq!(addr_of(&f.io.arrival_socket(&req.reply_ctx)).await, server);
        let (data, from) = f.r_receives().await;
        assert_eq!(
            from, server,
            "response did not go out from the server socket"
        );
        assert_eq!(decode_response(&data).id, Some(NODE_ID));
    }

    #[tokio::test]
    async fn client_socket_request_is_answered_from_it_without_node_id() {
        // given: a request accepted on the IO client socket
        let mut f = setup(Opts::default()).await;
        let req = incoming(f.when_request_arrives_on(SocketKind::Client));
        // when: it is answered
        f.io.send_reply(&req, 0, None);
        // then: arrival socket and response sender are the IO client socket, no node id
        let client = addr_of(&f.io.client_socket).await;
        assert_eq!(addr_of(&f.io.arrival_socket(&req.reply_ctx)).await, client);
        let (data, from) = f.r_receives().await;
        assert_eq!(
            from, client,
            "response did not go out from the client socket"
        );
        assert_eq!(decode_response(&data).id, None);
    }
}
