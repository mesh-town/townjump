// JS: `connect` and `createServer` options reaching the fork: relay choice, relay addresses, holepunch filter, keep-alive.
// See also: dht.rs (`connect`), server.rs (`create`); opts_tests.rs.
use crate::nodes::{to_addrs, to_peers};
use peeroxide_dht::hyperdht::{
    ConnectOpts as ForkConnectOpts, HolepunchFilter as ForkFilter, KeyPair as ForkKeyPair,
    RelayAddress as ForkRelayAddress, RelayThrough as ForkRelayThrough,
};
use peeroxide_dht::messages::Ipv4Peer;
use peeroxide_dht::server::{Firewall as ForkFirewall, ServerOpts as ForkOpts};
use std::net::SocketAddrV4;
use std::sync::Arc;
use townjump_dht::{
    ConnectOpts, HolepunchFilter, HolepunchInfo, RelayAddress, RelayThrough, ServerOpts,
};

fn fork_through(through: RelayThrough) -> ForkRelayThrough {
    match through {
        RelayThrough::Key(key) => ForkRelayThrough::Key(key),
        RelayThrough::List(keys) => ForkRelayThrough::List(keys),
    }
}

fn fork_addresses(list: Vec<RelayAddress>) -> Vec<ForkRelayAddress> {
    list.into_iter()
        .map(|a| {
            let addr: Ipv4Peer = to_peers(&[a.addr]).remove(0);
            ForkRelayAddress {
                public_key: a.public_key,
                addr,
            }
        })
        .collect()
}

fn addrs(peers: &[Ipv4Peer]) -> Vec<SocketAddrV4> {
    to_addrs(peers.to_vec())
}

fn fork_filter(filter: HolepunchFilter) -> ForkFilter {
    ForkFilter::new(move |input| {
        filter(&HolepunchInfo {
            remote_firewall: input.remote_firewall,
            local_firewall: input.local_firewall,
            remote_addresses: addrs(input.remote_addresses),
            local_addresses: addrs(input.local_addresses),
        })
    })
}

/// holesail connect options → fork options; unset levers keep the fork defaults.
pub(crate) fn fork_connect_opts(opts: ConnectOpts) -> ForkConnectOpts {
    let mut fork = ForkConnectOpts::default();
    fork.reusable_socket = opts.reusable_socket;
    fork.relay_through = opts.relay_through.map(fork_through);
    fork.relay_through_addresses = fork_addresses(opts.relay_addresses);
    fork.local_connection = opts.local_connection;
    fork.holepunch = opts.holepunch.map(fork_filter);
    if let Some(keep_alive) = opts.relay_keep_alive {
        fork.relay_keep_alive = keep_alive;
    }
    fork.punch_delay = opts.punch_delay;
    fork
}

/// holesail server options → fork options; the callback sees only the client public key, the relay is
/// reached with `relay_key_pair` (the node key, as JS `dht.defaultKeyPair`).
pub(crate) fn fork_opts(opts: ServerOpts, relay_key_pair: ForkKeyPair) -> ForkOpts {
    let mut fork = ForkOpts::default();
    fork.firewall = opts
        .firewall
        .map(|f| -> ForkFirewall { Arc::new(move |input| f(input.remote_public_key)) });
    fork.reusable_socket = opts.reusable_socket;
    fork.relay_through = opts.relay_through.map(fork_through);
    fork.relay_through_addresses = fork_addresses(opts.relay_addresses);
    fork.holepunch = opts.holepunch.map(fork_filter);
    if let Some(keep_alive) = opts.relay_keep_alive {
        fork.relay_keep_alive = keep_alive;
    }
    fork.relay_key_pair = Some(relay_key_pair);
    fork
}
