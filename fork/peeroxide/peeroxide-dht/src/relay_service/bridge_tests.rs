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

    let pair = open_pair(&runtime, [&socket, &socket], &relay, [11, 12], [100, 200])
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

    let result = open_pair(&runtime, [&socket, &socket], &relay, [21, 22], [bad, 1]).await;
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

mod pair_on_two_sockets {
    use super::*;
    use crate::hyperdht::HyperDhtError;
    use crate::relay_service::session::SessionSockets;

    #[tokio::test]
    async fn bytes_cross_between_streams_on_different_sockets() {
        // given: a relay whose two bridged streams sit on two different sockets, one client per socket
        let runtime = UdxRuntime::new().expect("runtime");
        let (relay_a, relay_b) = (bound(&runtime).await, bound(&runtime).await);
        let (client_a, client_b) = (bound(&runtime).await, bound(&runtime).await);
        let relay = matched_server();
        let _bridged = open_pair(&runtime, [&relay_a, &relay_b], &relay, [11, 12], [101, 102])
            .await
            .expect("open pair");
        let mut a = runtime.create_stream(101).await.expect("stream a");
        let mut b = runtime.create_stream(102).await.expect("stream b");
        let addr_a = relay_a.local_addr().await.expect("relay a address");
        let addr_b = relay_b.local_addr().await.expect("relay b address");

        // when: each client connects to the relay socket its stream lives on and speaks first
        a.connect(&client_a, 11, addr_a).await.expect("connect a");
        b.connect(&client_b, 12, addr_b).await.expect("connect b");
        let (wa, wb) = tokio::join!(a.write(b"from a"), b.write(b"from b"));
        wa.expect("write a");
        wb.expect("write b");

        // then: each hears the other
        let heard = tokio::time::timeout(Duration::from_secs(10), async {
            (
                b.read().await.expect("read b"),
                a.read().await.expect("read a"),
            )
        })
        .await
        .expect("relayed bytes did not arrive");
        assert_eq!(heard.0.as_deref(), Some(&b"from a"[..]));
        assert_eq!(heard.1.as_deref(), Some(&b"from b"[..]));
    }

    #[tokio::test]
    async fn pairing_without_a_session_socket_counts_an_error() {
        // given: a matched pairing whose sessions have no registered socket
        let runtime = UdxRuntime::new().expect("runtime");
        let relay = BlindRelayServer::new(BlindRelayServerConfig::default());
        relay.try_pair(1, true, TOKEN, 100, 0, noop_outbound());
        let PairOutcome::Matched(matched) =
            relay.try_pair(2, false, TOKEN, 200, 0, noop_outbound())
        else {
            panic!("expected Matched");
        };

        // when: the bridge is asked to open it
        let result = bridge_one_pairing(
            runtime.handle(),
            &SessionSockets::default(),
            &relay,
            matched,
        )
        .await;

        // then: it fails the way a missing server socket did, and counts the error
        assert!(matches!(result, Err(HyperDhtError::StreamEstablishment(_))));
        assert_eq!(relay.stats().streams_errors, 1);
    }
}

async fn bound(runtime: &UdxRuntime) -> UdxSocket {
    let socket = runtime.create_socket().await.expect("socket");
    socket
        .bind("127.0.0.1:0".parse().unwrap())
        .await
        .expect("bind");
    socket
}
