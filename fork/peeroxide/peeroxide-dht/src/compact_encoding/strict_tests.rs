//! Strict decoder of compact-encoding 3.5.2 (vectors: testvectors/deltas/codec.json `d12.new`).
use super::*;

fn state(hex_str: &str) -> State {
    State::from_buffer(&hex::decode(hex_str).unwrap())
}

#[test]
fn uint_decodes_up_to_max_safe() {
    let ok = [
        ("00", 0u64),
        ("fc", 252),
        ("fdfd00", 253),
        ("fdffff", 65535),
        ("fe00000100", 65536),
        ("feffffffff", 4294967295),
        ("ff0000000001000000", 4294967296),
        ("ffffffffffffff1f00", 9007199254740991),
    ];
    for (h, v) in ok {
        assert_eq!(decode_uint(&mut state(h)).unwrap(), v, "{h}");
    }
}

#[test]
fn uint_above_max_safe_is_error() {
    for h in ["ff0000000000002000", "ffffffffffffffff"] {
        assert!(decode_uint(&mut state(h)).is_err(), "{h}");
    }
}

#[test]
fn uint56_above_max_safe_is_error() {
    assert!(decode_uint56(&mut state("00000000000020")).is_err());
    assert_eq!(
        decode_uint56(&mut state("ffffffffffff1f")).unwrap(),
        MAX_SAFE_UINT
    );
}

#[test]
fn buffer_decode_vectors() {
    assert_eq!(decode_buffer(&mut state("00")).unwrap(), Some(vec![]));
    assert_eq!(
        decode_buffer(&mut state("03aabbcc")).unwrap(),
        Some(vec![0xaa, 0xbb, 0xcc])
    );
    assert!(decode_buffer(&mut state("0300")).is_err());
    assert!(decode_buffer(&mut state("03aa")).is_err());
}

#[test]
fn buffer_huge_length_is_error_not_panic() {
    assert!(decode_buffer(&mut state("ffffffffffffff1f00")).is_err());
}

#[test]
fn optional_buffer_empty_is_none() {
    assert_eq!(decode_optional_buffer(&mut state("00")).unwrap(), None);
    assert_eq!(
        decode_optional_buffer(&mut state("02aabb")).unwrap(),
        Some(vec![0xaa, 0xbb])
    );
}
