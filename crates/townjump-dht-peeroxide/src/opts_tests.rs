// Tests of the option mapping: holesail `ServerOpts`/`ConnectOpts` → fork options (relay fields and the levers next to them).
// Pure functions, no network.
use crate::node::fork_pair;
use crate::opts_map::{fork_connect_opts, fork_opts};
use peeroxide_dht::hyperdht::RelayThrough as ForkThrough;
use std::net::SocketAddrV4;
use std::sync::Arc;
use std::time::Duration;
use townjump_dht::{ConnectOpts, RelayAddress, RelayThrough, ServerOpts};
use townjump_keys::KeyPair;

const KEY: [u8; 32] = [0x42; 32];

fn node_key() -> KeyPair {
    townjump_keys::key_pair_from_seed(&[9; 32])
}

fn relay_address() -> RelayAddress {
    RelayAddress {
        public_key: KEY,
        addr: "127.0.0.1:4100".parse::<SocketAddrV4>().unwrap(),
    }
}

mod server {
    use super::*;

    #[test]
    fn default_has_no_relay_but_carries_the_node_key() {
        let key = node_key();
        let fork = fork_opts(ServerOpts::default(), fork_pair(&key));

        assert!(fork.relay_through.is_none());
        assert!(fork.relay_through_addresses.is_empty());
        let pair = fork.relay_key_pair.expect("node key is always set");
        assert_eq!(pair.public_key, key.public_key);
    }

    #[test]
    fn maps_relay_fields_and_the_node_key() {
        let opts = ServerOpts {
            relay_through: Some(RelayThrough::Key(KEY)),
            relay_addresses: vec![relay_address()],
            reusable_socket: true,
            holepunch: Some(Arc::new(|_| true)),
            relay_keep_alive: Some(Duration::from_secs(3)),
            ..Default::default()
        };
        let key = node_key();

        let fork = fork_opts(opts, fork_pair(&key));

        assert!(matches!(fork.relay_through, Some(ForkThrough::Key(k)) if k == KEY));
        assert_eq!(fork.relay_through_addresses.len(), 1);
        assert_eq!(fork.relay_through_addresses[0].public_key, KEY);
        assert_eq!(fork.relay_through_addresses[0].addr.host, "127.0.0.1");
        assert_eq!(fork.relay_through_addresses[0].addr.port, 4100);
        assert!(fork.reusable_socket);
        assert!(fork.holepunch.is_some());
        assert_eq!(fork.relay_keep_alive, Duration::from_secs(3));
        assert_eq!(fork.relay_key_pair.unwrap().public_key, key.public_key);
    }
}

mod connect {
    use super::*;

    #[test]
    fn default_has_no_relay_and_keeps_fork_levers() {
        let fork = fork_connect_opts(ConnectOpts::default());

        assert!(fork.relay_through.is_none());
        assert!(fork.relay_through_addresses.is_empty());
        assert!(fork.local_connection);
        assert!(fork.holepunch.is_none());
        assert_eq!(fork.punch_delay, None);
    }

    #[test]
    fn maps_a_key() {
        let opts = ConnectOpts {
            relay_through: Some(KEY.into()),
            reusable_socket: true,
            ..Default::default()
        };

        let fork = fork_connect_opts(opts);

        assert!(matches!(fork.relay_through, Some(ForkThrough::Key(k)) if k == KEY));
        assert!(fork.reusable_socket);
    }

    #[test]
    fn maps_every_new_field() {
        let opts = ConnectOpts {
            relay_through: Some(RelayThrough::List(vec![KEY, [1; 32]])),
            relay_addresses: vec![relay_address()],
            local_connection: false,
            holepunch: Some(Arc::new(|_| false)),
            relay_keep_alive: Some(Duration::from_secs(2)),
            punch_delay: Some(Duration::from_millis(300)),
            ..Default::default()
        };

        let fork = fork_connect_opts(opts);

        assert!(matches!(&fork.relay_through, Some(ForkThrough::List(l)) if l == &[KEY, [1; 32]]));
        assert_eq!(fork.relay_through_addresses.len(), 1);
        assert_eq!(fork.relay_through_addresses[0].addr.port, 4100);
        assert!(!fork.local_connection);
        assert!(fork.holepunch.is_some());
        assert_eq!(fork.relay_keep_alive, Duration::from_secs(2));
        assert_eq!(fork.punch_delay, Some(Duration::from_millis(300)));
    }
}
