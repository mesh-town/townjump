//! Unit tests of `pick_latest_mutable`: highest `seq` wins, bad signatures and low `seq` are ignored.

use super::records::pick_latest_mutable;
use super::*;
use crate::crypto::{NS_MUTABLE_PUT, mutable_signable, sign_detached};
use crate::hyperdht_messages::{MutableGetResponse, encode_mutable_get_response_to_bytes};
use crate::messages::Ipv4Peer;
use crate::query::QueryReply;
use std::time::Duration;

fn reply(kp: &KeyPair, seq: u64, value: &[u8], port: u16) -> QueryReply {
    let signable = mutable_signable(&NS_MUTABLE_PUT, seq, value);
    let bytes = encode_mutable_get_response_to_bytes(&MutableGetResponse {
        seq,
        value: value.to_vec(),
        signature: sign_detached(&signable, &kp.secret_key),
    })
    .unwrap();
    QueryReply {
        from: Ipv4Peer {
            host: "127.0.0.1".into(),
            port,
        },
        to: None,
        from_id: None,
        token: None,
        closer_nodes: vec![],
        error: 0,
        value: Some(bytes),
        rtt: Duration::ZERO,
    }
}

#[test]
fn picks_highest_seq_regardless_of_order() {
    let kp = KeyPair::generate();
    let r = vec![reply(&kp, 7, b"old", 1), reply(&kp, 10, b"new", 2)];
    let got = pick_latest_mutable(r, &kp.public_key, 0).unwrap();
    assert_eq!(
        (got.seq, got.value, got.from.port),
        (10, b"new".to_vec(), 2)
    );
    let r = vec![reply(&kp, 10, b"new", 2), reply(&kp, 7, b"old", 1)];
    assert_eq!(pick_latest_mutable(r, &kp.public_key, 0).unwrap().seq, 10);
}

#[test]
fn ignores_bad_signature_and_below_min_seq() {
    let kp = KeyPair::generate();
    let other = KeyPair::generate();
    let r = vec![reply(&other, 99, b"forged", 3), reply(&kp, 7, b"ok", 1)];
    assert_eq!(pick_latest_mutable(r, &kp.public_key, 0).unwrap().seq, 7);
    let r = vec![reply(&kp, 7, b"ok", 1)];
    assert!(pick_latest_mutable(r, &kp.public_key, 8).is_none());
}
