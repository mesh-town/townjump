//! `handle_peer_holepunch`: the node's answer to PEER_HOLEPUNCH (server, relay or forward).
//! Neighbours: `node` dispatches here; `server_passive` builds the reply.

use std::sync::{Arc, Mutex};

use tokio::sync::{mpsc, oneshot};

use crate::hyperdht_messages::PEER_HOLEPUNCH;
use crate::router::{HolepunchAction, Router};
use crate::rpc::DhtHandle;

use super::types::ServerEvent;

pub(super) fn handle_peer_holepunch(
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
        match router.route_holepunch(req.target.as_ref(), &req.from, value) {
            Ok(a) => a,
            Err(_) => {
                req.error(1);
                return;
            }
        }
    };

    match action {
        HolepunchAction::ForwardRequest { value, to } => {
            // FE-holder relay: forward the inbound REQUEST as a new REQUEST
            // to `to`, preserving the original requester's tid end-to-end.
            // Fire-and-forget — the eventual REPLY reaches the original
            // client directly via the tid-preserved chain (server replies
            // with `dht.relay_with_tid` back to this FE-holder, which then
            // dispatches HolepunchAction::ReplyTo to send the REPLY packet
            // straight to the client at peer_address). Mirrors Node
            // `dht-rpc::Request.relay(value, to)`.
            tracing::debug!(
                from = %format!("{}:{}", req.from.host, req.from.port),
                to = %format!("{}:{}", to.host, to.port),
                "holepunch FORWARD_REQUEST — tid-preserved relay"
            );
            if let Err(e) =
                dht.relay_with_tid(PEER_HOLEPUNCH, req.target, Some(value), &to, req.tid)
            {
                tracing::debug!(err = %e, "ForwardRequest: relay_with_tid send failed");
            }
            // No reply to the inbound REQUEST — the original client awaits
            // its REPLY via the preserved-tid chain, not via this hop.
            req.release();
        }
        HolepunchAction::ReplyTo { value, to } => {
            // FE-holder finalises the relay chain: send a REPLY packet
            // (Response, not Request) with the inbound tid (= original
            // client's tid) directly to the client at `to`. The REPLY
            // matches the client's outstanding inflight entry. Mirrors
            // Node `dht-rpc::Request.reply(value, { to })` as invoked from
            // `lib/server.js _addHolepunch case FROM_SERVER`.
            tracing::debug!(
                from = %format!("{}:{}", req.from.host, req.from.port),
                to = %format!("{}:{}", to.host, to.port),
                "holepunch REPLY_TO — finalise tid-preserved chain"
            );
            if let Err(e) = dht.send_reply_to(req.tid, req.target, &to, Some(value)) {
                tracing::debug!(err = %e, "ReplyTo: send_reply_to failed");
            }
            // The inbound REQUEST came from the server via its relay_with_tid;
            // the server is not awaiting a reply, so we do not call
            // req.reply / req.error here.
            req.release();
        }
        HolepunchAction::Relay { value, to } => {
            // Legacy variant — the router no longer emits this for holepunch.
            // Retained for forward-compatibility (non_exhaustive enum) so any
            // external producer continues to function. Treat as fire-and-forget
            // tid-preserved relay (equivalent to ForwardRequest).
            tracing::warn!(
                from = %format!("{}:{}", req.from.host, req.from.port),
                to = %format!("{}:{}", to.host, to.port),
                "holepunch legacy Relay action — treating as ForwardRequest"
            );
            if let Err(e) =
                dht.relay_with_tid(PEER_HOLEPUNCH, req.target, Some(value), &to, req.tid)
            {
                tracing::debug!(err = %e, "legacy Relay: relay_with_tid send failed");
            }
            req.release();
        }
        HolepunchAction::Reply { value, to } => {
            // Legacy variant — the router no longer emits this for holepunch.
            // Retained for forward-compatibility (non_exhaustive enum). Treat
            // as REPLY-to-arbitrary-address (equivalent to ReplyTo).
            tracing::warn!(
                from = %format!("{}:{}", req.from.host, req.from.port),
                to = %format!("{}:{}", to.host, to.port),
                "holepunch legacy Reply action — treating as ReplyTo"
            );
            if let Err(e) = dht.send_reply_to(req.tid, req.target, &to, Some(value)) {
                tracing::debug!(err = %e, "legacy Reply: send_reply_to failed");
            }
            req.release();
        }
        HolepunchAction::HandleLocally { msg, peer_address } => {
            tracing::debug!(
                from = %format!("{}:{}", req.from.host, req.from.port),
                peer = %format!("{:?}", peer_address),
                inbound_mode = msg.mode,
                "holepunch HANDLE_LOCALLY"
            );
            let (reply_tx, reply_rx) = oneshot::channel();
            let from = req.from.clone();
            let target = req.target;
            // FROM_CLIENT direct holepunch (if ever wired) is answered via
            // the inbound request's REPLY channel (`req.reply`). FROM_RELAY /
            // FROM_SECOND_RELAY holepunches arrive as REQUESTs whose tid was
            // preserved by the FE-holder via `req.relay`-style forwarding;
            // we mirror Node `lib/server.js _addHolepunch case FROM_RELAY`
            // by calling `dht.relay_with_tid(...)` to push a tid-preserved
            // FROM_SERVER REQUEST back to the FE-holder. The FE-holder then
            // finalises the chain by emitting the REPLY directly to the
            // original client (see HolepunchAction::ReplyTo above).
            let inbound_mode = msg.mode;
            let inbound_tid = req.tid;

            let sent = server_tx
                .send(ServerEvent::PeerHolepunch {
                    msg,
                    from,
                    peer_address,
                    target,
                    reply_tx,
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
                                if let Err(e) = dht.relay_with_tid(
                                    PEER_HOLEPUNCH,
                                    target,
                                    Some(value),
                                    &req.from,
                                    inbound_tid,
                                ) {
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
        HolepunchAction::Drop => {
            tracing::debug!(from = %format!("{}:{}", req.from.host, req.from.port), "holepunch DROP");
            drop(req);
        }
    }
}
