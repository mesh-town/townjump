// JS: nat-sampler/index.js
//! Node-level address after a network change: `handle_io_event` feeds replies into the NAT window and
//! `RemoteAddress` reads it back. Fixture: `node_test_support`.

use super::super::command::DhtCommand;
use super::super::node_test_support::*;
use super::super::requests::StandaloneRequest;
use crate::hyperdht::client_same_host;
use crate::io::{IoEvent, ResolvedRequest};
use crate::messages::Ipv4Peer;
use tokio::sync::oneshot;

const OLD_HOST: &str = "203.0.113.1";

fn old_addr() -> Ipv4Peer {
    Ipv4Peer {
        host: OLD_HOST.into(),
        port: 5000,
    }
}

fn new_addr() -> Ipv4Peer {
    Ipv4Peer {
        host: "198.51.100.7".into(),
        port: 6000,
    }
}

fn reply(tid: u16, from_port: u16, to: &Ipv4Peer) -> IoEvent {
    IoEvent::Response {
        server_arrival: true,
        tid,
        from: spot(from_port),
        to: to.clone(),
        id: None,
        token: None,
        closer_nodes: vec![],
        error: 0,
        value: None,
        rtt: std::time::Duration::from_millis(5),
        request: ResolvedRequest {
            tid: 9999,
            to: spot(1),
            command: 1,
            internal: true,
            target: None,
        },
    }
}

fn feed(f: &mut Fixture, i: u16, to: &Ipv4Peer) {
    f.node.handle_io_event(reply(9999, 20_000 + i, to));
}

fn read(f: &mut Fixture) -> Option<Ipv4Peer> {
    let (reply_tx, mut rx) = oneshot::channel();
    f.node
        .handle_command(DhtCommand::RemoteAddress { reply_tx });
    rx.try_recv().expect("reply").0
}

#[tokio::test]
async fn remote_address_moves_to_new_host_within_window() {
    // given: a full window of replies naming A
    let mut f = setup(Search::WithSpare).await;
    for i in 0..16 {
        feed(&mut f, i, &old_addr());
    }
    assert_eq!(read(&mut f), Some(old_addr()));
    // when: replies naming B arrive one by one from new responders
    for k in 1..=13u16 {
        feed(&mut f, 100 + k, &new_addr());
        let addr = read(&mut f);
        // then: A until 3, unsettled for 4..=12, B from 13
        match k {
            1..=3 => assert_eq!(addr, Some(old_addr()), "k={k}"),
            4..=12 => assert_eq!(addr, None, "k={k}"),
            _ => assert_eq!(addr, Some(new_addr()), "k={k}"),
        }
        assert_eq!(
            client_same_host(true, addr.as_ref(), OLD_HOST),
            k <= 3,
            "k={k}"
        );
    }
}

#[tokio::test]
async fn repeat_replies_from_one_node_do_not_move_address() {
    // given: a full window of replies naming A
    let mut f = setup(Search::WithSpare).await;
    for i in 0..16 {
        feed(&mut f, i, &old_addr());
    }
    // when: one responder sends 20 replies naming B within one tick
    for _ in 0..20 {
        feed(&mut f, 500, &new_addr());
    }
    // then: only one of them counted
    assert_eq!(read(&mut f), Some(old_addr()));
    // when: the interval passes and it replies once more
    f.node.tick += 12;
    feed(&mut f, 500, &new_addr());
    // then: two B samples out of 16 are tolerated
    assert_eq!(read(&mut f), Some(old_addr()));
}

#[tokio::test]
async fn puncher_socket_replies_are_not_sampled() {
    // given: a full window of replies naming A
    let mut f = setup(Search::WithSpare).await;
    for i in 0..16 {
        feed(&mut f, i, &old_addr());
    }
    // when: 16 puncher-socket ping replies naming B arrive from new responders
    let mut receivers = Vec::new();
    for i in 0..16u16 {
        let tid = 7000 + i;
        let (tx, rx) = oneshot::channel();
        f.node
            .standalone_tids
            .insert(tid, StandaloneRequest::PingViaSocket(tx));
        f.node.handle_io_event(reply(tid, 30_000 + i, &new_addr()));
        receivers.push(rx);
    }
    // then: the node address is unchanged, the puncher still gets its sample
    assert_eq!(read(&mut f), Some(old_addr()));
    for mut rx in receivers {
        let ping = rx.try_recv().expect("reply").expect("ping response");
        assert_eq!(ping.to, Some(new_addr()));
    }
}
