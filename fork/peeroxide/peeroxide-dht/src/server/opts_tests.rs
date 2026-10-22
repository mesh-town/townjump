//! Tests for [`crate::server::opts`].

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use super::*;

fn payload() -> NoisePayload {
    NoisePayload {
        version: 1,
        error: 0,
        firewall: 0,
        holepunch: None,
        addresses4: vec![],
        addresses6: vec![],
        udx: None,
        secret_stream: None,
        relay_through: None,
        relay_addresses: None,
    }
}

fn check(opts: &ServerOpts, key: u8) -> bool {
    let pk = [key; 32];
    let address = Ipv4Peer {
        host: "10.0.0.1".into(),
        port: 4000,
    };
    let remote_payload = payload();
    let input = FirewallInput {
        remote_public_key: &pk,
        remote_payload: &remote_payload,
        client_address: &address,
    };
    rejects(opts, &input)
}

#[test]
fn firewall_default_accepts_all() {
    let opts = ServerOpts::default();
    assert!(!check(&opts, 1));
}

#[test]
fn firewall_true_rejects() {
    let opts = ServerOpts::default().with_firewall(Arc::new(|_| true));
    assert!(check(&opts, 1));
}

#[test]
fn firewall_false_accepts_and_sees_the_key() {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&calls);
    let firewall: Firewall = Arc::new(move |input| {
        seen.fetch_add(1, Ordering::SeqCst);
        input.remote_public_key[0] != 7
    });
    let opts = ServerOpts::default().with_firewall(firewall);
    assert!(!check(&opts, 7));
    assert!(check(&opts, 8));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn firewall_panic_rejects() {
    let opts = ServerOpts::default().with_firewall(Arc::new(|_| panic!("firewall bug")));
    assert!(check(&opts, 1));
    assert!(check(&opts, 2));
}

#[test]
fn debug_hides_the_callback() {
    let opts = ServerOpts::default().with_firewall(Arc::new(|_| false));
    let text = format!("{opts:?}");
    assert!(text.contains("<callback>") && text.contains("reusable_socket"));
}

#[test]
fn relay_defaults() {
    let opts = ServerOpts::default();
    assert_eq!(opts.relay_keep_alive, Duration::from_millis(5000));
    assert!(opts.relay_through.is_none() && opts.holepunch.is_none());
}

#[test]
fn debug_hides_holepunch_callback() {
    let opts = ServerOpts {
        relay_through: Some(RelayThrough::Key([7; 32])),
        holepunch: Some(HolepunchFilter::new(|_| true)),
        ..ServerOpts::default()
    };
    let text = format!("{opts:?}");
    assert!(text.contains("relay_through") && text.contains("Key"));
    assert!(text.contains("holepunch: Some(\"<callback>\")"));
}
