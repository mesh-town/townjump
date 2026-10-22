// JS: libudx/src/udx.c
//! Tests of `is_plausible_first_packet`: which first packets are admitted.

use super::*;

fn header_with_flags(type_flags: u8) -> Header {
    Header {
        type_flags,
        data_offset: 0,
        remote_id: 42,
        recv_window: DEFAULT_RWND,
        seq: 1,
        ack: 0,
    }
}

#[test]
fn first_packet_with_data_flag_is_plausible() {
    let hdr = header_with_flags(FLAG_DATA);
    assert!(is_plausible_first_packet(&hdr));
}

#[test]
fn first_packet_with_heartbeat_is_plausible() {
    let hdr = header_with_flags(FLAG_HEARTBEAT);
    assert!(is_plausible_first_packet(&hdr));
}

#[test]
fn first_packet_with_data_and_sack_is_plausible() {
    let hdr = header_with_flags(FLAG_DATA | FLAG_SACK);
    assert!(is_plausible_first_packet(&hdr));
}

#[test]
fn first_packet_with_end_is_plausible() {
    let hdr = header_with_flags(FLAG_END);
    assert!(
        is_plausible_first_packet(&hdr),
        "a zero-length stream may legitimately lead with FLAG_END"
    );
}

#[test]
fn bare_first_packet_is_rejected() {
    let hdr = header_with_flags(0);
    assert!(
        !is_plausible_first_packet(&hdr),
        "type_flags == 0 carries no meaningful intent and must not commit the 4-tuple"
    );
}

#[test]
fn destroy_only_first_packet_is_rejected() {
    let hdr = header_with_flags(FLAG_DESTROY);
    assert!(
        !is_plausible_first_packet(&hdr),
        "FLAG_DESTROY before any agreed state is illegitimate"
    );
}

#[test]
fn destroy_combined_with_data_is_still_rejected() {
    let hdr = header_with_flags(FLAG_DESTROY | FLAG_DATA);
    assert!(
        !is_plausible_first_packet(&hdr),
        "FLAG_DESTROY in any combination is illegitimate for a first packet"
    );
}

#[test]
fn sack_only_first_packet_is_rejected() {
    let hdr = header_with_flags(FLAG_SACK);
    assert!(
        !is_plausible_first_packet(&hdr),
        "SACK-only first packet is meaningless before we have sent any data"
    );
}
