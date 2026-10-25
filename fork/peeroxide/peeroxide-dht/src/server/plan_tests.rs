//! Tests for [`crate::server::plan`].

use super::*;
use crate::hyperdht_messages::{FIREWALL_CONSISTENT, FIREWALL_RANDOM, FIREWALL_UNKNOWN};

#[test]
fn holepunch_open_client_is_dialed_by_server() {
    assert_eq!(
        plan(FIREWALL_OPEN, false, FIREWALL_UNKNOWN),
        Plan::DialClient
    );
}

#[test]
fn direct_request_is_dialed() {
    assert_eq!(
        plan(FIREWALL_UNKNOWN, true, FIREWALL_UNKNOWN),
        Plan::DialClient
    );
    assert_eq!(plan(FIREWALL_RANDOM, true, FIREWALL_OPEN), Plan::DialClient);
}

#[test]
fn open_server_awaits_unknown_client() {
    assert_eq!(
        plan(FIREWALL_UNKNOWN, false, FIREWALL_OPEN),
        Plan::AwaitClient
    );
    assert_eq!(
        plan(FIREWALL_CONSISTENT, false, FIREWALL_OPEN),
        Plan::AwaitClient
    );
}

#[test]
fn firewalled_server_holepunches_unknown_client() {
    assert_eq!(
        plan(FIREWALL_UNKNOWN, false, FIREWALL_UNKNOWN),
        Plan::Holepunch
    );
    assert_eq!(
        plan(FIREWALL_RANDOM, false, FIREWALL_CONSISTENT),
        Plan::Holepunch
    );
}

#[test]
fn only_a_direct_dial_leaves_from_the_arrival_socket() {
    assert!(dial_from_arrival(Plan::DialClient, true));
    assert!(!dial_from_arrival(Plan::DialClient, false));
    assert!(!dial_from_arrival(Plan::AwaitClient, true));
    assert!(!dial_from_arrival(Plan::Holepunch, true));
}

#[test]
fn only_holepunch_answers_from_the_puncher() {
    assert!(reply_from_puncher(Plan::Holepunch));
    assert!(!reply_from_puncher(Plan::DialClient));
    assert!(!reply_from_puncher(Plan::AwaitClient));
}

/// The client is `127.0.0.1:7` unless a test says otherwise.
mod same_host_fn {
    use super::*;

    #[test]
    fn all_three_hold() {
        // given: our host equals the client's, the client announced that host and a LAN address
        let ours = peer("10.0.0.1", 1);
        let client = peer("10.0.0.1", 7);
        let addrs = [peer("10.0.0.1", 7), peer("192.168.1.5", 7)];
        // when / then: the client counts as sitting behind our IP (ports are not compared)
        assert!(same_host(Some(&ours), &client, &addrs, false));
    }

    #[test]
    fn loopback_client_with_a_lan_address() {
        // given: server and client both see each other on loopback; the client announced a LAN address
        let ours = peer("127.0.0.1", 1);
        let addrs = [peer("127.0.0.1", 7), peer("192.168.1.5", 1)];
        // when / then: same host (the loopback cluster of the tests)
        assert!(same_host(Some(&ours), &loopback_client(), &addrs, false));
    }

    #[test]
    fn without_own_address_is_false() {
        // given: the server has no `remote_address()` yet
        let addrs = [peer("127.0.0.1", 7), peer("192.168.1.5", 1)];
        // when / then: the default behaviour
        assert!(!same_host(None, &loopback_client(), &addrs, false));
    }

    #[test]
    fn other_host_is_false() {
        // given: the client's host differs from ours
        let ours = peer("10.0.0.2", 1);
        let client = peer("10.0.0.1", 7);
        let addrs = [peer("10.0.0.1", 7), peer("192.168.1.5", 7)];
        // when / then
        assert!(!same_host(Some(&ours), &client, &addrs, false));
    }

    #[test]
    fn unsettled_client_is_false() {
        // given: the client did not announce an address with its own host (its reflexive address is not settled)
        let ours = peer("127.0.0.1", 1);
        let addrs = [peer("192.168.1.5", 1)];
        // when / then
        assert!(!same_host(Some(&ours), &loopback_client(), &addrs, false));
    }

    #[test]
    fn lone_loopback_reflexive_is_false() {
        // given: the client announced only its own loopback host: loopback is private, but it is the client's host
        let ours = peer("127.0.0.1", 1);
        let addrs = [peer("127.0.0.1", 7)];
        // when / then: no LAN address, no shortcut
        assert!(!same_host(Some(&ours), &loopback_client(), &addrs, false));
    }

    #[test]
    fn direct_is_false() {
        // given: everything as in `loopback_client_with_a_lan_address`, but the request is direct
        let ours = peer("127.0.0.1", 1);
        let addrs = [peer("127.0.0.1", 7), peer("192.168.1.5", 1)];
        // when / then
        assert!(!same_host(Some(&ours), &loopback_client(), &addrs, true));
    }
}

mod client_agrees_fn {
    use super::*;

    #[test]
    fn finds_an_address_with_the_clients_host() {
        // given: the client's host is among the announced addresses (any port)
        let addrs = [peer("192.168.1.5", 1), peer("127.0.0.1", 9)];
        // when / then
        assert!(client_agrees(&loopback_client(), &addrs));
    }

    #[test]
    fn without_the_clients_host_is_false() {
        // given: only other hosts were announced
        let addrs = [peer("192.168.1.5", 7), peer("10.0.0.1", 7)];
        // when / then
        assert!(!client_agrees(&loopback_client(), &addrs));
    }
}

mod client_lan_fn {
    use super::*;

    #[test]
    fn private_address_of_another_host_counts() {
        // given: besides its own host the client announced a private address
        let addrs = [peer("127.0.0.1", 7), peer("192.168.1.5", 1)];
        // when / then
        assert!(client_lan(&loopback_client(), &addrs));
    }

    #[test]
    fn the_clients_own_private_host_does_not_count() {
        // given: the client announced only its own (private) host
        let client = peer("10.0.0.1", 7);
        let addrs = [peer("10.0.0.1", 7), peer("10.0.0.1", 8)];
        // when / then
        assert!(!client_lan(&client, &addrs));
    }

    #[test]
    fn public_address_of_another_host_does_not_count() {
        // given: besides its own host the client announced a public address
        let addrs = [peer("127.0.0.1", 7), peer("8.8.8.8", 1)];
        // when / then
        assert!(!client_lan(&loopback_client(), &addrs));
    }
}

fn peer(host: &str, port: u16) -> Ipv4Peer {
    Ipv4Peer {
        host: host.into(),
        port,
    }
}

fn loopback_client() -> Ipv4Peer {
    peer("127.0.0.1", 7)
}
