//! Tests for the `opts.holepunch` filter in [`crate::server::punch::round`].

use std::sync::{Arc, Mutex};

use super::*;
use crate::hyperdht_messages::{FIREWALL_CONSISTENT, FIREWALL_UNKNOWN, MODE_FROM_RELAY};

const SECRET: [u8; 32] = [7; 32];

fn peer(host: &str, port: u16) -> Ipv4Peer {
    Ipv4Peer {
        host: host.into(),
        port,
    }
}

async fn puzzle_state() -> (PunchState, UdxRuntime) {
    let rt = UdxRuntime::new().expect("runtime");
    let pool = SocketPool::new("127.0.0.1".into());
    let (tx, _rx) = mpsc::unbounded_channel::<HolepunchEvent>();
    let mut p = Holepuncher::new(&pool, &rt, true, false, FIREWALL_CONSISTENT, tx)
        .await
        .expect("puzzle");
    p.nat.firewall = FIREWALL_CONSISTENT;
    let mut st = PunchState::new(p, vec![peer("10.0.0.2", 4000)]);
    st.arm(SECRET);
    (st, rt)
}

fn message(punching: bool) -> HolepunchMessage {
    let payload = HolepunchPayload {
        error: ERROR_NONE,
        firewall: FIREWALL_UNKNOWN,
        round: 1,
        connected: false,
        punching,
        addresses: Some(vec![peer("10.0.0.9", 5000)]),
        remote_address: None,
        token: Some([9; 32]),
        remote_token: None,
    };
    HolepunchMessage {
        mode: MODE_FROM_RELAY,
        id: 0,
        payload: SecurePayload::new(SECRET)
            .encrypt(&payload)
            .expect("encrypts"),
        peer_address: Some(peer("203.0.113.5", 6000)),
    }
}

fn run(st: &mut PunchState, punching: bool, filter: Option<&HolepunchFilter>) -> Round {
    let client = peer("203.0.113.5", 6000);
    let relay = RelayInfo {
        relay_address: peer("198.51.100.1", 49737),
        peer_address: client.clone(),
    };
    let from = relay.relay_address.clone();
    round(
        st,
        &message(punching),
        &from,
        &client,
        &[relay],
        FIREWALL_UNKNOWN,
        filter,
    )
}

#[tokio::test]
async fn filter_refuses_punching_round() {
    let (mut st, _rt) = puzzle_state().await;
    let filter = HolepunchFilter::new(|_| false);
    let out = run(&mut st, true, Some(&filter));
    assert!(matches!(out, Round::Abort(_)), "a vetoed punch aborts");
    assert!(st.puncher.is_some(), "the puzzle was not started");
    assert!(!st.punching);
    st.destroy();
}

#[tokio::test]
async fn filter_is_not_asked_when_the_client_does_not_punch() {
    let (mut st, _rt) = puzzle_state().await;
    let asked = Arc::new(Mutex::new(0));
    let seen = asked.clone();
    let filter = HolepunchFilter::new(move |_| {
        *seen.lock().unwrap() += 1;
        false
    });
    assert!(matches!(
        run(&mut st, false, Some(&filter)),
        Round::Reply { .. }
    ));
    assert_eq!(*asked.lock().unwrap(), 0);
    st.destroy();
}

#[tokio::test]
async fn filter_sees_firewalls_and_addresses() {
    let (mut st, _rt) = puzzle_state().await;
    let seen = Arc::new(Mutex::new(None));
    let sink = seen.clone();
    let filter = HolepunchFilter::new(move |i| {
        let got = (
            i.remote_firewall,
            i.local_firewall,
            i.remote_addresses.to_vec(),
            i.local_addresses.to_vec(),
        );
        *sink.lock().unwrap() = Some(got);
        true
    });
    let out = run(&mut st, true, Some(&filter));
    assert!(matches!(out, Round::Reply { punching: true, .. }));
    let got = seen.lock().unwrap().take().expect("filter was asked");
    assert_eq!(got.0, FIREWALL_UNKNOWN);
    assert_eq!(got.1, FIREWALL_CONSISTENT);
    assert_eq!(got.2, vec![peer("10.0.0.9", 5000)]);
    assert_eq!(got.3, vec![peer("10.0.0.2", 4000)]);
}
