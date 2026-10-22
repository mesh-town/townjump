// JS: dht-rpc/lib/io.js
//! `impl Io`: creating requests, sending them through a socket, retransmit of in-flight ones.
//! Neighbours: `event_loop` times them out; `replies` answers incoming ones.

use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::time::Duration;

use libudx::UdxSocket;
use tokio::time::Instant;

use crate::messages;

use super::state::Io;
use super::types::{InflightEntry, Owner, PendingSend, RequestParams, SocketKind};

const DEFAULT_TIMEOUT_MS: u64 = 1000;
const DEFAULT_RETRIES: u32 = 3;

impl Io {
    /// Create an outgoing request.
    /// Returns the assigned TID, or `None` if the IO is destroying or the
    /// destination address is invalid.
    pub fn create_request(&mut self, params: RequestParams) -> Option<u16> {
        self.create_request_owned(params, None)
    }

    /// [`Self::create_request`] with an owner: `cancel_owner(owner)` drops the request.
    pub(crate) fn create_request_owned(
        &mut self,
        params: RequestParams,
        owner: Option<Owner>,
    ) -> Option<u16> {
        self.new_inflight(params, owner, None)
    }

    /// [`Self::create_request_owned`] through an external (held) socket: the first send, the
    /// window-queued send and every retry go out of `via`, without our node id (JS `io.js:488`).
    /// The clone stays in the entry until it is removed, so `via` outlives its lease.
    pub(crate) fn create_request_via(
        &mut self,
        params: RequestParams,
        owner: Option<Owner>,
        via: UdxSocket,
    ) -> Option<u16> {
        self.new_inflight(params, owner, Some(via))
    }

    /// Shared body of `create_request_owned` / `create_request_via`.
    fn new_inflight(
        &mut self,
        params: RequestParams,
        owner: Option<Owner>,
        via: Option<UdxSocket>,
    ) -> Option<u16> {
        if self.closed() {
            return None;
        }

        let addr_str = format!("{}:{}", params.to.host, params.to.port);
        let addr: SocketAddr = addr_str.parse().ok()?;

        let tid = self.tid;
        self.tid = self.tid.wrapping_add(1);

        let socket_kind = if self.firewalled {
            SocketKind::Client
        } else {
            SocketKind::Server
        };

        let include_id = !self.ephemeral && via.is_none() && socket_kind == SocketKind::Server;
        let id = if include_id {
            self.table.lock().ok().map(|t| *t.id())
        } else {
            None
        };

        let request = messages::Request {
            tid,
            to: params.to.clone(),
            id,
            token: params.token,
            internal: params.internal,
            command: params.command,
            target: params.target,
            value: params.value,
        };

        let buffer = match messages::encode_request_to_bytes(&request) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(err = %e, "create_request: encode failed");
                return None;
            }
        };

        self.stats.active += 1;
        self.stats.total += 1;

        let now = Instant::now();
        let to_str = format!("{}:{}", params.to.host, params.to.port);

        let timeout = Duration::from_millis(params.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS));
        let entry = InflightEntry {
            tid,
            to: params.to,
            addr,
            internal: params.internal,
            command: params.command,
            target: params.target,
            node_id: params.node_id,
            buffer,
            socket_kind,
            sent: 0,
            retries: params.retries.unwrap_or(DEFAULT_RETRIES),
            deadline: now + timeout,
            timestamp: now,
            timeout,
            owner,
            via,
        };

        let has_via = entry.via.is_some();
        self.inflight.push(entry);

        let cong_full = self.congestion.is_full();
        tracing::debug!(
            tid,
            command = params.command,
            to = %to_str,
            ?socket_kind,
            via = has_via,
            cong_full,
            inflight_count = self.inflight.len(),
            ?owner,
            "create_request: new inflight"
        );

        if cong_full {
            self.pending.push_back(PendingSend { tid });
        } else {
            let idx = self.inflight.len() - 1;
            self.send_inflight_at(idx);
        }

        Some(tid)
    }

    /// Send a one-shot DHT request out an arbitrary UDP socket (typically a
    /// puncher socket). The TID is allocated from the shared registry so the
    /// reply still matches via the existing inflight machinery, but no
    /// automatic retry will fire — we don't own the override socket beyond
    /// this call. Replies must be forwarded back through
    /// [`Self::handle_inbound_reply_bytes`].
    pub fn send_request_via_socket(
        &mut self,
        params: RequestParams,
        socket: &UdxSocket,
    ) -> Option<u16> {
        if self.closed() {
            return None;
        }

        let addr_str = format!("{}:{}", params.to.host, params.to.port);
        let addr: SocketAddr = addr_str.parse().ok()?;

        let tid = self.tid;
        self.tid = self.tid.wrapping_add(1);

        let include_id = !self.ephemeral;
        let id = if include_id {
            self.table.lock().ok().map(|t| *t.id())
        } else {
            None
        };

        let request = messages::Request {
            tid,
            to: params.to.clone(),
            id,
            token: params.token,
            internal: params.internal,
            command: params.command,
            target: params.target,
            value: params.value,
        };

        let buffer = match messages::encode_request_to_bytes(&request) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(err = %e, "send_request_via_socket: encode failed");
                return None;
            }
        };

        let buffer_len = buffer.len() as u64;
        if let Err(e) = socket.send_to(&buffer, addr) {
            tracing::warn!(err = %e, "send_request_via_socket: send_to failed");
            return None;
        }
        self.wire
            .bytes_sent
            .fetch_add(buffer_len, Ordering::Relaxed);

        self.stats.active += 1;
        self.stats.total += 1;

        let now = Instant::now();
        let timeout = Duration::from_millis(params.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS));

        self.inflight.push(InflightEntry {
            tid,
            to: params.to,
            addr,
            internal: params.internal,
            command: params.command,
            target: params.target,
            node_id: params.node_id,
            buffer,
            socket_kind: SocketKind::Client,
            sent: 1,
            retries: 0,
            deadline: now + timeout,
            timestamp: now,
            timeout,
            owner: None,
            via: None,
        });

        Some(tid)
    }

    /// Actually send the inflight entry at index `idx`: out of its `via` socket if it has one,
    /// else out of the IO socket of its kind.
    pub(super) fn send_inflight_at(&mut self, idx: usize) {
        let (buffer, addr, socket_kind, via) = {
            let entry = &mut self.inflight[idx];
            entry.sent += 1;
            entry.deadline = Instant::now() + entry.timeout;
            (
                entry.buffer.clone(),
                entry.addr,
                entry.socket_kind,
                entry.via.clone(),
            )
        };

        self.congestion.send();

        let socket = via.unwrap_or_else(|| self.socket_for(socket_kind));

        let buffer_len = buffer.len() as u64;
        if let Err(e) = socket.send_to(&buffer, addr) {
            tracing::warn!(err = %e, "send_inflight_at: send_to failed");
        } else {
            self.wire
                .bytes_sent
                .fetch_add(buffer_len, Ordering::Relaxed);
        }
    }
}
