// Tests of the option mapping: holesail `ServerOpts`/`ConnectOpts` → fork options (`relay_through` and the fields next to it).
// Pure functions, no network.
use crate::dht::fork_connect_opts;
use crate::server::fork_opts;
use peeroxide_dht::hyperdht::RelayThrough;
use townjump_dht::{ConnectOpts, ServerOpts};

const KEY: [u8; 32] = [0x42; 32];

mod server {
    use super::*;

    #[test]
    fn fork_opts_default_has_no_relay() {
        let fork = fork_opts(ServerOpts::default());
        assert!(fork.relay_through.is_none());
    }

    #[test]
    fn fork_opts_maps_key() {
        let opts = ServerOpts {
            relay_through: Some(KEY),
            reusable_socket: true,
            ..Default::default()
        };
        let fork = fork_opts(opts);
        assert!(matches!(fork.relay_through, Some(RelayThrough::Key(k)) if k == KEY));
        assert!(fork.reusable_socket);
    }
}

mod connect {
    use super::*;

    #[test]
    fn fork_connect_opts_default_has_no_relay() {
        let fork = fork_connect_opts(ConnectOpts::default());
        assert!(fork.relay_through.is_none());
    }

    #[test]
    fn fork_connect_opts_maps_key() {
        let opts = ConnectOpts {
            relay_through: Some(KEY),
            reusable_socket: true,
        };
        let fork = fork_connect_opts(opts);
        assert!(matches!(fork.relay_through, Some(RelayThrough::Key(k)) if k == KEY));
        assert!(fork.reusable_socket);
    }
}
