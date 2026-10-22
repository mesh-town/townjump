//! Tests for [`crate::server::reply`].

use super::*;
use crate::hyperdht_messages::{
    ERROR_TRY_LATER, FIREWALL_CONSISTENT, MODE_FROM_CLIENT, decode_handshake_from_bytes,
    decode_holepunch_msg_from_bytes,
};

fn peer(host: &str, port: u16) -> Ipv4Peer {
    Ipv4Peer {
        host: host.into(),
        port,
    }
}

fn relay(n: u16) -> RelayInfo {
    RelayInfo {
        relay_address: peer("10.0.0.1", 1000 + n),
        peer_address: peer("203.0.113.9", 2000 + n),
    }
}

fn ours(firewall: u64, relays: Vec<RelayInfo>) -> Ours {
    Ours {
        error: ERROR_NONE,
        firewall,
        addresses4: vec![peer("192.168.1.5", 4242)],
        holepunch_id: 77,
        relays,
        udx_id: 9,
        reusable_socket: false,
        relay_through: None,
    }
}

fn remote(version: u64, udx: bool) -> NoisePayload {
    let mut payload = handshake_payload(&ours(FIREWALL_OPEN, vec![]));
    payload.version = version;
    payload.udx = udx.then_some(UdxInfo {
        version: 1,
        reusable_socket: false,
        id: 5,
        seq: 0,
    });
    payload
}

#[test]
fn from_client_gets_reply_without_peer_address() {
    let via = peer("203.0.113.9", 2000);
    assert_eq!(reply_mode(MODE_FROM_CLIENT, Some(&via)), (MODE_REPLY, None));
    assert_eq!(reply_mode(MODE_REPLY, None), (MODE_REPLY, None));
}

#[test]
fn from_relay_gets_from_server_with_peer_address() {
    let via = peer("203.0.113.9", 2000);
    for inbound in [MODE_FROM_RELAY, MODE_FROM_SECOND_RELAY] {
        let (mode, address) = reply_mode(inbound, Some(&via));
        assert_eq!((mode, address), (MODE_FROM_SERVER, Some(via.clone())));
    }
}

#[test]
fn encode_reply_follows_the_request_mode() {
    let via = peer("203.0.113.9", 2000);
    let request = HandshakeMessage {
        mode: MODE_FROM_RELAY,
        noise: vec![1, 2, 3],
        peer_address: Some(via.clone()),
        relay_address: None,
    };
    let bytes = encode_reply(&request, vec![7; 4]).unwrap();
    let decoded = decode_handshake_from_bytes(&bytes).unwrap();
    assert_eq!(decoded.mode, MODE_FROM_SERVER);
    assert_eq!(
        (decoded.noise, decoded.peer_address),
        (vec![7; 4], Some(via))
    );
    let direct = HandshakeMessage {
        mode: MODE_FROM_CLIENT,
        peer_address: None,
        ..request
    };
    let decoded = decode_handshake_from_bytes(&encode_reply(&direct, vec![7; 4]).unwrap()).unwrap();
    assert_eq!((decoded.mode, decoded.peer_address), (MODE_REPLY, None));
}

#[test]
fn version_2_is_version_mismatch() {
    assert_eq!(reply_error(&remote(2, true)), ERROR_VERSION_MISMATCH);
    assert_eq!(reply_error(&remote(2, false)), ERROR_VERSION_MISMATCH);
}

#[test]
fn missing_udx_is_aborted() {
    assert_eq!(reply_error(&remote(1, false)), ERROR_ABORTED);
    assert_eq!(reply_error(&remote(1, true)), ERROR_NONE);
}

#[test]
fn open_server_reply_has_no_holepunch() {
    let payload = handshake_payload(&ours(FIREWALL_OPEN, vec![relay(1)]));
    assert_eq!(payload.holepunch, None);
    assert_eq!(payload.firewall, FIREWALL_OPEN);
    assert_eq!(payload.addresses4, vec![peer("192.168.1.5", 4242)]);
    assert_eq!(payload.udx.map(|udx| (udx.id, udx.seq)), Some((9, 0)));
    assert_eq!(payload.secret_stream, Some(SecretStreamInfo { version: 1 }));
    assert_eq!((payload.version, payload.addresses6), (1, vec![]));
    assert_eq!(payload.relay_through, None);
}

#[test]
fn firewalled_reply_carries_relays_as_given() {
    let relays = vec![relay(1), relay(2)];
    let payload = handshake_payload(&ours(FIREWALL_CONSISTENT, relays.clone()));
    let info = payload.holepunch.expect("holepunch info");
    assert_eq!(info.id, 77);
    assert_eq!(info.relays, relays);
    assert_eq!(info.relays[1].peer_address, peer("203.0.113.9", 2002));
}

#[test]
fn relay_addresses_capped_at_3() {
    let relays: Vec<RelayInfo> = (0..5).map(relay).collect();
    let payload = handshake_payload(&ours(FIREWALL_UNKNOWN, relays.clone()));
    let addresses = payload.relay_addresses.expect("relay addresses");
    let first: Vec<Ipv4Peer> = relays[..3]
        .iter()
        .map(|r| r.relay_address.clone())
        .collect();
    assert_eq!(addresses, first);
    assert_eq!(payload.holepunch.expect("holepunch").relays.len(), 5);
}

#[test]
fn no_relays_means_no_relay_addresses() {
    let payload = handshake_payload(&ours(FIREWALL_UNKNOWN, vec![]));
    assert_eq!(payload.relay_addresses, None);
}

#[test]
fn error_and_reusable_socket_are_passed_through() {
    let mut o = ours(FIREWALL_OPEN, vec![]);
    (o.error, o.udx_id, o.reusable_socket) = (ERROR_ABORTED, 0, true);
    let payload = handshake_payload(&o);
    assert_eq!(payload.error, ERROR_ABORTED);
    let udx = payload.udx.expect("udx");
    assert_eq!((udx.id, udx.reusable_socket, udx.version), (0, true, 1));
}

#[test]
fn abort_payload_matches_js() {
    let expected = HolepunchPayload {
        error: ERROR_ABORTED,
        firewall: FIREWALL_UNKNOWN,
        round: 6,
        connected: false,
        punching: false,
        addresses: None,
        remote_address: None,
        token: None,
        remote_token: None,
    };
    assert_eq!(abort_payload(6), expected);
}

#[test]
fn round_payload_never_claims_connected() {
    let addresses = vec![peer("192.168.1.5", 4242)];
    let payload = round_payload(
        ERROR_TRY_LATER,
        FIREWALL_CONSISTENT,
        3,
        true,
        addresses.clone(),
        Some([1; 32]),
        Some([2; 32]),
    );
    assert_eq!((payload.error, payload.firewall, payload.round), (3, 2, 3));
    assert_eq!((payload.connected, payload.punching), (false, true));
    assert_eq!(payload.addresses, Some(addresses));
    assert_eq!(payload.remote_address, None);
    assert_eq!(
        (payload.token, payload.remote_token),
        (Some([1; 32]), Some([2; 32]))
    );
}

#[test]
fn encode_holepunch_replies_with_request_id_and_peer_address() {
    let via = peer("203.0.113.9", 2000);
    let request = HolepunchMessage {
        mode: MODE_FROM_RELAY,
        id: 41,
        payload: vec![9],
        peer_address: Some(via.clone()),
    };
    let bytes = encode_holepunch(&request, &via, vec![5, 6]).unwrap();
    let decoded = decode_holepunch_msg_from_bytes(&bytes).unwrap();
    assert_eq!((decoded.mode, decoded.id), (MODE_FROM_SERVER, 41));
    assert_eq!(
        (decoded.payload, decoded.peer_address),
        (vec![5, 6], Some(via.clone()))
    );
    let direct = HolepunchMessage {
        mode: MODE_FROM_CLIENT,
        ..request
    };
    let decoded =
        decode_holepunch_msg_from_bytes(&encode_holepunch(&direct, &via, vec![]).unwrap()).unwrap();
    assert_eq!(
        (decoded.mode, decoded.peer_address),
        (MODE_REPLY, Some(via))
    );
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[test]
fn handshake_payload_without_relay_is_frozen() {
    // Reference hex of the encoding without `relay_through`; it must not change.
    let bytes =
        |p: &NoisePayload| hex(&hyperdht_messages::encode_noise_payload_to_bytes(p).unwrap());
    let open = handshake_payload(&ours(FIREWALL_OPEN, vec![relay(1)]));
    assert_eq!(
        bytes(&open),
        "015a000101c0a8010592100100090001010a000001e903"
    );
    let firewalled = handshake_payload(&ours(FIREWALL_CONSISTENT, vec![relay(1), relay(2)]));
    assert_eq!(
        bytes(&firewalled),
        "015b00024d020a000001e903cb007109d1070a000001ea03cb007109d20701c0a8010592100100090001020a000001e9030a000001ea03"
    );
}
