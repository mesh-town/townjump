//! `handle_peer_handshake`: the node's answer to PEER_HANDSHAKE (server, relay or forward).
//! Neighbours: `node` dispatches here; `server_run` receives the forwarded handshake.

use std::sync::{Arc, Mutex};

use tokio::sync::{mpsc, oneshot};

use crate::handshake_route::{HandshakeRoute, relay_target};
use crate::hyperdht_messages::PEER_HANDSHAKE;
use crate::router::{HandshakeAction, Router};
use crate::rpc::DhtHandle;

use super::types::ServerEvent;

pub(super) fn handle_peer_handshake(
    mut req: crate::rpc::UserRequest,
    dht: &DhtHandle,
    router: &Arc<Mutex<Router>>,
    server_tx: &mpsc::UnboundedSender<ServerEvent>,
) {
    let Some(value) = &req.value else {
        req.error(1);
        return;
    };

    let action = {
        let router = match router.lock() {
            Ok(r) => r,
            Err(_) => {
                req.error(1);
                return;
            }
        };
        match router.route_handshake(req.target.as_ref(), &req.from, value) {
            Ok(a) => a,
            Err(_) => {
                req.error(1);
                return;
            }
        }
    };

    match action {
        HandshakeAction::Reply(value) => {
            tracing::debug!(from = %format!("{}:{}", req.from.host, req.from.port), "handshake REPLY");
            req.reply(Some(value));
        }
        HandshakeAction::ForwardRequest { value, to } => {
            // FE-holder relay: forward the inbound REQUEST as a new REQUEST
            // to `to`, preserving the original requester's tid end-to-end.
            // Fire-and-forget — the eventual REPLY will reach the original
            // client directly via the tid-preserved chain (server replies
            // with `dht.relay_with_tid` back to this FE-holder, which then
            // dispatches HandshakeAction::ReplyTo to send the REPLY packet
            // straight to the client at peer_address). Mirrors Node
            // `dht-rpc::Request.relay(value, to)`.
            tracing::debug!(
                from = %format!("{}:{}", req.from.host, req.from.port),
                to = %format!("{}:{}", to.host, to.port),
                "handshake FORWARD_REQUEST — tid-preserved relay"
            );
            if let Err(e) =
                dht.relay_with_tid(PEER_HANDSHAKE, req.target, Some(value), &to, req.tid)
            {
                tracing::debug!(err = %e, "ForwardRequest: relay_with_tid send failed");
            }
            // No reply to the inbound REQUEST — the original client awaits
            // its REPLY via the preserved-tid chain, not via this hop.
            req.release();
        }
        HandshakeAction::ReplyTo { value, to } => {
            // FE-holder finalises the relay chain: send a REPLY packet
            // (Response, not Request) with the inbound tid (= original
            // client's tid) directly to the client at `to`. The REPLY
            // matches the client's outstanding inflight entry. Mirrors
            // Node `dht-rpc::Request.reply(value, { to })` as invoked from
            // `lib/server.js _addHandshake case FROM_SERVER`.
            tracing::debug!(
                from = %format!("{}:{}", req.from.host, req.from.port),
                to = %format!("{}:{}", to.host, to.port),
                "handshake REPLY_TO — finalise tid-preserved chain"
            );
            if let Err(e) = dht.send_reply_to(req.tid, req.target, &to, Some(value)) {
                tracing::debug!(err = %e, "ReplyTo: send_reply_to failed");
            }
            // The inbound REQUEST came from the server via its relay_with_tid;
            // the server is not awaiting a reply, so we do not call
            // req.reply / req.error here.
            req.release();
        }
        HandshakeAction::Relay { value, to } => {
            // Legacy variant — the router no longer emits this for handshakes.
            // Retained for forward-compatibility (non_exhaustive enum) so any
            // external producer continues to function. Treat as fire-and-forget
            // tid-preserved relay (equivalent to ForwardRequest).
            tracing::warn!(
                from = %format!("{}:{}", req.from.host, req.from.port),
                to = %format!("{}:{}", to.host, to.port),
                "handshake legacy Relay action — treating as ForwardRequest"
            );
            if let Err(e) =
                dht.relay_with_tid(PEER_HANDSHAKE, req.target, Some(value), &to, req.tid)
            {
                tracing::debug!(err = %e, "legacy Relay: relay_with_tid send failed");
            }
            req.release();
        }
        HandshakeAction::HandleLocally(msg) => {
            tracing::debug!(from = %format!("{}:{}", req.from.host, req.from.port), inbound_mode = msg.mode, "handshake HANDLE_LOCALLY");
            let (reply_tx, reply_rx) = oneshot::channel();
            let from = req.from.clone();
            let target = req.target;
            let peer_address = msg.peer_address.clone();
            // FROM_CLIENT direct handshakes are answered via the inbound
            // request's REPLY channel (`req.reply`). FROM_RELAY /
            // FROM_SECOND_RELAY handshakes arrive as REQUESTs whose tid was
            // preserved by the FE-holder via `req.relay`-style forwarding;
            // we mirror Node `lib/server.js _addHandshake case FROM_RELAY`
            // by calling `dht.relay_with_tid(...)` to push a tid-preserved
            // FROM_SERVER REQUEST back to the FE-holder. The FE-holder then
            // finalises the chain by emitting the REPLY directly to the
            // original client (see HandshakeAction::ReplyTo above).
            let inbound_mode = msg.mode;
            let inbound_tid = req.tid;
            // FROM_SERVER goes back to `req.from`, or for FROM_SECOND_RELAY to the
            // first relay named in the message (JS `router.js`); none: silence.
            let relay_to = relay_target(inbound_mode, &req.from, msg.relay_address.as_ref());
            let route = HandshakeRoute::new(req.arrived_on.clone());
            let reply_via = route.reply_via.clone();

            let sent = server_tx
                .send(ServerEvent::PeerHandshake {
                    msg,
                    from,
                    peer_address,
                    target,
                    reply_tx,
                    route,
                })
                .is_ok();

            if sent {
                let dht = dht.clone();
                tokio::spawn(async move {
                    match reply_rx.await {
                        Ok(Some(value)) => match inbound_mode {
                            crate::hyperdht_messages::MODE_FROM_CLIENT => req.reply(Some(value)),
                            crate::hyperdht_messages::MODE_FROM_RELAY
                            | crate::hyperdht_messages::MODE_FROM_SECOND_RELAY => {
                                if let Some(to) = &relay_to
                                    && let Err(e) = dht.relay_with_tid_via(
                                        PEER_HANDSHAKE,
                                        target,
                                        Some(value),
                                        to,
                                        inbound_tid,
                                        reply_via.take(),
                                    )
                                {
                                    tracing::debug!(err = %e, "relay_with_tid send failed");
                                }
                                // The FE-holder used `req.relay`-style
                                // forwarding to reach us; it is not awaiting
                                // a REPLY for this REQUEST. Release the
                                // inbound req so the deferred-reply pipeline
                                // does not emit an auto-error REPLY racing
                                // against the genuine REPLY arriving via the
                                // chain's tail.
                                req.release();
                            }
                            _ => req.reply(Some(value)),
                        },
                        Ok(None) => req.release(),
                        Err(_) => req.error(1),
                    }
                });
            } else {
                req.reply(None);
            }
        }
        HandshakeAction::CloserNodes => {
            tracing::debug!(from = %format!("{}:{}", req.from.host, req.from.port), "handshake CLOSER_NODES");
            req.reply(None);
        }
        HandshakeAction::Drop => {
            tracing::debug!(from = %format!("{}:{}", req.from.host, req.from.port), "handshake DROP");
            drop(req);
        }
    }
}
