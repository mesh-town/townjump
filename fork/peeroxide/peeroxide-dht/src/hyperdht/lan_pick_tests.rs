use std::net::Ipv4Addr;

use super::{pick_direct_address, pick_lan_addr};
use crate::messages::Ipv4Peer;

fn peer(h: &str) -> Ipv4Peer {
    Ipv4Peer {
        host: h.to_string(),
        port: 35400,
    }
}

fn net(s: &str, len: u8) -> (Ipv4Addr, u8) {
    (s.parse().unwrap(), len)
}

#[test]
fn picks_address_on_own_subnet() {
    let adv = [peer("192.168.1.100"), peer("10.42.0.123")];
    let got = pick_lan_addr(&adv, &[net("10.42.0.129", 24)]);
    assert_eq!(got, Some(peer("10.42.0.123")));
}

#[test]
fn first_in_advertised_order_wins() {
    let adv = [peer("192.168.1.100"), peer("10.42.0.123")];
    let nets = [net("192.168.1.5", 24), net("10.42.0.129", 24)];
    assert_eq!(pick_lan_addr(&adv, &nets), Some(peer("192.168.1.100")));
}

#[test]
fn no_common_subnet_or_empty_inputs_give_none() {
    let adv = [peer("192.168.1.100")];
    assert_eq!(pick_lan_addr(&adv, &[net("10.0.0.2", 24)]), None);
    assert_eq!(pick_lan_addr(&adv, &[]), None);
    assert_eq!(pick_lan_addr(&[], &[net("10.0.0.2", 24)]), None);
}

#[test]
fn public_garbage_and_bad_prefix_give_none() {
    assert_eq!(
        pick_lan_addr(&[peer("8.8.8.8")], &[net("8.8.8.4", 24)]),
        None
    );
    assert_eq!(
        pick_lan_addr(&[peer("not-an-ip")], &[net("10.0.0.2", 24)]),
        None
    );
    let adv = [peer("10.1.2.3")];
    assert_eq!(pick_lan_addr(&adv, &[net("10.1.2.9", 0)]), None);
    assert_eq!(pick_lan_addr(&adv, &[net("10.1.2.9", 33)]), None);
}

#[test]
fn mask_is_respected_for_non_24() {
    let adv = [peer("10.1.2.3")];
    assert_eq!(
        pick_lan_addr(&adv, &[net("10.1.180.7", 16)]),
        Some(peer("10.1.2.3"))
    );
    assert_eq!(pick_lan_addr(&adv, &[net("10.2.0.7", 16)]), None);
}

#[test]
fn loopback_advertised_matches_loopback_interface() {
    let adv = [peer("127.0.0.1"), peer("192.168.1.100")];
    assert_eq!(
        pick_lan_addr(&adv, &[net("127.0.0.1", 8)]),
        Some(peer("127.0.0.1"))
    );
}

#[test]
fn own_bridge_address_is_skipped() {
    let adv = [peer("172.17.0.1"), peer("192.168.1.100")];
    assert_eq!(pick_lan_addr(&adv, &[net("172.17.0.1", 16)]), None);
}

#[test]
fn own_address_skipped_neighbor_in_same_subnet_kept() {
    let adv = [peer("10.42.0.129"), peer("10.42.0.123")];
    let got = pick_lan_addr(&adv, &[net("10.42.0.129", 24)]);
    assert_eq!(got, Some(peer("10.42.0.123")));
}

#[test]
fn own_loopback_address_still_matches() {
    let adv = [peer("127.0.0.1")];
    assert_eq!(
        pick_lan_addr(&adv, &[net("127.0.0.1", 8)]),
        Some(peer("127.0.0.1"))
    );
}

#[test]
fn direct_dial_keeps_the_dialed_address_when_a_public_one_is_known() {
    let adv = [peer("8.8.8.8"), peer("10.0.0.5")];
    let server = peer("198.51.100.9");
    for same_host in [false, true] {
        let got = pick_direct_address(true, same_host, &adv, &server, &[net("10.0.0.2", 24)]);
        assert_eq!(got, server);
    }
}

#[test]
fn relayed_pick_is_unchanged() {
    let server = peer("198.51.100.9");
    let adv = [peer("10.0.0.5"), peer("8.8.8.8"), peer("9.9.9.9")];
    let got = pick_direct_address(false, false, &adv, &server, &[net("10.0.0.2", 24)]);
    assert_eq!(got, peer("8.8.8.8"));
    let got = pick_direct_address(false, false, &[peer("10.0.0.5")], &server, &[]);
    assert_eq!(got, server);
}

#[test]
fn server_as_holder_keeps_the_lan_address() {
    let server = peer("198.51.100.9");
    let adv = [peer("8.8.8.8"), peer("192.168.5.5"), peer("10.0.0.5")];
    let nets = [net("10.0.0.2", 24)];
    // own subnet first
    assert_eq!(
        pick_direct_address(false, true, &adv, &server, &nets),
        peer("10.0.0.5")
    );
    // no own-subnet match: first private
    assert_eq!(
        pick_direct_address(false, true, &adv, &server, &[]),
        peer("192.168.5.5")
    );
    // no private: first public
    assert_eq!(
        pick_direct_address(false, true, &[peer("8.8.8.8")], &server, &nets),
        peer("8.8.8.8")
    );
    // nothing advertised: server address
    assert_eq!(
        pick_direct_address(false, true, &[], &server, &nets),
        server
    );
}
