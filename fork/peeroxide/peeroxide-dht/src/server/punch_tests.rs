//! Tests for [`crate::server::punch`]: the PEER_HOLEPUNCH round without a puzzle and without a cluster.

use super::*;
use crate::hyperdht_messages::{
    ERROR_ABORTED, FIREWALL_CONSISTENT, FIREWALL_UNKNOWN, MODE_FROM_RELAY,
};

const SECRET: [u8; 32] = [7; 32];

fn peer(host: &str, port: u16) -> Ipv4Peer {
    Ipv4Peer {
        host: host.into(),
        port,
    }
}

fn state(round: u64) -> PunchState {
    PunchState {
        puncher: None,
        punch_addrs: vec![peer("10.0.0.2", 4000)],
        secure: Some(SecurePayload::new(SECRET)),
        round,
        punching: false,
    }
}

fn client_payload(error: u64, round: u64, punching: bool) -> HolepunchPayload {
    HolepunchPayload {
        error,
        firewall: FIREWALL_CONSISTENT,
        round,
        connected: false,
        punching,
        addresses: Some(vec![peer("10.0.0.9", 5000)]),
        remote_address: None,
        token: Some([9; 32]),
        remote_token: None,
    }
}

fn message(client: &SecurePayload, payload: &HolepunchPayload) -> HolepunchMessage {
    HolepunchMessage {
        mode: MODE_FROM_RELAY,
        id: 0,
        payload: client.encrypt(payload).expect("client encrypts"),
        peer_address: Some(peer("203.0.113.5", 6000)),
    }
}

fn relay() -> Ipv4Peer {
    peer("198.51.100.1", 49737)
}

fn relay_info() -> Vec<RelayInfo> {
    vec![RelayInfo {
        relay_address: relay(),
        peer_address: peer("203.0.113.5", 6000),
    }]
}

fn run(st: &mut PunchState, msg: &HolepunchMessage, from: &Ipv4Peer) -> Round {
    let client = peer("203.0.113.5", 6000);
    round(
        st,
        msg,
        from,
        &client,
        &relay_info(),
        FIREWALL_UNKNOWN,
        None,
    )
}

fn reply_of(round: Round) -> HolepunchPayload {
    let Round::Reply { bytes, .. } = round else {
        panic!("expected a reply");
    };
    SecurePayload::new(SECRET)
        .decrypt(&bytes)
        .expect("client decrypts")
}

#[test]
fn garbage_payload_is_silent() {
    let mut st = state(0);
    let mut msg = message(&SecurePayload::new(SECRET), &client_payload(0, 1, false));
    msg.payload = vec![1, 2, 3];
    assert!(matches!(run(&mut st, &msg, &relay()), Round::Silent));
}

#[test]
fn payload_of_another_session_is_silent() {
    let mut st = state(0);
    let other = SecurePayload::new([8; 32]);
    let msg = message(&other, &client_payload(0, 1, false));
    assert!(matches!(run(&mut st, &msg, &relay()), Round::Silent));
}

#[test]
fn remote_error_aborts_with_the_higher_round() {
    let mut st = state(2);
    let client = SecurePayload::new(SECRET);
    let msg = message(&client, &client_payload(ERROR_ABORTED, 5, false));
    let Round::Abort(bytes) = run(&mut st, &msg, &relay()) else {
        panic!("expected an abort");
    };
    let abort = client.decrypt(&bytes).expect("client decrypts");
    assert_eq!((abort.error, abort.round), (ERROR_ABORTED, 5));
    assert_eq!(st.round, 5);
}

#[test]
fn relay_gets_our_token_and_the_echo_of_theirs() {
    let mut st = state(0);
    let client = SecurePayload::new(SECRET);
    let msg = message(&client, &client_payload(0, 1, false));
    let reply = reply_of(run(&mut st, &msg, &relay()));
    let expected = st.secure.as_ref().map(|s| s.token("203.0.113.5"));
    assert_eq!(reply.token, expected);
    assert_eq!(reply.remote_token, Some([9; 32]));
}

#[test]
fn non_relay_sender_gets_no_token() {
    let mut st = state(0);
    let client = SecurePayload::new(SECRET);
    let msg = message(&client, &client_payload(0, 1, false));
    let reply = reply_of(run(&mut st, &msg, &peer("192.0.2.77", 1234)));
    assert_eq!(reply.token, None);
}

#[test]
fn reply_carries_our_addresses_and_never_connected() {
    let mut st = state(0);
    let client = SecurePayload::new(SECRET);
    let msg = message(&client, &client_payload(0, 1, false));
    let reply = reply_of(run(&mut st, &msg, &relay()));
    assert_eq!(reply.addresses, Some(vec![peer("10.0.0.2", 4000)]));
    assert_eq!((reply.connected, reply.remote_address), (false, None));
}

#[test]
fn round_never_goes_back() {
    let mut st = state(3);
    let client = SecurePayload::new(SECRET);
    let msg = message(&client, &client_payload(0, 1, false));
    assert_eq!(reply_of(run(&mut st, &msg, &relay())).round, 3);
    let msg = message(&client, &client_payload(0, 4, false));
    assert_eq!(reply_of(run(&mut st, &msg, &relay())).round, 4);
}

#[test]
fn punching_client_without_a_puzzle_starts_nothing() {
    let mut st = state(0);
    let client = SecurePayload::new(SECRET);
    let msg = message(&client, &client_payload(0, 1, true));
    let round = run(&mut st, &msg, &relay());
    assert!(matches!(
        round,
        Round::Reply {
            punching: false,
            ..
        }
    ));
    assert!(!st.punching);
}

fn consistent_nat(ours: Ipv4Peer) -> Nat {
    let mut nat = Nat::new(true);
    nat.firewall = FIREWALL_CONSISTENT;
    nat.addresses = Some(vec![ours]);
    nat
}

#[test]
fn consistent_nat_and_our_address_target_the_client() {
    let nat = consistent_nat(peer("203.0.113.7", 7000));
    let client = peer("203.0.113.5", 6000);
    let target = fast_ping_target(&nat, Some(&peer("203.0.113.7", 7000)), &client);
    assert_eq!(target, Some(client));
}

#[test]
fn unknown_nat_targets_nobody() {
    let mut nat = consistent_nat(peer("203.0.113.7", 7000));
    nat.firewall = FIREWALL_UNKNOWN;
    let client = peer("203.0.113.5", 6000);
    assert_eq!(
        fast_ping_target(&nat, Some(&peer("203.0.113.7", 7000)), &client),
        None
    );
}

#[test]
fn same_host_other_port_targets_nobody() {
    let nat = consistent_nat(peer("203.0.113.7", 7000));
    let client = peer("203.0.113.5", 6000);
    assert_eq!(
        fast_ping_target(&nat, Some(&peer("203.0.113.7", 7001)), &client),
        None
    );
}

#[test]
fn no_remote_address_targets_nobody() {
    let nat = consistent_nat(peer("203.0.113.7", 7000));
    let client = peer("203.0.113.5", 6000);
    assert_eq!(fast_ping_target(&nat, None, &client), None);
}

/// A CONSISTENT puzzle on loopback, plus a client socket that listens for holepunch packets.
async fn consistent_puzzle(ours: Ipv4Peer) -> (PunchState, UdxRuntime) {
    let rt = UdxRuntime::new().expect("runtime");
    let pool = SocketPool::new("127.0.0.1".into());
    let (tx, _rx) = mpsc::unbounded_channel::<HolepunchEvent>();
    let mut p = Holepuncher::new(&pool, &rt, true, false, FIREWALL_CONSISTENT, tx)
        .await
        .expect("puzzle");
    p.nat.firewall = FIREWALL_CONSISTENT;
    p.nat.addresses = Some(vec![ours]);
    let mut st = PunchState::new(p, vec![peer("10.0.0.2", 4000)]);
    st.arm(SECRET);
    (st, rt)
}

async fn ping_events(
    remote_address: Option<Ipv4Peer>,
    wait: std::time::Duration,
) -> (Option<crate::socket_pool::HolepunchEvent>, u16) {
    let ours = peer("203.0.113.7", 7000);
    let (mut st, rt) = consistent_puzzle(ours.clone()).await;
    let puzzle_port = st
        .puncher
        .as_ref()
        .and_then(|p| p.primary_socket())
        .expect("primary")
        .socket
        .local_addr()
        .await
        .expect("addr")
        .port();

    let pool = SocketPool::new("127.0.0.1".into());
    let mut client_ref = pool.acquire(&rt).await.expect("client socket");
    let mut rx = client_ref.take_holepunch_rx().expect("rx");
    let client_addr = client_ref.socket.local_addr().await.expect("client addr");
    let client_peer = peer("127.0.0.1", client_addr.port());

    let client = SecurePayload::new(SECRET);
    let mut payload = client_payload(0, 1, false);
    payload.remote_address = remote_address;
    let msg = message(&client, &payload);
    round(
        &mut st,
        &msg,
        &relay(),
        &client_peer,
        &relay_info(),
        FIREWALL_CONSISTENT,
        None,
    );
    let ev = tokio::time::timeout(wait, rx.recv()).await.ok().flatten();
    st.destroy();
    (ev, puzzle_port)
}

#[tokio::test]
async fn consistent_puzzle_pings_the_client_from_its_socket() {
    let (ev, puzzle_port) = ping_events(
        Some(peer("203.0.113.7", 7000)),
        std::time::Duration::from_secs(2),
    )
    .await;
    let ev = ev.expect("ping within 2s");
    assert_eq!(ev.addr.port(), puzzle_port);
}

#[tokio::test]
async fn consistent_puzzle_without_remote_address_sends_no_ping() {
    let (ev, _) = ping_events(None, std::time::Duration::from_millis(300)).await;
    assert!(ev.is_none(), "no remote_address, no ping");
}
