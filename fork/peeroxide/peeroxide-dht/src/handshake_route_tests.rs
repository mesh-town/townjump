//! Table of the JS reference behaviour: who receives the relayed answer.

use super::*;
use crate::hyperdht_messages::MODE_FROM_CLIENT;

fn peer(port: u16) -> Ipv4Peer {
    Ipv4Peer {
        host: "127.0.0.1".into(),
        port,
    }
}

#[test]
fn from_relay_answer_goes_back_to_the_forwarder() {
    let to = relay_target(MODE_FROM_RELAY, &peer(52423), None);
    assert_eq!(to, Some(peer(52423)));
}

#[test]
fn from_second_relay_answer_goes_to_the_first_relay_not_the_forwarder() {
    // JS reference: R2 (52925) forwards, `relayAddress` names H1 (58959); the server answers H1.
    let to = relay_target(MODE_FROM_SECOND_RELAY, &peer(52925), Some(&peer(58959)));
    assert_eq!(to, Some(peer(58959)));
}

#[test]
fn from_second_relay_without_relay_address_is_silence() {
    assert_eq!(
        relay_target(MODE_FROM_SECOND_RELAY, &peer(52925), None),
        None
    );
}

#[test]
fn direct_request_is_not_relayed() {
    assert_eq!(
        relay_target(MODE_FROM_CLIENT, &peer(1), Some(&peer(2))),
        None
    );
}

#[tokio::test]
async fn reply_via_is_empty_until_the_server_sets_it_and_is_taken_once() {
    let runtime = libudx::UdxRuntime::new().expect("runtime");
    let socket = runtime.create_socket().await.expect("socket");
    let via = ReplyVia::default();
    assert!(via.take().is_none());
    via.set(socket);
    let shared = via.clone();
    assert!(shared.take().is_some());
    assert!(via.take().is_none());
}
