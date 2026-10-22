//! Tests for the address functions of [`super::Holepuncher`].

use super::*;
use crate::hyperdht_messages::FIREWALL_CONSISTENT;
use crate::socket_pool::SocketPool;
use libudx::UdxRuntime;
use tokio::sync::mpsc;

#[test]
fn local_addresses_assigns_input_port_to_every_entry() {
    let addrs = Holepuncher::local_addresses(49737);
    assert!(!addrs.is_empty(), "must return at least loopback fallback");
    for a in &addrs {
        assert_eq!(a.port, 49737, "every entry uses the supplied port");
    }
}

#[test]
fn local_addresses_excludes_loopback_when_other_ifaces_exist() {
    // On any host with a real network interface (CI runners, dev boxes),
    // the result must NOT contain only loopback. If only loopback is
    // returned, the host literally has no non-internal IPv4 interface
    // and the fallback path was taken — that's allowed but documented.
    let addrs = Holepuncher::local_addresses(1234);
    let has_real = addrs.iter().any(|a| a.host != "127.0.0.1");
    let only_loopback = !has_real;
    // We can't *force* a real interface to exist in CI, but we can at
    // least assert the function emits non-loopback when one is present.
    // If only_loopback, document that path was taken (test still passes).
    if only_loopback {
        assert_eq!(
            addrs.len(),
            1,
            "loopback fallback must be exactly one entry"
        );
        assert_eq!(addrs[0].host, "127.0.0.1");
    } else {
        assert!(
            addrs.iter().all(|a| a.host != "127.0.0.1"),
            "when real interfaces exist, loopback must be excluded entirely"
        );
    }
}

#[test]
fn local_addresses_returns_only_ipv4_strings() {
    let addrs = Holepuncher::local_addresses(0);
    for a in &addrs {
        let parsed: std::net::Ipv4Addr = a.host.parse().expect("must parse as IPv4");
        assert_eq!(parsed.to_string(), a.host, "canonical form expected");
    }
}

#[test]
fn local_addresses_fallback_is_loopback_zero_port_ok() {
    // The fallback path uses port=0 if requested; verify shape.
    let addrs = Holepuncher::local_addresses(0);
    assert!(!addrs.is_empty());
    for a in &addrs {
        assert_eq!(a.port, 0);
    }
}

fn reflexive_samples() -> Vec<Ipv4Peer> {
    vec![
        Ipv4Peer {
            host: "47.197.162.13".into(),
            port: 49737,
        },
        Ipv4Peer {
            host: "47.197.162.13".into(),
            port: 49738,
        },
    ]
}

/// Holepunch payload `addresses`: only NAT samples (JS `p.nat.addresses`),
/// never interface addresses (VPN tunnels, RFC1918, loopback).
mod holepunch_payload {
    use super::*;

    mod with_reflexive_samples {
        use super::*;

        #[test]
        fn carries_only_the_samples() {
            let nat = reflexive_samples();
            let result = Holepuncher::select_punch_addresses(Some(&nat));
            assert_eq!(result, nat, "payload is exactly the NAT samples");
        }
    }

    mod without_samples {
        use super::*;

        #[test]
        fn is_empty_when_nat_is_none() {
            let result = Holepuncher::select_punch_addresses(None);
            assert_eq!(
                result,
                Vec::<Ipv4Peer>::new(),
                "None nat.addresses -> empty"
            );
        }

        #[test]
        fn is_empty_when_nat_is_empty() {
            let empty: Vec<Ipv4Peer> = Vec::new();
            let result = Holepuncher::select_punch_addresses(Some(&empty));
            assert_eq!(
                result,
                Vec::<Ipv4Peer>::new(),
                "empty nat.addresses -> empty"
            );
        }
    }

    mod from_the_puzzle {
        use super::*;

        #[tokio::test]
        async fn reads_nat_addresses_only() {
            let runtime = UdxRuntime::new().expect("runtime");
            let pool = SocketPool::new("127.0.0.1".to_string());
            let (event_tx, _event_rx) = mpsc::unbounded_channel();
            let mut puncher =
                Holepuncher::new(&pool, &runtime, true, true, FIREWALL_CONSISTENT, event_tx)
                    .await
                    .expect("puncher");

            assert_eq!(
                puncher.punch_addresses(),
                Vec::<Ipv4Peer>::new(),
                "fresh puzzle has no samples -> empty payload"
            );

            let nat = reflexive_samples();
            puncher.nat.addresses = Some(nat.clone());
            assert_eq!(puncher.punch_addresses(), nat, "payload == nat.addresses");
        }
    }
}

mod handshake_addresses4 {
    use super::*;

    #[test]
    fn puts_samples_first_then_interfaces() {
        let nat_addrs = reflexive_samples();
        let result = Holepuncher::handshake_addresses4(&nat_addrs, 1234);
        let local = Holepuncher::local_addresses(1234);
        // First N entries are reflexive (in order), trailing entries are local.
        assert_eq!(result[..2], nat_addrs[..], "reflexive entries come first");
        assert_eq!(result[2..], local[..], "local entries appended after");
    }

    #[test]
    fn without_samples_is_interfaces_only() {
        let result = Holepuncher::handshake_addresses4(&[], 1234);
        let expected = Holepuncher::local_addresses(1234);
        assert_eq!(result, expected, "no samples -> interfaces only");
    }
}
