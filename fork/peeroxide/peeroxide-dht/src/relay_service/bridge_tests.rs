//! Tests for the bridge tail: stream counters and teardown of both bridged streams.

use std::sync::Arc;
use std::time::Duration;

use libudx::UdxRuntime;

use super::*;
use crate::blind_relay::{BlindRelayServerConfig, PairOutcome, UnpairOutcome};

const TOKEN: [u8; 32] = [0x5a; 32];

fn noop_outbound() -> mpsc::UnboundedSender<SessionOutbound> {
    mpsc::unbounded_channel().0
}

/// Server with `TOKEN` matched between sessions 1 (initiator) and 2.
fn matched_server() -> BlindRelayServer {
    let server = BlindRelayServer::new(BlindRelayServerConfig::default());
    server.try_pair(1, true, TOKEN, 100, 0, noop_outbound());
    let PairOutcome::Matched(_) = server.try_pair(2, false, TOKEN, 200, 0, noop_outbound()) else {
        panic!("expected Matched");
    };
    server
}

async fn loopback() -> (UdxRuntime, UdxSocket) {
    let runtime = UdxRuntime::new().expect("runtime");
    let socket = runtime.create_socket().await.expect("socket");
    socket
        .bind("127.0.0.1:0".parse().unwrap())
        .await
        .expect("bind");
    (runtime, socket)
}

/// Spawns `hold_pair` and returns once the relay can signal teardown for it.
async fn held_pair(
    relay: &BlindRelayServer,
    active: &ActiveStreamMap,
    pair: (UdxStream, UdxStream),
) -> tokio::task::JoinHandle<()> {
    let (relay, active) = (relay.clone(), Arc::clone(active));
    tokio::spawn(async move { hold_pair(&active, &relay, TOKEN, [1, 2], pair).await })
}

async fn unpair_when_active(relay: &BlindRelayServer, session_id: u64) {
    for _ in 0..200 {
        if relay.unpair(session_id, TOKEN) == UnpairOutcome::Destroyed {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("pair never became active");
}

#[tokio::test]
async fn bridge_counts_streams() {
    let (runtime, socket) = loopback().await;
    let relay = matched_server();
    let active: ActiveStreamMap = Arc::default();

    let pair = open_pair(&runtime, &socket, &relay, [11, 12], [100, 200])
        .await
        .expect("open pair");
    assert_eq!(relay.stats().streams_opened, 2);

    let task = held_pair(&relay, &active, pair).await;
    unpair_when_active(&relay, 1).await;
    task.await.expect("hold task");

    let stats = relay.stats();
    assert_eq!((stats.streams_closed, stats.pairings_active), (2, 0));
}

#[tokio::test]
async fn open_pair_with_bad_client_id_counts_an_error() {
    let (runtime, socket) = loopback().await;
    let relay = matched_server();
    let bad = u64::from(u32::MAX) + 1;

    let result = open_pair(&runtime, &socket, &relay, [21, 22], [bad, 1]).await;
    assert!(result.is_err());
    assert_eq!(relay.stats().streams_errors, 1);
}

#[tokio::test]
async fn unpair_from_one_side_tears_down_both_streams() {
    let (runtime, socket) = loopback().await;
    let relay = matched_server();
    let active: ActiveStreamMap = Arc::default();
    let alive = Arc::new(());

    let mut pair = Vec::new();
    for id in [31, 32] {
        let guard = Arc::clone(&alive);
        let stream = runtime.create_stream(id).await.expect("stream");
        stream
            .set_firewall_hook(&socket, 1, move |_, _, _| {
                let _keep = &guard;
                true
            })
            .expect("hook");
        pair.push(stream);
    }
    let second = pair.pop().expect("second");
    let pair = (pair.pop().expect("first"), second);
    assert_eq!(Arc::strong_count(&alive), 3);

    let task = held_pair(&relay, &active, pair).await;
    unpair_when_active(&relay, 1).await;
    task.await.expect("hold task");

    assert!(active.lock().await.is_empty(), "bridge entry removed");
    for _ in 0..200 {
        if Arc::strong_count(&alive) == 1 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!(
        "both bridged streams must be dropped, live handles: {}",
        Arc::strong_count(&alive) - 1
    );
}
