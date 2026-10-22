// JS: dht-rpc/lib/io.js
//! `impl Io`: decoding and dispatching one incoming datagram.
//! Neighbours: `event_loop` feeds it; `replies` and `tokens` serve its branches.

use std::net::SocketAddr;

use libudx::Datagram;

use crate::messages::{self, Ipv4Peer};
use crate::peer;

use super::state::Io;
use super::types::{
    IncomingRequest, IoEvent, ReplyContext, ReplyInternalParams, ResolvedRequest, SocketKind,
};

const ERROR_INVALID_TOKEN: u64 = 2;

impl Io {
    /// Decode and dispatch a datagram from either socket.
    pub(super) fn process_datagram(
        &mut self,
        datagram: Datagram,
        socket_kind: SocketKind,
    ) -> Option<IoEvent> {
        self.process_arrival(
            datagram,
            ReplyContext {
                socket_kind,
                via: None,
            },
        )
    }

    /// Decode and dispatch a datagram; `ctx` says which socket it arrived on (replies go back there).
    pub(super) fn process_arrival(
        &mut self,
        datagram: Datagram,
        ctx: ReplyContext,
    ) -> Option<IoEvent> {
        if self.suspended || datagram.data.len() < 2 {
            return None;
        }

        let (host, port) = match datagram.addr {
            SocketAddr::V4(v4) => (v4.ip().to_string(), v4.port()),
            SocketAddr::V6(_) => return None,
        };

        if port == 0 {
            return None;
        }

        let from = Ipv4Peer { host, port };

        match messages::decode_message(&datagram.data) {
            Err(ref e) => {
                tracing::debug!(
                    from = %format!("{}:{}", from.host, from.port),
                    len = datagram.data.len(),
                    first_byte = datagram.data.first().copied().unwrap_or(0),
                    err = %e,
                    "process_datagram: decode failed"
                );
                None
            }
            Ok(messages::Message::Request(req)) => {
                // Validate token if present.
                if req.token.is_some() {
                    let token_val = req
                        .token
                        .as_ref()
                        .map(|t| self.validate_token(&from.host, t));
                    if token_val == Some(false) {
                        let tid = req.tid;
                        let target = req.target;
                        let from_clone = from.clone();
                        self.send_reply_internal(
                            &from_clone,
                            ReplyInternalParams {
                                socket_kind: ctx.socket_kind,
                                tid,
                                target,
                                error: ERROR_INVALID_TOKEN,
                                include_token: true,
                                value: None,
                                via: ctx.via,
                            },
                        );
                        return None;
                    }
                }

                // Validate incoming ID.
                let validated_id = req.id.and_then(|id| {
                    let expected = peer::peer_id(&from.host, from.port);
                    if expected == id { Some(expected) } else { None }
                });

                Some(IoEvent::IncomingRequest(IncomingRequest {
                    tid: req.tid,
                    from: from.clone(),
                    to: req.to,
                    id: validated_id,
                    token: req.token,
                    internal: req.internal,
                    command: req.command,
                    target: req.target,
                    value: req.value,
                    reply_ctx: ctx,
                }))
            }

            Ok(messages::Message::Response(res)) => {
                // Find the matching inflight entry by TID.
                let pos = match self.inflight.iter().position(|e| e.tid == res.tid) {
                    Some(p) => p,
                    None => {
                        tracing::debug!(
                            tid = res.tid,
                            from = %format!("{}:{}", from.host, from.port),
                            "response TID not found in inflight"
                        );
                        return None;
                    }
                };
                let entry = self.inflight.swap_remove(pos);

                let rtt = entry.timestamp.elapsed();

                self.congestion.recv();
                self.stats.active = self.stats.active.saturating_sub(1);
                self.stats.responses += 1;

                // Validate incoming ID.
                let validated_id = res.id.and_then(|id| {
                    let expected = peer::peer_id(&from.host, from.port);
                    if expected == id { Some(expected) } else { None }
                });

                // A reply that came through an external socket says nothing about our address
                // as seen from the NAT of the IO sockets: `to` stays empty (JS `external`).
                let res_to = if ctx.via.is_some() {
                    Ipv4Peer::default()
                } else {
                    res.to
                };

                let request = ResolvedRequest {
                    tid: entry.tid,
                    to: entry.to,
                    command: entry.command,
                    internal: entry.internal,
                    target: entry.target,
                };

                Some(IoEvent::Response {
                    tid: res.tid,
                    from,
                    to: res_to,
                    id: validated_id,
                    token: res.token,
                    closer_nodes: res.closer_nodes,
                    error: res.error,
                    value: res.value,
                    rtt,
                    request,
                    server_arrival: ctx.via.is_none() && ctx.socket_kind == SocketKind::Server,
                })
            }
        }
    }
}
