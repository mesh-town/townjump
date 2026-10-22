//! Tests for blind-relay sessions: end-to-end pairing, teardown and idle timeout.
// JS: blind-relay/index.js

use super::test_support::{mem_pair, noop_outbound};
use super::*;
use crate::protomux::Mux;
use tokio::sync::mpsc;

// ── BlindRelaySession end-to-end (two real BlindRelayClients vs two
//    BlindRelaySessions sharing one BlindRelayServer) ─────────────────

#[tokio::test]
async fn session_end_to_end_pair_match() {
    let server = BlindRelayServer::new(BlindRelayServerConfig::default());
    let (matched_tx, mut matched_rx) = mpsc::unbounded_channel::<MatchedPairing>();

    // Client A <-> relay-side session A, over one in-memory pipe.
    let (client_a_stream, session_a_stream) = mem_pair();
    let (mux_client_a, run_client_a) = Mux::new(client_a_stream);
    let (mux_session_a, run_session_a) = Mux::new(session_a_stream);
    tokio::spawn(run_client_a);
    tokio::spawn(run_session_a);

    // Client B <-> relay-side session B, over a second in-memory pipe.
    let (client_b_stream, session_b_stream) = mem_pair();
    let (mux_client_b, run_client_b) = Mux::new(client_b_stream);
    let (mux_session_b, run_session_b) = Mux::new(session_b_stream);
    tokio::spawn(run_client_b);
    tokio::spawn(run_session_b);

    let channel_a = mux_session_a
        .create_channel(PROTOCOL_NAME, None, None)
        .await
        .unwrap();
    let channel_b = mux_session_b
        .create_channel(PROTOCOL_NAME, None, None)
        .await
        .unwrap();

    let mut session_a = BlindRelaySession::new(server.clone(), channel_a).unwrap();
    let mut session_b = BlindRelaySession::new(server.clone(), channel_b).unwrap();

    let matched_tx_a = matched_tx.clone();
    let session_a_task = tokio::spawn(async move {
        session_a.run(&matched_tx_a).await;
    });
    let session_b_task = tokio::spawn(async move {
        session_b.run(&matched_tx).await;
    });

    let token = [0x77; 32];

    let mut client_a = BlindRelayClient::open(&mux_client_a, None).await.unwrap();
    let mut client_b = BlindRelayClient::open(&mux_client_b, None).await.unwrap();
    client_a.wait_opened().await.unwrap();
    client_b.wait_opened().await.unwrap();

    // Stand in for `relay_service`: once the match arrives, create the
    // (fake, in this test) data-plane streams and reply to both sides
    // with the newly-assigned local stream ids.
    let wiring_task = tokio::spawn(async move {
        let matched = matched_rx.recv().await.unwrap();
        assert_eq!(matched.token, token);

        matched
            .first
            .outbound_tx
            .send(SessionOutbound::PairMatched {
                token,
                is_initiator: matched.first.is_initiator,
                local_stream_id: 9001,
            })
            .unwrap();
        matched
            .second
            .outbound_tx
            .send(SessionOutbound::PairMatched {
                token,
                is_initiator: matched.second.is_initiator,
                local_stream_id: 9002,
            })
            .unwrap();

        matched
    });

    let (resp_a, resp_b) = tokio::join!(
        client_a.pair(true, &token, 111),
        client_b.pair(false, &token, 222),
    );
    let resp_a = resp_a.unwrap();
    let resp_b = resp_b.unwrap();
    let matched = wiring_task.await.unwrap();

    // Each client's reported remote_id is the relay-assigned local
    // stream id sent for its own session (9001 for whichever session
    // was A, 9002 for whichever was B) — assert both distinct ids were
    // actually delivered to the two clients (order depends on which
    // pair message the relay processed first).
    let ids = [resp_a.remote_id, resp_b.remote_id];
    assert!(ids.contains(&9001) && ids.contains(&9002));
    let _ = matched;

    client_a.close();
    client_b.close();
    session_a_task.abort();
    session_b_task.abort();
}

#[test]
fn session_new_respects_max_sessions() {
    let config = BlindRelayServerConfig {
        max_sessions: 0,
        ..BlindRelayServerConfig::default()
    };
    let server = BlindRelayServer::new(config);

    // We don't need a real channel to exercise the capacity check —
    // try_accept_session is what BlindRelaySession::new consults first.
    assert!(server.try_accept_session().is_none());
}

// ── Stream teardown on unpair-of-active / session-close ───────────────
// (Node precedent: blind-relay's `_onunpair` destroys an already-
// matched stream found in `this._streams`; `_onclose` destroys every
// stream in that map when the session's channel closes.)

#[test]
fn unpair_on_active_pairing_signals_teardown() {
    let server = BlindRelayServer::new(BlindRelayServerConfig::default());
    let token = [0x88; 32];
    let (teardown_tx, mut teardown_rx) = mpsc::unbounded_channel::<()>();

    // Simulate a matched pairing whose data-plane the caller has
    // already wired up (mirrors relay_service::bridge_one_pairing
    // calling mark_active after try_pair returned Matched).
    server.try_pair(1, true, token, 1, 0, noop_outbound());
    let PairOutcome::Matched(_) = server.try_pair(2, false, token, 2, 0, noop_outbound()) else {
        panic!("expected Matched");
    };
    server.mark_active(token, [1, 2], teardown_tx);
    assert_eq!(server.stats().pairings_active, 1);

    // Either session can unpair an active pairing (mirrors Node: the
    // lookup is by token in the server-wide `_streams`-equivalent
    // table, not scoped to "the session that registered this slot").
    assert_eq!(server.unpair(1, token), UnpairOutcome::Destroyed);
    assert!(teardown_rx.try_recv().is_ok(), "teardown signal not sent");
    assert_eq!(server.stats().pairings_active, 0);

    // A second unpair against the same (now-removed) token finds nothing.
    assert_eq!(server.unpair(1, token), UnpairOutcome::NotFound);
}

#[test]
fn release_session_tears_down_its_active_pairings() {
    let server = BlindRelayServer::new(BlindRelayServerConfig::default());
    let token = [0x99; 32];
    let (teardown_tx, mut teardown_rx) = mpsc::unbounded_channel::<()>();

    let session_a = server.try_accept_session().unwrap();
    let session_b = server.try_accept_session().unwrap();
    server.try_pair(session_a, true, token, 1, 0, noop_outbound());
    server.try_pair(session_b, false, token, 2, 0, noop_outbound());
    server.mark_active(token, [session_a, session_b], teardown_tx);
    assert_eq!(server.stats().pairings_active, 1);

    // Releasing *either* session (not just the one that happened to
    // register second) tears down the shared pairing.
    server.release_session(session_a);

    assert!(teardown_rx.try_recv().is_ok(), "teardown signal not sent");
    assert_eq!(server.stats().pairings_active, 0);
}

#[test]
fn release_session_only_tears_down_its_own_active_pairings() {
    let server = BlindRelayServer::new(BlindRelayServerConfig::default());
    let token_a = [0xaa; 32];
    let token_b = [0xbb; 32];
    let (teardown_a_tx, mut teardown_a_rx) = mpsc::unbounded_channel::<()>();
    let (teardown_b_tx, mut teardown_b_rx) = mpsc::unbounded_channel::<()>();

    let session_1 = server.try_accept_session().unwrap();
    let session_2 = server.try_accept_session().unwrap();
    let session_3 = server.try_accept_session().unwrap();

    server.try_pair(session_1, true, token_a, 1, 0, noop_outbound());
    server.try_pair(session_2, false, token_a, 2, 0, noop_outbound());
    server.mark_active(token_a, [session_1, session_2], teardown_a_tx);

    server.try_pair(session_2, true, token_b, 3, 1, noop_outbound());
    server.try_pair(session_3, false, token_b, 4, 0, noop_outbound());
    server.mark_active(token_b, [session_2, session_3], teardown_b_tx);

    assert_eq!(server.stats().pairings_active, 2);

    // Releasing session_1 only affects token_a's pairing (session_1
    // wasn't part of token_b's pairing).
    server.release_session(session_1);

    assert!(teardown_a_rx.try_recv().is_ok());
    assert!(teardown_b_rx.try_recv().is_err());
    assert_eq!(server.stats().pairings_active, 1);
}

// ── Idle-session-timeout (peeroxide-only, no Node precedent) ──────────

#[test]
fn sweep_idle_sessions_closes_sessions_past_timeout() {
    let config = BlindRelayServerConfig {
        idle_session_timeout: Duration::from_millis(1),
        ..BlindRelayServerConfig::default()
    };
    let server = BlindRelayServer::new(config);
    let session_id = server.try_accept_session().unwrap();
    let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel();
    server.register_session(session_id, outbound_tx);

    std::thread::sleep(Duration::from_millis(20));

    let closed = server.sweep_idle_sessions();
    assert_eq!(closed, vec![session_id]);
    assert!(matches!(outbound_rx.try_recv(), Ok(SessionOutbound::Close)));
}

#[test]
fn sweep_idle_sessions_keeps_recently_active_sessions() {
    let config = BlindRelayServerConfig {
        idle_session_timeout: Duration::from_secs(300),
        ..BlindRelayServerConfig::default()
    };
    let server = BlindRelayServer::new(config);
    let session_id = server.try_accept_session().unwrap();
    let (outbound_tx, _outbound_rx) = mpsc::unbounded_channel();
    server.register_session(session_id, outbound_tx);

    assert_eq!(server.sweep_idle_sessions(), Vec::<u64>::new());
}

#[test]
fn pair_and_unpair_reset_idle_clock() {
    let config = BlindRelayServerConfig {
        idle_session_timeout: Duration::from_millis(50),
        ..BlindRelayServerConfig::default()
    };
    let server = BlindRelayServer::new(config);
    let session_id = server.try_accept_session().unwrap();
    let (outbound_tx, _outbound_rx) = mpsc::unbounded_channel();
    server.register_session(session_id, outbound_tx);

    std::thread::sleep(Duration::from_millis(30));
    // Activity within the timeout window resets the clock.
    server.try_pair(session_id, true, [0x77; 32], 1, 0, noop_outbound());
    std::thread::sleep(Duration::from_millis(30));

    assert_eq!(
        server.sweep_idle_sessions(),
        Vec::<u64>::new(),
        "recent pair() activity should have reset the idle clock"
    );
}
