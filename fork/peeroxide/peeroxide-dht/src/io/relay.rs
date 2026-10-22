// JS: dht-rpc/lib/io.js
//! `impl Io`: relaying a request to another node.
//! Neighbours: `requests` sends the datagram.

use std::net::SocketAddr;
use std::sync::atomic::Ordering;

use libudx::UdxSocket;

use crate::messages::{self, Ipv4Peer};
use crate::peer::NodeId;

use super::state::Io;
use super::types::SocketKind;

impl Io {
    /// Send a fire-and-forget relay request (no inflight tracking, no response).
    /// Used by the Router to forward PEER_HANDSHAKE/PEER_HOLEPUNCH messages
    /// to relay targets.
    ///
    /// When `preserve_tid` is `Some(t)`, the outgoing packet uses tid `t`
    /// instead of allocating a fresh one. This is required for relay-back
    /// semantics that mirror Node `dht-rpc::Request.relay`: the inbound
    /// request's tid is propagated end-to-end so the eventual REPLY matches
    /// the original requester's inflight entry. Without tid preservation,
    /// relayed responses are silently dropped at intermediate hops.
    pub fn relay(
        &mut self,
        command: u64,
        target: Option<NodeId>,
        value: Option<Vec<u8>>,
        to: &Ipv4Peer,
        preserve_tid: Option<u16>,
        via: Option<&UdxSocket>,
    ) -> bool {
        if self.closed() {
            return false;
        }

        let addr: SocketAddr = match format!("{}:{}", to.host, to.port).parse() {
            Ok(a) => a,
            Err(_) => return false,
        };

        let tid = match preserve_tid {
            Some(t) => t,
            None => {
                let t = self.tid;
                self.tid = self.tid.wrapping_add(1);
                t
            }
        };

        let socket_kind = if self.firewalled {
            SocketKind::Client
        } else {
            SocketKind::Server
        };

        // JS `dht-rpc` writes the node id only when the packet leaves the server socket.
        let include_id = via.is_none() && !self.ephemeral && socket_kind == SocketKind::Server;
        let id = if include_id {
            self.table.lock().ok().map(|t| *t.id())
        } else {
            None
        };

        let request = messages::Request {
            tid,
            to: to.clone(),
            id,
            token: None,
            internal: false,
            command,
            target,
            value,
        };

        let buffer = match messages::encode_request_to_bytes(&request) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(err = %e, "relay: encode failed");
                return false;
            }
        };

        let socket = via.unwrap_or(match socket_kind {
            SocketKind::Client => &self.client_socket,
            SocketKind::Server => &self.server_socket,
        });

        let buffer_len = buffer.len() as u64;
        if let Err(e) = socket.send_to(&buffer, addr) {
            tracing::warn!(err = %e, "relay: send_to failed");
            return false;
        }
        self.wire
            .bytes_sent
            .fetch_add(buffer_len, Ordering::Relaxed);

        true
    }
}
