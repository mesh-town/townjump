//! Tests for [`crate::noise_wrap`].

use super::*;
use crate::hyperdht_messages::NoisePayload;
use crate::noise::{generate_keypair, keypair_from_seed};

fn minimal_payload(firewall: u64) -> NoisePayload {
    NoisePayload {
        version: 1,
        error: 0,
        firewall,
        holepunch: None,
        addresses4: vec![],
        addresses6: vec![],
        udx: None,
        secret_stream: None,
        relay_through: None,
        relay_addresses: None,
    }
}

#[test]
fn roundtrip_initiator_responder() {
    let kp_init = generate_keypair();
    let kp_resp = generate_keypair();
    let init_pub = kp_init.public_key;
    let resp_pub = kp_resp.public_key;

    let mut init = NoiseWrap::new_initiator(kp_init, resp_pub);
    let mut resp = NoiseWrap::new_responder(kp_resp);

    let m1 = init.send(&minimal_payload(1)).unwrap();

    let recv_payload = resp.recv(&m1).unwrap();
    assert_eq!(recv_payload.firewall, 1);

    let m2 = resp.send(&minimal_payload(2)).unwrap();

    let recv_payload2 = init.recv(&m2).unwrap();
    assert_eq!(recv_payload2.firewall, 2);

    assert!(init.complete());
    assert!(resp.complete());

    let init_result = init.finalize().unwrap();
    let resp_result = resp.finalize().unwrap();

    assert_eq!(init_result.remote_public_key, resp_pub);
    assert_eq!(resp_result.remote_public_key, init_pub);
    assert!(init_result.is_initiator);
    assert!(!resp_result.is_initiator);

    assert_eq!(init_result.tx, resp_result.rx);
    assert_eq!(init_result.rx, resp_result.tx);

    assert_eq!(init_result.handshake_hash, resp_result.handshake_hash);

    assert_eq!(init_result.holepunch_secret, resp_result.holepunch_secret);
    assert_ne!(init_result.holepunch_secret, [0u8; 32]);
}

#[test]
fn remote_public_key_known_after_recv() {
    let kp_init = generate_keypair();
    let kp_resp = generate_keypair();
    let init_pub = kp_init.public_key;
    let mut init = NoiseWrap::new_initiator(kp_init, kp_resp.public_key);
    let mut resp = NoiseWrap::new_responder(kp_resp);

    assert_eq!(resp.remote_public_key(), None);
    let m1 = init.send(&minimal_payload(1)).unwrap();
    resp.recv(&m1).unwrap();

    assert_eq!(resp.remote_public_key(), Some(init_pub));
}

#[test]
fn deterministic_holepunch_secret() {
    let seed_init = [0x11u8; 32];
    let seed_resp = [0x22u8; 32];
    let seed_e_init = [0x33u8; 32];
    let seed_e_resp = [0x44u8; 32];

    let kp_init = keypair_from_seed(&seed_init);
    let kp_resp = keypair_from_seed(&seed_resp);

    let mut init = NoiseWrap::new_initiator(kp_init.clone(), kp_resp.public_key);
    let mut resp = NoiseWrap::new_responder(kp_resp.clone());

    init.set_ephemeral(keypair_from_seed(&seed_e_init));
    resp.set_ephemeral(keypair_from_seed(&seed_e_resp));

    let m1 = init.send(&minimal_payload(1)).unwrap();
    resp.recv(&m1).unwrap();
    let m2 = resp.send(&minimal_payload(2)).unwrap();
    init.recv(&m2).unwrap();

    let r1 = init.finalize().unwrap();

    // Repeat with same seeds
    let kp_init2 = keypair_from_seed(&seed_init);
    let kp_resp2 = keypair_from_seed(&seed_resp);

    let mut init2 = NoiseWrap::new_initiator(kp_init2, kp_resp2.public_key);
    let mut resp2 = NoiseWrap::new_responder(kp_resp2);

    init2.set_ephemeral(keypair_from_seed(&seed_e_init));
    resp2.set_ephemeral(keypair_from_seed(&seed_e_resp));

    let m1b = init2.send(&minimal_payload(1)).unwrap();
    resp2.recv(&m1b).unwrap();
    let m2b = resp2.send(&minimal_payload(2)).unwrap();
    init2.recv(&m2b).unwrap();

    let r2 = init2.finalize().unwrap();

    assert_eq!(r1.holepunch_secret, r2.holepunch_secret);
    assert_eq!(r1.handshake_hash, r2.handshake_hash);
    assert_eq!(r1.tx, r2.tx);
    assert_eq!(r1.rx, r2.rx);
}

#[test]
fn finalize_before_complete_errors() {
    let kp_init = generate_keypair();
    let kp_resp = generate_keypair();
    let init = NoiseWrap::new_initiator(kp_init, kp_resp.public_key);
    let err = init.finalize().unwrap_err();
    assert!(matches!(err, NoiseWrapError::NotComplete));
}

#[test]
fn payload_with_all_fields_roundtrips() {
    use crate::hyperdht_messages::{HolepunchInfo, RelayThroughInfo, SecretStreamInfo, UdxInfo};
    use crate::messages::Ipv4Peer;

    let kp_init = generate_keypair();
    let kp_resp = generate_keypair();

    let mut init = NoiseWrap::new_initiator(kp_init, kp_resp.public_key);
    let mut resp = NoiseWrap::new_responder(kp_resp);

    let rich_payload = NoisePayload {
        version: 1,
        error: 0,
        firewall: 2,
        holepunch: Some(HolepunchInfo {
            id: 42,
            relays: vec![],
        }),
        addresses4: vec![Ipv4Peer {
            host: "192.168.1.100".to_string(),
            port: 9999,
        }],
        addresses6: vec![],
        udx: Some(UdxInfo {
            version: 1,
            reusable_socket: false,
            id: 7,
            seq: 0,
        }),
        secret_stream: Some(SecretStreamInfo { version: 1 }),
        relay_through: Some(RelayThroughInfo {
            version: 1,
            public_key: [0xBBu8; 32],
            token: [0xCCu8; 32],
        }),
        relay_addresses: None,
    };

    let m1 = init.send(&rich_payload).unwrap();
    let decoded = resp.recv(&m1).unwrap();

    assert_eq!(decoded.firewall, 2);
    assert_eq!(decoded.holepunch.as_ref().unwrap().id, 42);
    assert_eq!(decoded.addresses4.len(), 1);
    assert_eq!(decoded.addresses4[0].port, 9999);
    assert_eq!(decoded.udx.as_ref().unwrap().id, 7);
    assert_eq!(decoded.secret_stream.as_ref().unwrap().version, 1);
    assert_eq!(
        decoded.relay_through.as_ref().unwrap().public_key,
        [0xBBu8; 32]
    );
}

#[test]
fn holepunch_secret_is_keyed_blake2b() {
    // Manually verify the derivation matches the expected formula.
    let hash = [0x55u8; 64];
    let secret = derive_holepunch_secret(&hash);

    // Recompute independently
    let mut mac: Blake2bMac256 = KeyInit::new_from_slice(&hash[..]).expect("64-byte key");
    Mac::update(&mut mac, &*NS_PEER_HOLEPUNCH);
    let expected = mac.finalize().into_bytes();
    let mut expected_arr = [0u8; 32];
    expected_arr.copy_from_slice(&expected);

    assert_eq!(secret, expected_arr);
}

#[test]
fn different_keypairs_different_secrets() {
    let kp_init = generate_keypair();

    let kp_resp_a = generate_keypair();
    let kp_resp_b = generate_keypair();

    // Handshake A
    let mut init_a = NoiseWrap::new_initiator(kp_init.clone(), kp_resp_a.public_key);
    let mut resp_a = NoiseWrap::new_responder(kp_resp_a);
    let m1a = init_a.send(&minimal_payload(1)).unwrap();
    resp_a.recv(&m1a).unwrap();
    let m2a = resp_a.send(&minimal_payload(1)).unwrap();
    init_a.recv(&m2a).unwrap();
    let ra = init_a.finalize().unwrap();

    // Handshake B (different responder)
    let kp_init2 = generate_keypair();
    let mut init_b = NoiseWrap::new_initiator(kp_init2, kp_resp_b.public_key);
    let mut resp_b = NoiseWrap::new_responder(kp_resp_b);
    let m1b = init_b.send(&minimal_payload(1)).unwrap();
    resp_b.recv(&m1b).unwrap();
    let m2b = resp_b.send(&minimal_payload(1)).unwrap();
    init_b.recv(&m2b).unwrap();
    let rb = init_b.finalize().unwrap();

    // Different responders → different secrets
    assert_ne!(ra.holepunch_secret, rb.holepunch_secret);
}
