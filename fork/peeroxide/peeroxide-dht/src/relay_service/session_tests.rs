//! Tests for the relay control session: a peer that never opens the channel loses its slot.
// JS: hyperdht/lib/server.js (HANDSHAKE_INITIAL_TIMEOUT)

use super::*;
use crate::blind_relay::{BlindRelayServerConfig, PROTOCOL_NAME};
use crate::protomux::FramedStream;

/// A wire whose far end is silent: frames go nowhere, nothing ever arrives.
struct SilentPeer;

impl FramedStream for SilentPeer {
    async fn read_frame(&mut self) -> std::io::Result<Option<Vec<u8>>> {
        std::future::pending().await
    }

    async fn write_frame(&mut self, _data: &[u8]) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn silent_peer_releases_the_slot_after_the_open_timeout() {
    let relay = BlindRelayServer::new(BlindRelayServerConfig::default());
    let (mux, run) = Mux::new(SilentPeer);
    tokio::spawn(run);
    let channel = mux
        .create_channel(PROTOCOL_NAME, None, None)
        .await
        .expect("channel");
    let (matched_tx, _matched_rx) = mpsc::unbounded_channel();

    let serving = tokio::spawn({
        let relay = relay.clone();
        async move { serve_session(relay, channel, &matched_tx, OPEN_TIMEOUT).await }
    });
    tokio::time::sleep(OPEN_TIMEOUT / 2).await;
    assert_eq!(
        relay.stats().sessions_active,
        1,
        "slot is held while waiting"
    );

    tokio::time::sleep(OPEN_TIMEOUT).await;
    serving.await.expect("session task");
    assert_eq!(
        relay.stats().sessions_active,
        0,
        "slot returned after the timeout"
    );
}
