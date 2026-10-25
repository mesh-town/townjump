// JS: hyperdht/lib/server.js
//! Who opens the UDX path after the Noise reply (JS `_addHandshake`, lines 412-425).

use crate::hyperdht::is_addr_private;
use crate::hyperdht_messages::FIREWALL_OPEN;
use crate::messages::Ipv4Peer;

/// How the server gets a connection out of a handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Plan {
    /// The client is reachable (or asked directly): the server sends first.
    DialClient,
    /// The server is reachable (open firewall): wait for the client's first packet.
    AwaitClient,
    /// Both sides are behind NAT: run the PEER_HOLEPUNCH rounds.
    Holepunch,
}

/// Chooses the plan from the client's firewall, `direct` (no relay in the
/// request) and our own firewall. A client behind our own public IP is hole-punched like any
/// other firewalled client; it reaches the puncher over the LAN.
pub(super) fn plan(remote_fw: u64, direct: bool, ours: u64) -> Plan {
    if remote_fw == FIREWALL_OPEN || direct {
        return Plan::DialClient;
    }
    if ours == FIREWALL_OPEN {
        return Plan::AwaitClient;
    }
    Plan::Holepunch
}

/// Whether a relayed client sits behind our own public IP (JS `server.js:440-445`): not `direct`,
/// our `remote_address()` host equals the client's host as the relay sees it, the client announced
/// that host itself (`client_agrees`) and announced a private address with another host
/// (`client_lan`). Hosts are compared as strings, ports are ignored.
pub(super) fn same_host(
    ours: Option<&Ipv4Peer>,
    client: &Ipv4Peer,
    client_addrs: &[Ipv4Peer],
    direct: bool,
) -> bool {
    !direct
        && ours.is_some_and(|o| o.host == client.host)
        && client_agrees(client, client_addrs)
        && client_lan(client, client_addrs)
}

/// The client announced an address with its own host. Stands in for JS `req.to`, which the Rust
/// server does not get.
pub(super) fn client_agrees(client: &Ipv4Peer, client_addrs: &[Ipv4Peer]) -> bool {
    client_addrs.iter().any(|a| a.host == client.host)
}

/// The client announced a private address other than its own host. Loopback counts as private, so
/// a lone reflexive `127.0.0.1` is not LAN.
pub(super) fn client_lan(client: &Ipv4Peer, client_addrs: &[Ipv4Peer]) -> bool {
    client_addrs
        .iter()
        .any(|a| is_addr_private(&a.host) && a.host != client.host)
}

/// The socket the server dials the client from: a direct request is answered on the socket it
/// came in on (`hs.onsocket(direct ? socket : dht.socket)` in JS), other plans use the primary one.
pub(super) fn dial_from_arrival(plan: Plan, direct: bool) -> bool {
    plan == Plan::DialClient && direct
}

/// Whether the answer to a relayed handshake leaves from the hole-punch socket (JS `replyFromPuncher`).
pub(super) fn reply_from_puncher(plan: Plan) -> bool {
    plan == Plan::Holepunch
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
