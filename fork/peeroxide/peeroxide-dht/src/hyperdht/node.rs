//! `spawn`: the node task, its request handler and plugin replies.
//! Neighbours: `node_handshake` and `node_holepunch` handle the peer requests it dispatches.

use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use libudx::UdxRuntime;

use crate::hyperdht_messages::{
    ANNOUNCE, FIND_PEER, IMMUTABLE_GET, IMMUTABLE_PUT, LOOKUP, MUTABLE_GET, MUTABLE_PUT,
    PEER_HANDSHAKE, PEER_HOLEPUNCH, PLUGIN, UNANNOUNCE,
};
use crate::persistent::{HandlerReply, IncomingHyperRequest, Persistent, PersistentConfig};
use crate::router::{ForwardEntry, Router};
use crate::rpc::DhtHandle;

use super::config::HyperDhtConfig;
use super::handle::{AdminRequest, HyperDhtHandle};
use super::node_handshake::handle_peer_handshake;
use super::node_holepunch::handle_peer_holepunch;
use super::relay_cache::RelayCache;
use super::reuse::Reuse;
use super::types::{HyperDhtError, ServerEvent};

// ── Spawn ─────────────────────────────────────────────────────────────────────

/// Create a HyperDHT instance and start its background tasks.
pub async fn spawn(
    runtime: &UdxRuntime,
    config: HyperDhtConfig,
) -> Result<
    (
        JoinHandle<Result<(), HyperDhtError>>,
        HyperDhtHandle,
        mpsc::UnboundedReceiver<ServerEvent>,
    ),
    HyperDhtError,
> {
    let (dht_join, dht_handle) = crate::rpc::spawn(runtime, config.dht).await?;
    let persistent_config = config.persistent;

    let request_rx = dht_handle
        .subscribe_requests()
        .await
        .ok_or(HyperDhtError::ChannelClosed)?;

    let router = Arc::new(Mutex::new(Router::new()));
    let (server_tx, server_rx) = mpsc::unbounded_channel();
    let (admin_tx, admin_rx) = mpsc::unbounded_channel::<AdminRequest>();

    let request_task = tokio::spawn(run_request_handler(
        request_rx,
        persistent_config,
        dht_handle.clone(),
        Arc::clone(&router),
        server_tx.clone(),
        admin_rx,
    ));

    let join = tokio::spawn(async move {
        tokio::select! {
            res = dht_join => {
                match res {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(e)) => Err(HyperDhtError::Dht(e)),
                    Err(_) => Err(HyperDhtError::ChannelClosed),
                }
            }
            res = request_task => {
                match res {
                    Ok(()) => Ok(()),
                    Err(_) => Err(HyperDhtError::ChannelClosed),
                }
            }
        }
    });

    let forward_dht = dht_handle.clone();
    let reuse = Reuse::new(Arc::new(move |addr, data, via| {
        forward_dht.forward_inbound_via(addr, data, via)
    }));
    let handle = HyperDhtHandle {
        dht: dht_handle,
        router,
        server_tx,
        admin_tx,
        current_relay_addresses: Arc::new(Mutex::new(Vec::new())),
        relay_cache: Arc::new(Mutex::new(RelayCache::default())),
        freezers: Arc::new(Mutex::new(Vec::new())),
        reuse,
    };
    Ok((join, handle, server_rx))
}

async fn run_request_handler(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<crate::rpc::UserRequest>,
    config: PersistentConfig,
    dht: DhtHandle,
    router: Arc<Mutex<Router>>,
    server_tx: mpsc::UnboundedSender<ServerEvent>,
    mut admin_rx: mpsc::UnboundedReceiver<AdminRequest>,
) {
    let mut storage = Persistent::new(config);

    loop {
        let mut req = tokio::select! {
            biased;
            Some(admin_req) = admin_rx.recv() => {
                match admin_req {
                    AdminRequest::PersistentStats { reply } => {
                        let _ = reply.send(storage.stats());
                    }
                }
                continue;
            }
            req = rx.recv() => match req {
                Some(r) => r,
                None => break,
            },
        };
        match req.command {
            PEER_HANDSHAKE => {
                tracing::debug!(from = %format!("{}:{}", req.from.host, req.from.port), "request: PEER_HANDSHAKE");
                handle_peer_handshake(req, &dht, &router, &server_tx);
                continue;
            }
            PEER_HOLEPUNCH => {
                tracing::debug!(from = %format!("{}:{}", req.from.host, req.from.port), "request: PEER_HOLEPUNCH");
                handle_peer_holepunch(req, &dht, &router, &server_tx);
                continue;
            }
            _ => {}
        }

        let node_id = req.id;

        let incoming = IncomingHyperRequest {
            command: req.command,
            target: req.target,
            token: req.token,
            value: req.value.clone(),
            from: req.from.clone(),
            id: node_id,
        };

        let reply = match req.command {
            FIND_PEER => {
                tracing::debug!(from = %format!("{}:{}", req.from.host, req.from.port), "request: FIND_PEER");
                storage.on_find_peer(&incoming)
            }
            LOOKUP => {
                tracing::debug!(from = %format!("{}:{}", req.from.host, req.from.port), "request: LOOKUP");
                storage.on_lookup(&incoming)
            }
            ANNOUNCE => {
                tracing::debug!(from = %format!("{}:{}", req.from.host, req.from.port), "request: ANNOUNCE");
                let own_id = dht.table_id().await.ok().flatten();
                if let Some(nid) = own_id {
                    let result = storage.on_announce(&incoming, &nid);
                    if !matches!(result, HandlerReply::Silent) {
                        if let Some(target) = incoming.target {
                            if let Ok(mut r) = router.lock() {
                                let already_server = r.get(&target).is_some_and(|e| e.has_server);
                                if !already_server {
                                    r.set(
                                        &target,
                                        ForwardEntry {
                                            relay: Some(incoming.from.clone()),
                                            has_server: false,
                                            inserted: std::time::Instant::now(),
                                        },
                                    );
                                }
                            }
                        }
                    }
                    result
                } else {
                    HandlerReply::Silent
                }
            }
            UNANNOUNCE => {
                tracing::debug!(from = %format!("{}:{}", req.from.host, req.from.port), "request: UNANNOUNCE");
                let own_id = dht.table_id().await.ok().flatten();
                if let Some(nid) = own_id {
                    storage.on_unannounce(&incoming, &nid)
                } else {
                    HandlerReply::Silent
                }
            }
            MUTABLE_PUT => {
                tracing::debug!(from = %format!("{}:{}", req.from.host, req.from.port), "request: MUTABLE_PUT");
                storage.on_mutable_put(&incoming)
            }
            MUTABLE_GET => {
                tracing::debug!(from = %format!("{}:{}", req.from.host, req.from.port), "request: MUTABLE_GET");
                storage.on_mutable_get(&incoming)
            }
            IMMUTABLE_PUT => {
                tracing::debug!(from = %format!("{}:{}", req.from.host, req.from.port), "request: IMMUTABLE_PUT");
                storage.on_immutable_put(&incoming)
            }
            IMMUTABLE_GET => {
                tracing::debug!(from = %format!("{}:{}", req.from.host, req.from.port), "request: IMMUTABLE_GET");
                storage.on_immutable_get(&incoming)
            }
            PLUGIN => {
                tracing::debug!(from = %format!("{}:{}", req.from.host, req.from.port), "request: PLUGIN");
                reply_to_plugin(req);
                continue;
            }
            _ => {
                tracing::debug!(cmd = req.command, from = %format!("{}:{}", req.from.host, req.from.port), "request: unknown command");
                req.error(crate::rpc::ERR_UNKNOWN_COMMAND);
                continue;
            }
        };

        match reply {
            HandlerReply::Value(v) | HandlerReply::ValueNoToken(v) => {
                req.reply(v);
            }
            HandlerReply::Error(code) => {
                req.error(code);
            }
            HandlerReply::Silent => {
                drop(req);
            }
        }
    }
}

/// PLUGIN: no plugins are registered, so the request gets no reply — like hyperdht
/// `persistent.js:onplugin` on a persistent node. (A JS node that has no id yet answers
/// UNKNOWN_COMMAND; the routing table here always has an id, so that case is not modelled.)
fn reply_to_plugin(mut req: crate::rpc::UserRequest) {
    req.release();
}
