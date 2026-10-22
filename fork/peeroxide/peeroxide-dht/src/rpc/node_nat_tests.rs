use super::*;
use crate::rpc::consts::OLD_NODE;

fn peer(host: &str, port: u16) -> Ipv4Peer {
    Ipv4Peer {
        host: host.to_owned(),
        port,
    }
}

fn from(i: u16) -> Ipv4Peer {
    peer("9.9.9.9", 20_000 + i)
}

#[test]
fn firewalled_needs_three_samples() {
    let mut n = NodeNat::new(true);
    let a = peer("1.1.1.1", 7);
    for i in 0..3 {
        assert_eq!(n.remote_state(), (None, FIREWALL_UNKNOWN), "before {i}");
        assert!(n.sample(&a, &from(i), 0));
    }
    assert_eq!(n.remote_state(), (Some(a), FIREWALL_CONSISTENT));
}

#[test]
fn single_responder_never_settles() {
    let mut n = NodeNat::new(true);
    let a = peer("1.1.1.1", 7);
    for tick in [0, 12, 24, 36] {
        assert!(n.sample(&a, &from(0), tick), "tick {tick}");
        assert_eq!(n.remote_state(), (None, FIREWALL_UNKNOWN), "tick {tick}");
    }
    assert!(n.sample(&a, &from(1), 36));
    assert!(n.sample(&a, &from(2), 36));
    assert_eq!(n.remote_state(), (Some(a), FIREWALL_CONSISTENT));
}

#[test]
fn unfirewalled_reports_open_without_address() {
    let mut n = NodeNat::new(false);
    for i in 0..5 {
        n.sample(&peer("1.1.1.1", 7), &from(i), 0);
    }
    assert_eq!(n.remote_state(), (None, FIREWALL_OPEN));
}

#[test]
fn random_ports_report_host_and_port_zero() {
    let mut n = NodeNat::new(true);
    for i in 0..4 {
        n.sample(&peer("1.1.1.1", 100 + i), &from(i), 0);
    }
    assert_eq!(
        n.remote_state(),
        (Some(peer("1.1.1.1", 0)), FIREWALL_RANDOM)
    );
}

#[test]
fn disagreeing_window_reports_unknown() {
    let mut n = NodeNat::new(true);
    for i in 0..4 {
        let host = if i < 2 { "1.1.1.1" } else { "2.2.2.2" };
        n.sample(&peer(host, 7), &from(i), 0);
    }
    assert_eq!(n.remote_state(), (None, FIREWALL_UNKNOWN));
}

#[test]
fn empty_to_is_ignored() {
    let mut n = NodeNat::new(true);
    assert!(!n.sample(&peer("1.1.1.1", 0), &from(0), 0));
    assert!(!n.sample(&peer("", 7), &from(1), 0));
    assert_eq!(n.window().size(), 0);
}

#[test]
fn same_responder_once_per_interval() {
    let mut n = NodeNat::new(true);
    let a = peer("1.1.1.1", 7);
    assert!(n.sample(&a, &from(0), 100));
    assert!(!n.sample(&a, &from(0), 100));
    assert!(!n.sample(&a, &from(0), 111));
    assert!(n.sample(&a, &from(0), 112));
}

#[test]
fn chatty_responder_cannot_fill_window() {
    let mut n = NodeNat::new(true);
    let a = peer("1.1.1.1", 7);
    for i in 0..16 {
        assert!(n.sample(&a, &from(i), 0));
    }
    let accepted = (0..100)
        .filter(|_| n.sample(&peer("2.2.2.2", 8), &from(100), 0))
        .count();
    assert_eq!(accepted, 1);
    assert_eq!(n.remote_state(), (Some(a), FIREWALL_CONSISTENT));
}

#[test]
fn wake_up_jump_makes_responders_eligible() {
    let mut n = NodeNat::new(true);
    let a = peer("1.1.1.1", 7);
    assert!(n.sample(&a, &from(0), 5));
    assert!(!n.sample(&a, &from(0), 6));
    assert!(n.sample(&a, &from(0), 5 + 2 * OLD_NODE));
}

fn three_samples(n: &mut NodeNat, to: &Ipv4Peer) {
    for i in 0..3 {
        n.sample(to, &from(i), 0);
    }
}

#[test]
fn open_publishes_window_address() {
    let mut n = NodeNat::new(true);
    let a = peer("1.1.1.1", 4000);
    three_samples(&mut n, &a);
    n.open(4000);
    assert_eq!(n.remote_state(), (Some(a), FIREWALL_OPEN));
}

#[test]
fn open_other_port_publishes_no_address() {
    let mut n = NodeNat::new(true);
    three_samples(&mut n, &peer("1.1.1.1", 5000));
    n.open(4000);
    assert_eq!(n.remote_state(), (None, FIREWALL_OPEN));
}

#[test]
fn close_restores_firewalled_state() {
    let mut n = NodeNat::new(true);
    let a = peer("1.1.1.1", 4000);
    three_samples(&mut n, &a);
    n.open(4000);
    n.close(true);
    assert_eq!(n.remote_state(), (Some(a), FIREWALL_CONSISTENT));
    n.close(false);
    assert_eq!(n.remote_state(), (None, FIREWALL_OPEN));
}

#[test]
fn replace_window_changes_address() {
    let mut n = NodeNat::new(true);
    assert_eq!(n.address(), None);
    let mut w = NatWindow::new();
    w.add("2.2.2.2", 4001);
    n.replace_window(w);
    assert_eq!(n.address(), Some(peer("2.2.2.2", 4001)));
}

#[test]
fn host_survives_port_zero() {
    let mut n = NodeNat::new(true);
    for i in 0..4 {
        n.sample(&peer("1.1.1.1", 100 + i), &from(i), 0);
    }
    assert_eq!(n.host(), Some("1.1.1.1"));
    assert_eq!(n.address(), None);
}
