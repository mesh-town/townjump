//! Tests for `client_payload`: the bytes of the first handshake payload.
// JS: hyperdht/lib/connect.js:455

use super::super::relay::RelayAsk;
use super::*;
use crate::hyperdht_messages::{decode_noise_payload_from_bytes, encode_noise_payload_to_bytes};

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn frozen(opts: &ConnectOpts) -> String {
    let addrs = vec![Ipv4Peer {
        host: "1.2.3.4".into(),
        port: 5000,
    }];
    hex(&encode_noise_payload_to_bytes(&client_payload(addrs, 7, opts)).unwrap())
}

#[test]
fn client_payload_without_relay_is_frozen() {
    // Frozen wire bytes of the payload.
    assert_eq!(
        frozen(&ConnectOpts::default()),
        "011a0000010102030488130100070001"
    );
    let other = ConnectOpts {
        local_connection: false,
        reusable_socket: true,
        ..ConnectOpts::default()
    };
    assert_eq!(frozen(&other), "011a0000010102030488130101070001");
}

#[test]
fn client_payload_with_relay_roundtrips() {
    // given: options carrying a relay ask
    let opts = ConnectOpts {
        relay_ask: Some(RelayAsk {
            public_key: [7; 32],
            token: [9; 32],
        }),
        ..ConnectOpts::default()
    };
    // when: the payload is encoded and decoded again
    let bytes = encode_noise_payload_to_bytes(&client_payload(vec![], 3, &opts)).unwrap();
    let back = decode_noise_payload_from_bytes(&bytes).unwrap();
    // then: the relay info arrives intact
    let info = back.relay_through.expect("relay_through set");
    assert_eq!(
        (info.version, info.public_key, info.token),
        (1, [7; 32], [9; 32])
    );
    assert_eq!(back.udx.map(|u| u.id), Some(3));
}

#[test]
fn each_handshake_of_a_connect_gets_its_own_pairing_token() {
    // Concurrent handshakes reach the server as separate slots; a shared token would let the
    // relay match the client with the slot of a handshake the client did not complete.
    let ask = RelayAsk {
        public_key: [9; 32],
        token: [3; 32],
    };
    let (a, b) = (ask.for_handshake(), ask.for_handshake());
    assert_eq!((a.public_key, b.public_key), ([9; 32], [9; 32]));
    assert_ne!(a.token, b.token);
    assert_ne!(a.token, ask.token);
}
