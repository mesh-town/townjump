//! The pure decisions of the relay's control-connection server: which punches it refuses, which relays
//! it names in a reply, and the options it hands to the server.
// JS: hyperdht/lib/server.js

use std::sync::Arc;

use super::*;
use crate::hyperdht::{HolepunchFilter, HolepunchInput};
use crate::hyperdht_messages::{
    FIREWALL_CONSISTENT, FIREWALL_OPEN, FIREWALL_RANDOM, FIREWALL_UNKNOWN,
};
use crate::messages::Ipv4Peer;
use crate::relay_service::RelayServiceConfig;
use crate::server::Firewall;

mod holepunch_filter {
    use super::*;

    #[test]
    fn refuses_a_relay_behind_a_random_nat() {
        // given: the relay's own NAT is random
        let filter = relay_holepunch_filter();

        // when: the filter is asked
        let allowed = allows(&filter, FIREWALL_RANDOM);

        // then: the punch is vetoed
        assert!(!allowed, "no port can be predicted behind a random NAT");
    }

    #[test]
    fn allows_consistent_and_unknown() {
        // given: the filter of the relay service
        let filter = relay_holepunch_filter();

        // when / then: a consistent and an unknown NAT both let the punch run
        assert!(allows(&filter, FIREWALL_CONSISTENT), "consistent NAT");
        assert!(allows(&filter, FIREWALL_UNKNOWN), "unknown NAT");
    }

    #[test]
    fn allows_an_open_relay() {
        let filter = relay_holepunch_filter();

        assert!(allows(&filter, FIREWALL_OPEN), "open node");
    }
}

mod server_opts_of_the_service {
    use super::*;

    #[test]
    fn server_opts_carry_the_peer_firewall_and_a_reusable_socket() {
        // given: a service config with a peer firewall
        let refuse_all: Firewall = Arc::new(|_| true);
        let config = RelayServiceConfig {
            peer_firewall: Some(refuse_all),
            ..RelayServiceConfig::default()
        };

        // when: the options of the control-connection server are built
        let opts = server_opts(&config);

        // then: the firewall passes through, the socket is reusable, a holepunch filter is set
        assert!(opts.firewall.is_some(), "the peer firewall is passed on");
        assert!(opts.reusable_socket, "the pair's socket is kept");
        assert!(opts.holepunch.is_some(), "a holepunch filter is installed");
    }

    #[test]
    fn without_a_peer_firewall_the_server_has_none() {
        let opts = server_opts(&RelayServiceConfig::default());

        assert!(opts.firewall.is_none(), "everyone is admitted");
    }

    #[test]
    fn the_installed_filter_vetoes_a_random_nat() {
        let opts = server_opts(&RelayServiceConfig::default());
        let filter = opts.holepunch.expect("a holepunch filter");

        assert!(!allows(&filter, FIREWALL_RANDOM));
        assert!(allows(&filter, FIREWALL_CONSISTENT));
    }
}

fn address(host_byte: u8, port: u16) -> Ipv4Peer {
    Ipv4Peer {
        host: format!("198.51.100.{host_byte}"),
        port,
    }
}

/// What the filter says when the relay's own firewall is `local_firewall`.
fn allows(filter: &HolepunchFilter, local_firewall: u64) -> bool {
    let remote = [address(50, 6000)];
    let local = [address(60, 7000)];
    filter.allows(&HolepunchInput {
        remote_firewall: FIREWALL_UNKNOWN,
        local_firewall,
        remote_addresses: &remote,
        local_addresses: &local,
    })
}
