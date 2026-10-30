//! Tests for the relay control session: a peer that never opens the channel loses its slot.
// JS: hyperdht/lib/server.js (HANDSHAKE_INITIAL_TIMEOUT)

use libudx::{UdxRuntime, UdxSocket};

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
    let runtime = UdxRuntime::new().expect("runtime");
    let socket = runtime.create_socket().await.expect("socket");

    let serving = tokio::spawn({
        let relay = relay.clone();
        async move {
            let sockets = SessionSockets::default();
            serve_session(relay, channel, &matched_tx, OPEN_TIMEOUT, &sockets, socket).await
        }
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

mod session_sockets {
    use super::*;

    #[tokio::test]
    async fn hold_registers_and_drop_forgets() {
        // given: a socket held for session 7
        let (_runtime, socket) = runtime_and_socket().await;
        let sockets = SessionSockets::default();
        let hold = sockets.hold(7, socket);

        // when / then: it is found while held ...
        assert!(sockets.get(7).is_some(), "a held socket is found");

        // ... and gone once the holder is dropped
        drop(hold);
        assert!(
            sockets.get(7).is_none(),
            "a dropped hold forgets the socket"
        );
    }

    #[tokio::test]
    async fn an_unknown_session_has_no_socket() {
        let sockets = SessionSockets::default();

        assert!(sockets.get(1).is_none());
    }

    #[tokio::test]
    async fn dropping_one_hold_keeps_the_other_session() {
        // given: sessions 1 and 2 on one registry
        let (_runtime, socket) = runtime_and_socket().await;
        let sockets = SessionSockets::default();
        let first = sockets.hold(1, socket.clone());
        let _second = sockets.hold(2, socket);

        // when: the first ends
        drop(first);

        // then: only its entry disappears
        assert!(sockets.get(1).is_none(), "session 1 is forgotten");
        assert!(sockets.get(2).is_some(), "session 2 is kept");
    }

    #[tokio::test]
    async fn a_clone_of_the_registry_sees_the_same_entries() {
        let (_runtime, socket) = runtime_and_socket().await;
        let sockets = SessionSockets::default();
        let _hold = sockets.hold(3, socket);

        assert!(sockets.clone().get(3).is_some());
    }
}

async fn runtime_and_socket() -> (UdxRuntime, UdxSocket) {
    let runtime = UdxRuntime::new().expect("runtime");
    let socket = runtime.create_socket().await.expect("socket");
    (runtime, socket)
}
