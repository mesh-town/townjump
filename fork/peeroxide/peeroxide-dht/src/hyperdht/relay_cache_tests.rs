use super::RelayCache;
use crate::messages::Ipv4Peer;

fn key(n: u32) -> [u8; 32] {
    let mut k = [0u8; 32];
    k[..4].copy_from_slice(&n.to_be_bytes());
    k
}

fn peers(port: u16) -> Vec<Ipv4Peer> {
    vec![Ipv4Peer {
        host: "10.0.0.1".into(),
        port,
    }]
}

#[test]
fn empty_get_is_empty() {
    assert!(RelayCache::default().get(&key(1)).is_empty());
}

#[test]
fn set_replaces_and_get_clones() {
    let mut c = RelayCache::default();
    c.set(key(1), peers(1));
    c.set(key(1), peers(2));
    assert_eq!(c.get(&key(1)), peers(2));
    assert_eq!(c.get(&key(1)), peers(2));
}

#[test]
fn evicts_oldest_on_513th() {
    let mut c = RelayCache::default();
    for n in 0..512 {
        c.set(key(n), peers(1));
    }
    assert_eq!(c.get(&key(0)), peers(1));
    c.set(key(512), peers(1));
    assert!(c.get(&key(0)).is_empty());
    assert_eq!(c.get(&key(1)), peers(1));
    assert_eq!(c.get(&key(512)), peers(1));
}

#[test]
fn replace_refreshes_age() {
    let mut c = RelayCache::default();
    for n in 0..512 {
        c.set(key(n), peers(1));
    }
    c.set(key(0), peers(2));
    c.set(key(512), peers(1));
    assert_eq!(c.get(&key(0)), peers(2));
    assert!(c.get(&key(1)).is_empty());
}
