// JS: dht-rpc/lib/io.js
//! `impl Io`: handling inbound replies and sending replies (plain, deferred, internal).
//! Neighbours: `datagram` dispatches here; `tokens` issues reply tokens.

use std::net::SocketAddr;
use std::sync::atomic::Ordering;

use libudx::{Datagram, UdxSocket};

use crate::messages::{self, Ipv4Peer, Response};
use crate::peer::NodeId;
use crate::routing_table::K;

use super::state::Io;
use super::types::{IncomingRequest, IoEvent, ReplyContext, ReplyInternalParams, SocketKind};

impl Io {
    /// Inject reply bytes received on a non-`Io`-owned socket (puncher
    /// socket) into the response-matching path. The actor calls this when
    /// a `DhtCommand::InboundReplyBytes` arrives from the puncher-socket
    /// demux task.
    pub fn handle_inbound_reply_bytes(
        &mut self,
        addr: SocketAddr,
        data: Vec<u8>,
    ) -> Option<IoEvent> {
        self.wire
            .bytes_received
            .fetch_add(data.len() as u64, Ordering::Relaxed);
        self.process_datagram(Datagram { data, addr }, SocketKind::Client)
    }

    /// Inject a datagram received on an external (held) socket `via`: a request is answered
    /// out of `via` (no id), a reply is matched by tid. UDX frames are dropped silently, as
    /// `classify_inbound` does: late packets of a closed stream would only add debug noise.
    pub(crate) fn handle_inbound_via(
        &mut self,
        addr: SocketAddr,
        data: Vec<u8>,
        via: UdxSocket,
    ) -> Option<IoEvent> {
        self.wire
            .bytes_received
            .fetch_add(data.len() as u64, Ordering::Relaxed);
        if data.len() >= 20 && data[0] == 0xFF {
            return None;
        }
        let ctx = ReplyContext {
            socket_kind: SocketKind::Client,
            via: Some(via),
        };
        self.process_arrival(Datagram { data, addr }, ctx)
    }

    /// Socket a reply to this request must leave from: the external one it came through, else the IO socket.
    pub(crate) fn arrival_socket(&self, ctx: &ReplyContext) -> UdxSocket {
        match &ctx.via {
            Some(via) => via.clone(),
            None => self.socket_for(ctx.socket_kind),
        }
    }

    /// Send a reply to an incoming request.
    pub fn send_reply(&mut self, req: &IncomingRequest, error: u64, value: Option<&[u8]>) {
        let socket_kind = req.reply_ctx.socket_kind;
        let via = req.reply_ctx.via.clone();
        let include_token = error == 0;
        self.send_reply_internal(
            &req.from,
            ReplyInternalParams {
                socket_kind,
                tid: req.tid,
                target: req.target,
                error,
                include_token,
                value: value.map(|v| v.to_vec()),
                via,
            },
        );
    }

    /// Send a reply using explicit parameters (for deferred/delayed replies).
    /// Unlike `send_reply`, this does not require an `IncomingRequest` reference.
    pub(crate) fn send_reply_deferred(
        &mut self,
        to: &Ipv4Peer,
        ctx: ReplyContext,
        tid: u16,
        target: Option<NodeId>,
        error: u64,
        value: Option<&[u8]>,
    ) {
        let include_token = error == 0;
        self.send_reply_internal(
            to,
            ReplyInternalParams {
                socket_kind: ctx.socket_kind,
                tid,
                target,
                error,
                include_token,
                value: value.map(|v| v.to_vec()),
                via: ctx.via,
            },
        );
    }

    /// Send a fire-and-forget REPLY (Response packet) to a specific address,
    /// using the provided tid. Mirrors Node `dht-rpc::Request.reply(value,
    /// { to })`: the tid is taken from an inbound request whose tid was
    /// preserved end-to-end through a relay chain, so the eventual REPLY
    /// matches the original requester's inflight entry at the chain's
    /// destination.
    ///
    /// Used by the handshake router's FROM_SERVER case (FE-holder finalising
    /// the tid-preserved chain by replying directly to the original client).
    /// Unlike [`Self::send_reply`], the destination address is independent
    /// of any inbound request; unlike the internal `send_reply_deferred`
    /// helper, the socket kind is derived from current firewall state rather
    /// than passed in via a `ReplyContext`.
    pub fn reply_to(
        &mut self,
        tid: u16,
        target: Option<NodeId>,
        to: &Ipv4Peer,
        value: Option<Vec<u8>>,
    ) -> bool {
        if self.closed() {
            return false;
        }

        let socket_kind = if self.firewalled {
            SocketKind::Client
        } else {
            SocketKind::Server
        };

        self.send_reply_internal(
            to,
            ReplyInternalParams {
                socket_kind,
                tid,
                target,
                error: 0,
                include_token: true,
                value,
                via: None,
            },
        );

        true
    }

    /// Encode and send a response message.
    pub(super) fn send_reply_internal(&mut self, to: &Ipv4Peer, params: ReplyInternalParams) {
        if self.closed() {
            return;
        }
        let include_id =
            !self.ephemeral && params.via.is_none() && params.socket_kind == SocketKind::Server;

        let (id, closer_nodes) = match self.table.lock() {
            Ok(table) => {
                let id = if include_id { Some(*table.id()) } else { None };
                let closer_nodes: Vec<Ipv4Peer> = if let Some(t) = &params.target {
                    table
                        .closest(t, K)
                        .into_iter()
                        .map(|n| Ipv4Peer {
                            host: n.host.clone(),
                            port: n.port,
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                (id, closer_nodes)
            }
            Err(_) => (None, Vec::new()),
        };

        let token = if params.include_token {
            Some(self.token(&to.host, 1))
        } else {
            None
        };

        let response = Response {
            tid: params.tid,
            to: to.clone(),
            id,
            token,
            closer_nodes,
            error: params.error,
            value: params.value,
        };

        let bytes = match messages::encode_response_to_bytes(&response) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(err = %e, "send_reply_internal: encode failed");
                return;
            }
        };

        let addr_str = format!("{}:{}", to.host, to.port);
        let addr: SocketAddr = match addr_str.parse() {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!(err = %e, "send_reply_internal: invalid address");
                return;
            }
        };

        let socket = match params.via {
            Some(via) => via,
            None => self.socket_for(params.socket_kind),
        };

        let bytes_len = bytes.len() as u64;
        if let Err(e) = socket.send_to(&bytes, addr) {
            tracing::warn!(err = %e, "send_reply_internal: send_to failed");
        } else {
            self.wire.bytes_sent.fetch_add(bytes_len, Ordering::Relaxed);
        }
    }
}
