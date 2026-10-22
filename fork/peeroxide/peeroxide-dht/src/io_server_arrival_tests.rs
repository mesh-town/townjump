// JS: dht-rpc/lib/io.js
//! `IoEvent::Response::server_arrival`: true only for a reply that came straight to the server
//! socket. Fixture and helpers are in the parent (`io_via_tests.rs`), included from there.

use super::*;

fn arrival_of(event: Option<IoEvent>) -> bool {
    match event {
        Some(IoEvent::Response { server_arrival, .. }) => server_arrival,
        _ => panic!("expected IoEvent::Response"),
    }
}

/// A plain request to R is in flight; returns its tid.
fn plain_request(f: &mut Fixture) -> u16 {
    let params = RequestParams {
        to: f.r_peer(),
        internal: true,
        command: COMMAND,
        ..RequestParams::default()
    };
    f.io.create_request(params).expect("create_request")
}

fn reply_on(f: &mut Fixture, tid: u16, kind: SocketKind) -> Option<IoEvent> {
    let datagram = Datagram {
        data: response_bytes(tid, peer(4242)),
        addr: f.r_addr,
    };
    f.io.process_datagram(datagram, kind)
}

#[tokio::test]
async fn client_socket_reply_is_not_server_arrival() {
    // given: a request in flight
    let mut f = setup(Opts::default()).await;
    let tid = plain_request(&mut f);
    // when: the reply arrives on the client socket
    let event = reply_on(&mut f, tid, SocketKind::Client);
    // then: it does not count as a server arrival
    assert!(!arrival_of(event));
}

#[tokio::test]
async fn server_socket_reply_is_server_arrival() {
    // given: a request in flight
    let mut f = setup(Opts::default()).await;
    let tid = plain_request(&mut f);
    // when: the reply arrives on the server socket
    let event = reply_on(&mut f, tid, SocketKind::Server);
    // then: it counts
    assert!(arrival_of(event));
}

#[tokio::test]
async fn reply_via_external_socket_is_not_server_arrival() {
    // given: a request via E in flight
    let mut f = setup(Opts::default()).await;
    let tid = f.when_request_via(None);
    // when: the reply is accepted via E
    let event = f.when_response_arrives_via_e(tid, peer(4242));
    // then: it does not count
    assert!(!arrival_of(event));
}
