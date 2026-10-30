//! One relay control session: drives a `BlindRelaySession` over a Protomux
//! channel, and the registry of the UDX socket each session runs on.
// JS: blind-relay/index.js

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use libudx::UdxSocket;

use crate::blind_relay::{BlindRelayServer, BlindRelaySession, MatchedPairing};
use crate::protomux::{Channel, Mux};
use std::time::Duration;
use tokio::sync::mpsc;

/// How long a session waits for the peer to open the channel (JS `HANDSHAKE_INITIAL_TIMEOUT`).
const OPEN_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) async fn run_relay_session(
    relay: BlindRelayServer,
    mux: Mux,
    remote_public_key: [u8; 32],
    matched_tx: mpsc::UnboundedSender<MatchedPairing>,
    sockets: SessionSockets,
    socket: UdxSocket,
) {
    let channel = match mux
        .create_channel(
            crate::blind_relay::PROTOCOL_NAME,
            Some(remote_public_key.to_vec()),
            None,
        )
        .await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!(err = %e, "relay: failed to open blind-relay channel");
            return;
        }
    };

    serve_session(relay, channel, &matched_tx, OPEN_TIMEOUT, &sockets, socket).await;
}

/// Runs the session once the peer opens its side of the channel; a peer that never does
/// within `open_timeout` loses the slot.
async fn serve_session(
    relay: BlindRelayServer,
    channel: Channel,
    matched_tx: &mpsc::UnboundedSender<MatchedPairing>,
    open_timeout: Duration,
    sockets: &SessionSockets,
    socket: UdxSocket,
) {
    let Some(mut session) = BlindRelaySession::new(relay, channel) else {
        tracing::warn!("relay: rejecting session — at max_sessions capacity");
        return;
    };

    // The pair's streams must sit on the socket this control connection runs on: the client
    // sends its pair packets to the address it reached us at.
    let _hold = sockets.hold(session.session_id(), socket);

    let opened = tokio::time::timeout(open_timeout, session.wait_opened()).await;
    if !matches!(opened, Ok(Ok(()))) {
        // A handshake whose client never dialed in (it raced several nodes and kept another
        // one) never opens the channel: a stream with no address is held, not failed by RTO,
        // so nothing else ends the wait. JS only creates the session once the client's first
        // packet arrives and drops a silent handshake after HANDSHAKE_INITIAL_TIMEOUT; the slot
        // counted at `new` must not outlive that.
        session.release();
        return;
    }

    session.run(matched_tx).await;
}

/// The UDX socket of each live control session, keyed by session id (cheap to clone).
#[derive(Clone, Default)]
pub(super) struct SessionSockets(Arc<Mutex<HashMap<u64, UdxSocket>>>);

/// Keeps a session's socket registered; dropping it forgets the entry.
pub(super) struct SocketHold {
    sockets: SessionSockets,
    session_id: u64,
}

impl SessionSockets {
    fn lock(&self) -> MutexGuard<'_, HashMap<u64, UdxSocket>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Register `socket` for `session_id` until the returned hold is dropped.
    pub(super) fn hold(&self, session_id: u64, socket: UdxSocket) -> SocketHold {
        self.lock().insert(session_id, socket);
        SocketHold {
            sockets: self.clone(),
            session_id,
        }
    }

    /// The socket of a live session.
    pub(super) fn get(&self, session_id: u64) -> Option<UdxSocket> {
        self.lock().get(&session_id).cloned()
    }
}

impl Drop for SocketHold {
    fn drop(&mut self) {
        self.sockets.lock().remove(&self.session_id);
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
