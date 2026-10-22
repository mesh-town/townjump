//! Tests for `match_address` of the `holepuncher` module.

use super::*;

fn peer(host: &str, port: u16) -> Ipv4Peer {
    Ipv4Peer {
        host: host.into(),
        port,
    }
}

#[test]
fn match_address_exact_subnet() {
    let local = vec![peer("192.168.1.100", 5000)];
    let remote = vec![peer("192.168.1.50", 6000)];
    let result = match_address(&local, &remote).unwrap();
    assert_eq!(result.host, "192.168.1.50");
}

#[test]
fn match_address_partial_subnet() {
    let local = vec![peer("192.168.1.100", 5000)];
    let remote = vec![peer("192.168.2.50", 6000)];
    let result = match_address(&local, &remote).unwrap();
    assert_eq!(result.host, "192.168.2.50");
}

#[test]
fn match_address_different_network() {
    let local = vec![peer("192.168.1.100", 5000)];
    let remote = vec![peer("10.0.0.1", 6000)];
    assert!(match_address(&local, &remote).is_none());
}

#[test]
fn match_address_empty_remote() {
    let local = vec![peer("192.168.1.100", 5000)];
    assert!(match_address(&local, &[]).is_none());
}

#[test]
fn match_address_picks_closest() {
    let local = vec![peer("192.168.1.100", 5000)];
    let remote = vec![peer("192.168.2.50", 6000), peer("192.168.1.99", 7000)];
    let result = match_address(&local, &remote).unwrap();
    assert_eq!(result.host, "192.168.1.99");
}

#[test]
fn match_address_first_octet_only() {
    let local = vec![peer("10.0.0.1", 5000)];
    let remote = vec![peer("10.1.2.3", 6000)];
    let result = match_address(&local, &remote).unwrap();
    assert_eq!(result.host, "10.1.2.3");
}
