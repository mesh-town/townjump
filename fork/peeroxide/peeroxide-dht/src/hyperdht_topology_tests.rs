//! Unit tests of `should_direct_connect` and the connect topologies (open, firewalled, CGNAT).

use super::connect_finish::client_same_host;
use super::*;
use crate::hyperdht_messages::{
    FIREWALL_CONSISTENT, FIREWALL_OPEN, FIREWALL_RANDOM, FIREWALL_UNKNOWN,
};
use crate::messages::Ipv4Peer;

#[test]
fn direct_connect_when_not_relayed() {
    assert!(should_direct_connect(false, FIREWALL_RANDOM, true, false));
}

#[test]
fn direct_connect_when_firewall_open() {
    assert!(should_direct_connect(true, FIREWALL_OPEN, true, false));
}

#[test]
fn direct_connect_when_not_holepunchable() {
    assert!(should_direct_connect(true, FIREWALL_RANDOM, false, false));
}

#[test]
fn direct_connect_when_same_host() {
    assert!(should_direct_connect(true, FIREWALL_RANDOM, true, true));
}

#[test]
fn holepunch_when_relayed_firewalled_holepunchable_different_host() {
    assert!(!should_direct_connect(true, FIREWALL_RANDOM, true, false));
    assert!(!should_direct_connect(
        true,
        FIREWALL_CONSISTENT,
        true,
        false
    ));
    assert!(!should_direct_connect(true, FIREWALL_UNKNOWN, true, false));
}

#[test]
fn direct_connect_all_conditions_false_except_one() {
    assert!(should_direct_connect(false, FIREWALL_RANDOM, true, false));
    assert!(should_direct_connect(true, FIREWALL_OPEN, true, false));
    assert!(should_direct_connect(true, FIREWALL_RANDOM, false, false));
    assert!(should_direct_connect(true, FIREWALL_RANDOM, true, true));
}

// ── Scenario-matrix topology tests ───────────────────────────────────
//
// These cover the following topology matrix:
//   T1: both open
//   T2: sender firewalled, receiver open
//   T3: sender open, receiver firewalled
//   T4: both firewalled, same network
//   T5: both firewalled, different networks
//   T6: one behind CGNAT
//
// For each topology we test the connection decision from the
// *connector's* perspective (the one calling should_direct_connect).

#[test]
fn topology_both_open() {
    // T1: Neither side relayed, both FIREWALL_OPEN → direct connect
    assert!(should_direct_connect(false, FIREWALL_OPEN, true, false));
}

#[test]
fn topology_sender_firewalled_receiver_open() {
    // T2: Sender is firewalled, discovered receiver via relay (relayed=true).
    // Receiver is FIREWALL_OPEN → direct connect (firewall==OPEN branch).
    assert!(should_direct_connect(true, FIREWALL_OPEN, true, false));
}

#[test]
fn topology_sender_open_receiver_firewalled() {
    // T3: Sender is open. Found receiver via relay (relayed=true).
    // Receiver is firewalled (CONSISTENT), holepunchable → holepunch.
    assert!(!should_direct_connect(
        true,
        FIREWALL_CONSISTENT,
        true,
        false
    ));
    // If receiver is NOT holepunchable → direct connect (fallback).
    assert!(should_direct_connect(
        true,
        FIREWALL_CONSISTENT,
        false,
        false
    ));
}

#[test]
fn topology_both_firewalled_same_network() {
    // T4: Both firewalled, same network (same_host=true).
    // same_host → direct connect regardless of firewall state.
    assert!(should_direct_connect(true, FIREWALL_RANDOM, true, true));
    assert!(should_direct_connect(true, FIREWALL_CONSISTENT, true, true));
}

#[test]
fn topology_both_firewalled_different_networks() {
    // T5: Both firewalled, different networks.
    // When holepunchable → holepunch attempt.
    assert!(!should_direct_connect(true, FIREWALL_RANDOM, true, false));
    assert!(!should_direct_connect(
        true,
        FIREWALL_CONSISTENT,
        true,
        false
    ));
    // When NOT holepunchable → direct connect fallback (no HP relay).
    assert!(should_direct_connect(true, FIREWALL_RANDOM, false, false));
    assert!(should_direct_connect(
        true,
        FIREWALL_CONSISTENT,
        false,
        false
    ));
}

#[test]
fn topology_one_behind_cgnat() {
    // T6: CGNAT peer is FIREWALL_RANDOM and holepunchable.
    // Non-CGNAT peer connects via relay → holepunch.
    assert!(!should_direct_connect(true, FIREWALL_RANDOM, true, false));
    // CGNAT peer with no holepunch support → direct connect fallback.
    assert!(should_direct_connect(true, FIREWALL_RANDOM, false, false));
}

#[test]
fn same_host_needs_own_address() {
    assert!(!client_same_host(true, None, "1.2.3.4"));
}

#[test]
fn same_host_when_hosts_match() {
    let ours = Ipv4Peer {
        host: "1.2.3.4".into(),
        port: 5,
    };
    assert!(client_same_host(true, Some(&ours), "1.2.3.4"));
}

#[test]
fn other_host_is_not_same_host() {
    let ours = Ipv4Peer {
        host: "5.6.7.8".into(),
        port: 5,
    };
    assert!(!client_same_host(true, Some(&ours), "1.2.3.4"));
}

#[test]
fn no_same_host_without_local_connection() {
    let ours = Ipv4Peer {
        host: "1.2.3.4".into(),
        port: 5,
    };
    assert!(!client_same_host(false, Some(&ours), "1.2.3.4"));
}
