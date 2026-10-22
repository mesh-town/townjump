//! One relay control session: drives a `BlindRelaySession` over a Protomux
//! channel. Siblings: `handshake`, `bridge`.
// JS: blind-relay/index.js

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

    serve_session(relay, channel, &matched_tx, OPEN_TIMEOUT).await;
}

/// Runs the session once the peer opens its side of the channel; a peer that never does
/// within `open_timeout` loses the slot.
async fn serve_session(
    relay: BlindRelayServer,
    channel: Channel,
    matched_tx: &mpsc::UnboundedSender<MatchedPairing>,
    open_timeout: Duration,
) {
    let Some(mut session) = BlindRelaySession::new(relay, channel) else {
        tracing::warn!("relay: rejecting session — at max_sessions capacity");
        return;
    };

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

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
