// Tests of `relay_opts`: relay options and their translation into fork configs. Fast, no network.
use super::*;
use peeroxide_dht::hyperdht_messages::{FIREWALL_UNKNOWN, NoisePayload};
use peeroxide_dht::server::FirewallInput;

mod default_opts {
    use super::*;

    #[test]
    fn keep_the_old_behaviour() {
        let opts = RelayOpts::default();
        assert!(opts.identity.is_none(), "random key");
        assert!(opts.firewall.is_none(), "everyone admitted");
        assert!(opts.announce);
        assert_eq!(opts.port, 0);
        assert!(opts.assume_open);
    }
}

mod nat_kind {
    use super::*;

    #[test]
    fn follows_the_fork_constants() {
        assert_eq!(NatKind::from(FIREWALL_UNKNOWN), NatKind::Unknown);
        assert_eq!(NatKind::from(FIREWALL_OPEN), NatKind::Open);
        assert_eq!(NatKind::from(FIREWALL_CONSISTENT), NatKind::Consistent);
        assert_eq!(NatKind::from(FIREWALL_RANDOM), NatKind::Random);
    }

    #[test]
    fn unknown_value_is_unknown() {
        assert_eq!(NatKind::from(99), NatKind::Unknown);
    }
}

mod node_config {
    use super::*;

    #[test]
    fn assumed_open_is_persistent_and_open() {
        let opts = RelayOpts {
            port: 4242,
            assume_open: true,
            ..RelayOpts::default()
        };
        let config = relay_node_config(HyperDhtConfig::default(), &opts);
        assert_eq!(config.dht.ephemeral, Some(false));
        assert!(!config.dht.firewalled);
        assert_eq!(config.dht.port, 4242, "the port is carried over");
    }

    #[test]
    fn checked_is_adaptive_and_firewalled() {
        let mut base = HyperDhtConfig::default();
        base.dht.adaptive = false;
        let opts = RelayOpts {
            assume_open: false,
            ..RelayOpts::default()
        };
        let config = relay_node_config(base, &opts);
        assert_eq!(config.dht.ephemeral, None, "left to the reachability round");
        assert!(config.dht.adaptive);
        assert!(config.dht.firewalled);
        assert_eq!(config.dht.stable_ticks, RELAY_STABLE_TICKS);
    }
}

mod service_config_of {
    use super::*;

    #[test]
    fn carries_announce_off() {
        let opts = RelayOpts {
            announce: false,
            ..RelayOpts::default()
        };
        assert!(!service_config(&opts).announce);
    }

    #[test]
    fn without_a_callback_has_no_peer_firewall() {
        let config = service_config(&RelayOpts::default());
        assert!(config.peer_firewall.is_none());
        assert!(config.announce, "announce stays on by default");
    }

    #[test]
    fn firewall_refuses_the_listed_key_and_admits_another() {
        let refused = [7u8; 32];
        let admitted = [8u8; 32];
        let callback: Firewall = Arc::new(move |key| *key == refused);
        let opts = RelayOpts {
            firewall: Some(callback),
            ..RelayOpts::default()
        };
        let fork = service_config(&opts).peer_firewall.expect("peer firewall");
        assert!(check(&fork, &refused), "the refused key is refused");
        assert!(!check(&fork, &admitted), "another key is admitted");
    }
}

mod publish_address_of {
    use super::*;

    #[test]
    fn not_open_node_has_none() {
        for kind in [NatKind::Unknown, NatKind::Consistent, NatKind::Random] {
            assert_eq!(
                publish_address(kind, peer("1.2.3.4", 5), peer("1.2.3.4", 5)),
                None,
                "{kind:?}"
            );
            assert_eq!(
                publish_address(kind, None, peer("1.2.3.4", 5)),
                None,
                "{kind:?} observed only"
            );
        }
    }

    #[test]
    fn open_node_prefers_the_declared_address() {
        let got = publish_address(NatKind::Open, peer("1.2.3.4", 5), peer("5.6.7.8", 9));
        assert_eq!(got, Some("1.2.3.4:5".parse().unwrap()));
    }

    #[test]
    fn open_node_without_declared_uses_the_observed_address() {
        let got = publish_address(NatKind::Open, None, peer("5.6.7.8", 9));
        assert_eq!(got, Some("5.6.7.8:9".parse().unwrap()));
    }

    #[test]
    fn ipv6_or_garbage_host_has_none() {
        assert_eq!(
            publish_address(NatKind::Open, peer("::1", 5), None),
            None,
            "ipv6"
        );
        assert_eq!(
            publish_address(NatKind::Open, peer("not-an-ip", 5), None),
            None,
            "garbage"
        );
    }

    #[test]
    fn open_node_without_any_address_has_none() {
        assert_eq!(publish_address(NatKind::Open, None, None), None);
    }
}

fn peer(host: &str, port: u16) -> Option<Ipv4Peer> {
    Some(Ipv4Peer {
        host: host.to_string(),
        port,
    })
}

/// Does the fork firewall refuse a handshake from `key`?
fn check(fork: &ForkFirewall, key: &[u8; 32]) -> bool {
    let payload = NoisePayload {
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
    };
    let address = Ipv4Peer {
        host: "10.0.0.1".to_string(),
        port: 4000,
    };
    let input = FirewallInput {
        remote_public_key: key,
        remote_payload: &payload,
        client_address: &address,
    };
    fork(&input)
}
