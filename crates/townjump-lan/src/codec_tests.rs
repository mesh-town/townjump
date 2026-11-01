use super::*;

fn req() -> Request {
    Request {
        nonce: [1, 2, 3, 4, 5, 6, 7, 8],
        target: [9; 32],
    }
}

fn reply() -> Reply {
    Reply {
        nonce: req().nonce,
        target: req().target,
        port: 0x1234,
    }
}

#[test]
fn request_roundtrip() {
    let bytes = encode_request(&req());
    assert_eq!(bytes.len(), REQUEST_LEN);
    assert_eq!(decode_request(&bytes), Some(req()));
}

#[test]
fn reply_roundtrip() {
    let bytes = encode_reply(&reply());
    assert_eq!(bytes.len(), REPLY_LEN);
    assert_eq!(&bytes[45..], &[0x12, 0x34]);
    assert_eq!(decode_reply(&bytes), Some(reply()));
}

#[test]
fn rejects_bad_magic() {
    let mut bytes = encode_reply(&reply());
    bytes[0] = b'X';
    assert_eq!(decode_reply(&bytes), None);
    assert_eq!(decode_request(&bytes), None);
}

#[test]
fn rejects_unknown_version() {
    let mut bytes = encode_reply(&reply());
    bytes[4] = 2;
    assert_eq!(decode_reply(&bytes), None);
    assert_eq!(decode_request(&bytes), None);
}

#[test]
fn rejects_short_packet() {
    let bytes = encode_reply(&reply());
    assert_eq!(decode_request(&bytes[..44]), None);
    assert_eq!(decode_reply(&bytes[..46]), None);
    assert_eq!(decode_reply(&encode_request(&req())), None);
    assert_eq!(decode_reply(&[]), None);
}

#[test]
fn ignores_trailing_bytes() {
    let mut bytes = encode_request(&req());
    bytes.extend_from_slice(b"future");
    assert_eq!(decode_request(&bytes), Some(req()));
    let mut bytes = encode_reply(&reply());
    bytes.extend_from_slice(b"future");
    assert_eq!(decode_reply(&bytes), Some(reply()));
}

#[test]
fn wire_bytes_are_fixed() {
    let mut want = b"TJLD".to_vec();
    want.push(1);
    want.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
    want.extend_from_slice(&[9; 32]);
    assert_eq!(want.len(), 45);
    assert_eq!(encode_request(&req()), want);
    want.extend_from_slice(&[0x12, 0x34]);
    assert_eq!(want.len(), 47);
    assert_eq!(encode_reply(&reply()), want);
    let cfg = crate::LanConfig::default();
    assert_eq!(cfg.group, Ipv4Addr::new(239, 255, 84, 74));
    assert_eq!(cfg.port, 28474);
}
