// JS: dht-rpc/index.js (opts.nodes, toArray) through the fork
// See also: factory.rs (seed nodes into the config), dht.rs (`Dht::to_array`).
use crate::error::map_error;
use crate::node::Node;
use peeroxide_dht::messages::Ipv4Peer;
use std::net::SocketAddrV4;
use townjump_dht::DhtError;

/// Seed addresses → fork peers, in the same order.
pub(crate) fn to_peers(nodes: &[SocketAddrV4]) -> Vec<Ipv4Peer> {
    nodes
        .iter()
        .map(|n| Ipv4Peer {
            host: n.ip().to_string(),
            port: n.port(),
        })
        .collect()
}

/// Fork peers → addresses; entries that are not dotted IPv4 (hostnames, IPv6) are dropped.
pub(crate) fn to_addrs(peers: Vec<Ipv4Peer>) -> Vec<SocketAddrV4> {
    peers
        .into_iter()
        .filter_map(|p| Some(SocketAddrV4::new(p.host.parse().ok()?, p.port)))
        .collect()
}

impl Node {
    /// Routing table export (dht-rpc `toArray`): freshest first.
    pub async fn to_array(&self, limit: Option<usize>) -> Result<Vec<SocketAddrV4>, DhtError> {
        self.check_alive()?;
        let peers = self.handle.to_array(limit).await.map_err(map_error)?;
        Ok(to_addrs(peers))
    }
}

#[cfg(test)]
#[path = "nodes_tests.rs"]
mod tests;
