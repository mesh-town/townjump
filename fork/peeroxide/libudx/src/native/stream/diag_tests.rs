//! `UDX_DIAG` parsing: pure `parse`, the environment is not touched.
use super::diag::{Mode, parse};

fn mode(z: bool, f1: bool) -> Mode {
    Mode { z, f1 }
}

#[test]
fn empty_value_leaves_f1_on_and_z_off() {
    assert_eq!(parse(""), mode(false, true));
}

#[test]
fn single_tokens() {
    assert_eq!(parse("z"), mode(true, true));
    assert_eq!(parse("f1"), mode(false, true));
    assert_eq!(parse("nof1"), mode(false, false));
}

#[test]
fn combined_tokens_and_unknown_ones() {
    assert_eq!(parse("z,f1"), mode(true, true));
    assert_eq!(parse("f1, z"), mode(true, true));
    assert_eq!(parse("z,nof1"), mode(true, false));
    assert_eq!(parse("x,zz"), mode(false, true));
}

#[test]
fn last_f1_token_wins() {
    assert_eq!(parse("nof1,f1"), mode(false, true));
    assert_eq!(parse("f1,nof1"), mode(false, false));
}
