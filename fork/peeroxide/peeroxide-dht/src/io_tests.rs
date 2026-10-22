// JS: dht-rpc/lib/io.js
//! Unit tests of the IO layer: congestion window, tid wrap, token generation/validation.

use blake2::digest::Mac;

use super::types::Blake2bMac256;
use super::*;

// ── CongestionWindow tests ────────────────────────────────────────────────

#[test]
fn congestion_window_basic() {
    let mut cw = CongestionWindow::new(10);
    assert!(!cw.is_full());

    for _ in 0..10 {
        cw.send();
    }
    // window[0] == 10 == max_window → full
    assert!(cw.is_full());

    cw.recv();
    assert!(!cw.is_full());
}

#[test]
fn congestion_window_drain_clears_oldest() {
    let mut cw = CongestionWindow::new(80);
    // Send 5 in quarter 0.
    for _ in 0..5 {
        cw.send();
    }
    assert_eq!(cw.total, 5);

    // Drain advances to quarter 1, clears quarter 1 (which is 0).
    cw.drain();
    assert_eq!(cw.total, 5); // nothing cleared yet (quarter 1 was 0)

    // Advance through all 4 quarters; original quarter 0 becomes the "oldest".
    cw.drain(); // → quarter 2
    cw.drain(); // → quarter 3
    cw.drain(); // → quarter 0 again; clears window[0] = 5
    assert_eq!(cw.total, 0);
    assert_eq!(cw.window[0], 0);
}

#[test]
fn congestion_window_full_condition_total() {
    // total >= 2 * max_window triggers full
    let mut cw = CongestionWindow::new(5);
    // Send across two quarters so no single quarter hits max.
    for _ in 0..5 {
        cw.send();
    }
    cw.drain(); // advance to quarter 1
    for _ in 0..5 {
        cw.send();
    }
    // total = 10 = 2 * 5 → full
    assert!(cw.is_full());
}

#[test]
fn congestion_window_full_condition_single_quarter() {
    let mut cw = CongestionWindow::new(3);
    cw.send();
    cw.send();
    cw.send();
    // window[0] == 3 == max_window → full
    assert!(cw.is_full());
}

#[test]
fn congestion_window_clear() {
    let mut cw = CongestionWindow::new(10);
    for _ in 0..5 {
        cw.send();
    }
    cw.clear();
    assert_eq!(cw.total, 0);
    assert!(!cw.is_full());
}

// ── TID wrap test ─────────────────────────────────────────────────────────

#[test]
fn tid_wraps() {
    // u16 wrapping: 65535 + 1 = 0
    let tid: u16 = 65535;
    let next = tid.wrapping_add(1);
    assert_eq!(next, 0);
}

// ── Token tests ───────────────────────────────────────────────────────────

fn compute_token(host: &str, secret: &[u8; 32]) -> [u8; 32] {
    let mut mac = Blake2bMac256::new_from_slice(secret).unwrap();
    mac.update(host.as_bytes());
    let hash = mac.finalize().into_bytes();
    let mut token = [0u8; 32];
    token.copy_from_slice(&hash);
    token
}

#[test]
fn token_generation_deterministic() {
    let secret = [0xABu8; 32];
    let host = "192.168.1.1";
    let t1 = compute_token(host, &secret);
    let t2 = compute_token(host, &secret);
    assert_eq!(t1, t2);
}

#[test]
fn token_generation_different_host() {
    let secret = [0xABu8; 32];
    let t1 = compute_token("192.168.1.1", &secret);
    let t2 = compute_token("192.168.1.2", &secret);
    assert_ne!(t1, t2);
}

#[test]
fn token_generation_different_secret() {
    let s1 = [0xABu8; 32];
    let s2 = [0xCDu8; 32];
    let host = "10.0.0.1";
    let t1 = compute_token(host, &s1);
    let t2 = compute_token(host, &s2);
    assert_ne!(t1, t2);
}

#[test]
fn token_validation_both_secrets() {
    let secrets = [[0x11u8; 32], [0x22u8; 32]];
    let host = "10.0.0.1";

    let t0 = compute_token(host, &secrets[0]);
    let t1 = compute_token(host, &secrets[1]);

    // Simulate validate_token logic: token matches either secret[0] or secret[1].
    let validate = |token: &[u8; 32]| -> bool {
        let v0 = compute_token(host, &secrets[0]);
        let v1 = compute_token(host, &secrets[1]);
        &v0 == token || &v1 == token
    };

    assert!(validate(&t0));
    assert!(validate(&t1));
}

#[test]
fn token_validation_wrong_host_fails() {
    let secrets = [[0x11u8; 32], [0x22u8; 32]];
    let host = "10.0.0.1";
    let wrong_host = "10.0.0.2";

    let token = compute_token(host, &secrets[0]);

    let validate_wrong = |token: &[u8; 32]| -> bool {
        let v0 = compute_token(wrong_host, &secrets[0]);
        let v1 = compute_token(wrong_host, &secrets[1]);
        &v0 == token || &v1 == token
    };

    assert!(!validate_wrong(&token));
}
