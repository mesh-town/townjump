use super::*;

#[test]
fn pattern_is_deterministic_and_salted() {
    assert_eq!(pattern(1000, 1), pattern(1000, 1));
    assert_ne!(pattern(1000, 1), pattern(1000, 2));
    assert_eq!(pattern(3, 5), [5, 36, 67]);
    assert_eq!(pattern(0, 1), Vec::<u8>::new());
}

#[test]
fn assert_same_accepts_equal() {
    assert_same(&pattern(10, 1), &pattern(10, 1), "equal");
}

#[test]
#[should_panic(expected = "position Some(3)")]
fn assert_same_names_first_difference() {
    let mut other = pattern(10, 1);
    other[3] ^= 1;
    assert_same(&other, &pattern(10, 1), "different");
}

#[test]
#[should_panic(expected = "length mismatch")]
fn assert_same_rejects_length() {
    assert_same(&[1, 2], &[1], "length");
}
