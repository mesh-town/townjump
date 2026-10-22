// Tests for `seed`: hex as in `Buffer.from(hex, 'hex')`, an empty seed is random.
use super::*;

#[test]
fn full_hex_is_kept_as_given() {
    let text = "ab".repeat(32);
    let (seed, shown) = resolve(Some(&text)).unwrap();
    assert_eq!((seed, shown), ([0xab; 32], text));
}

#[test]
fn short_or_broken_hex_is_a_seed_length_error() {
    assert!(resolve(Some("abcd")).is_err());
    assert!(resolve(Some(&format!("zz{}", "ab".repeat(31)))).is_err());
}

#[test]
fn empty_or_missing_seed_is_random_and_hex_matches() {
    for input in [None, Some("")] {
        let (seed, shown) = resolve(input).unwrap();
        assert_eq!(shown, encode(&seed));
        assert_eq!(shown.len(), 64);
    }
}
