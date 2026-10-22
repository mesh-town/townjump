//! Tests of `UDX_FIX` parsing.
use super::*;

fn mode_of(pace: bool, ackstep: bool) -> Mode {
    Mode { pace, ackstep }
}

#[test]
fn empty_means_pace_and_ackstep() {
    assert_eq!(parse(""), mode_of(true, true));
}

#[test]
fn pace_and_ackstep_tokens_change_nothing() {
    assert_eq!(parse("pace"), mode_of(true, true));
    assert_eq!(parse("ackstep"), mode_of(true, true));
    assert_eq!(parse("pace,ackstep"), mode_of(true, true));
}

#[test]
fn legacy_turns_pace_and_ackstep_off() {
    assert_eq!(parse("legacy"), mode_of(false, false));
    assert_eq!(parse("pace,ackstep,legacy"), mode_of(false, false));
    assert_eq!(parse("legacy,pace"), mode_of(false, false));
}

#[test]
fn rack_token_changes_nothing() {
    assert_eq!(parse("rack"), parse(""));
    assert_eq!(parse("legacy,rack"), parse("legacy"));
    assert_eq!(parse(" rack , legacy"), parse("legacy"));
}

#[test]
fn unknown_token_is_ignored() {
    assert_eq!(parse("nope"), mode_of(true, true));
    assert_eq!(parse("nope, legacy"), mode_of(false, false));
}
