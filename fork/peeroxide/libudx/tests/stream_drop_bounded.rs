// JS: libudx/src/udx.c (a dead stream's frames are ignored there, udx.c:1549-1550; DESTROY closes a live one, udx.c:1607-1608)
//! Dropping an adapter that still has unsent data: the drain is bounded, new data is refused, DESTROY is repeated.
#![deny(clippy::all)]

mod common;

use std::time::Duration;

use common::raw_peer::{
    FLAG_DESTROY, FLAG_END, FLAG_HEARTBEAT, Link, OPEN_WINDOW, RawPeer, Received, ack_of,
    ack_packet, data_packet, end_packet, flags_of,
};
use common::with_timeout;
use libudx::{UdxAsyncStream, UdxStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TEST_LIMIT: Duration = Duration::from_secs(40);
/// `clamp(4 * rto, 2 s, 10 s)` at the initial rto of 1 s, plus slack.
const DRAIN_BOUND_MS: u64 = 4_500;

fn is_destroy(packet: &Received) -> bool {
    flags_of(&packet.1) & FLAG_DESTROY != 0
}

/// An adapter with 64 KiB written; `window` is what the peer advertised first (`None` — the peer never spoke).
async fn adapter_with_unsent(
    stream: UdxStream,
    peer: &RawPeer,
    window: Option<u32>,
) -> UdxAsyncStream {
    if let Some(window) = window {
        peer.send(&ack_packet(0, window, &[])).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let mut adapter = stream.into_async_stream();
    adapter.write_all(&[7u8; 65_536]).await.expect("write");
    adapter
}

#[tokio::test]
async fn drop_at_zero_window_sends_destroy_within_drain_limit() {
    with_timeout(TEST_LIMIT, async {
        // given: the peer advertised window 0, the adapter holds 64 KiB it cannot send
        let Link {
            peer,
            stream,
            keep_alive: _keep,
        } = Link::new().await;
        let adapter = adapter_with_unsent(stream, &peer, Some(0)).await;
        peer.collect_for(Duration::from_millis(100)).await;
        // when: the adapter is dropped
        drop(adapter);
        // then: DESTROY arrives within the bound and no HEARTBEAT follows it
        let got = peer.collect_for(Duration::from_secs(7)).await;
        let first = got.iter().position(is_destroy).expect("no DESTROY");
        assert!(
            got[first].0 <= DRAIN_BOUND_MS,
            "DESTROY at {} ms",
            got[first].0
        );
        let late = got[first..]
            .iter()
            .filter(|(_, p)| flags_of(p) & FLAG_HEARTBEAT != 0)
            .count();
        assert_eq!(late, 0, "HEARTBEAT after DESTROY: {got:?}");
    })
    .await;
}

#[tokio::test]
async fn drop_with_silent_peer_frees_before_rto_limit() {
    with_timeout(TEST_LIMIT, async {
        // given: the peer never answers, the adapter holds unacknowledged data
        let Link {
            peer,
            stream,
            keep_alive: _keep,
        } = Link::new().await;
        let adapter = adapter_with_unsent(stream, &peer, None).await;
        peer.collect_for(Duration::from_millis(100)).await;
        // when: the adapter is dropped
        drop(adapter);
        // then: DESTROY within the bound
        let got = peer
            .collect_for(Duration::from_millis(DRAIN_BOUND_MS))
            .await;
        let destroy = got
            .iter()
            .find(|p| is_destroy(p))
            .expect("no DESTROY in time");
        assert!(destroy.0 <= DRAIN_BOUND_MS, "DESTROY at {} ms", destroy.0);
    })
    .await;
}

#[tokio::test]
async fn new_data_after_drop_gets_destroy_not_ack() {
    with_timeout(TEST_LIMIT, async {
        // given: the adapter is dropped with data still queued
        let Link {
            peer,
            stream,
            keep_alive: _keep,
        } = Link::new().await;
        let adapter = adapter_with_unsent(stream, &peer, None).await;
        peer.collect_for(Duration::from_millis(100)).await;
        drop(adapter);
        peer.collect_for(Duration::from_millis(100)).await;
        // when: the peer sends new data at the next expected number
        peer.send(&data_packet(0, b"new")).await;
        // then: within 300 ms there is DESTROY and no ACK that covers the data
        let got = peer.collect_for(Duration::from_millis(300)).await;
        assert!(got.iter().any(is_destroy), "no DESTROY: {got:?}");
        let acked = got
            .iter()
            .filter(|(_, p)| flags_of(p) & FLAG_DESTROY == 0 && ack_of(p) > 0)
            .count();
        assert_eq!(acked, 0, "new data was acknowledged: {got:?}");
    })
    .await;
}

#[tokio::test]
async fn destroy_repeats_exactly_three_times() {
    with_timeout(TEST_LIMIT, async {
        // given: a forced close after the drain bound, the peer stays silent
        let Link {
            peer,
            stream,
            keep_alive: _keep,
        } = Link::new().await;
        let adapter = adapter_with_unsent(stream, &peer, None).await;
        peer.collect_for(Duration::from_millis(100)).await;
        drop(adapter);
        // when: 10 s pass
        let got = peer.collect_for(Duration::from_secs(10)).await;
        // then: exactly three DESTROY, at least a second apart
        let times: Vec<u64> = got.iter().filter(|p| is_destroy(p)).map(|p| p.0).collect();
        assert_eq!(times.len(), 3, "DESTROY times: {times:?}");
        for pair in times.windows(2) {
            assert!(pair[1] - pair[0] >= 990, "gap too short: {times:?}");
        }
    })
    .await;
}

#[tokio::test]
async fn duplicate_after_drop_is_still_acked() {
    with_timeout(TEST_LIMIT, async {
        // given: the stream took one packet in order, then the adapter was dropped with data queued
        let Link {
            peer,
            stream,
            keep_alive: _keep,
        } = Link::new().await;
        let adapter = adapter_with_unsent(stream, &peer, None).await;
        peer.send(&data_packet(0, b"old")).await;
        peer.collect_for(Duration::from_millis(200)).await;
        drop(adapter);
        peer.collect_for(Duration::from_millis(100)).await;
        // when: the peer repeats the old number
        peer.send(&data_packet(0, b"old")).await;
        // then: it is acknowledged, not answered with DESTROY
        let got = peer.collect_for(Duration::from_millis(300)).await;
        assert!(
            got.iter()
                .any(|(_, p)| flags_of(p) & FLAG_DESTROY == 0 && ack_of(p) >= 1),
            "no ACK for the duplicate: {got:?}"
        );
    })
    .await;
}

#[tokio::test]
async fn drop_without_runtime_sends_destroy_at_once() {
    with_timeout(TEST_LIMIT, async {
        // given: the peer advertised window 0, the adapter holds 64 KiB it cannot send
        let Link {
            peer,
            stream,
            keep_alive: _keep,
        } = Link::new().await;
        let adapter = adapter_with_unsent(stream, &peer, Some(0)).await;
        peer.collect_for(Duration::from_millis(100)).await;
        // when: the adapter is dropped on a thread without a runtime
        let joined = std::thread::spawn(move || drop(adapter)).join();
        assert!(joined.is_ok(), "drop panicked");
        // then: DESTROY arrives at once and no HEARTBEAT follows it
        let got = peer.collect_for(Duration::from_millis(500)).await;
        assert!(got.iter().any(is_destroy), "no DESTROY: {got:?}");
        let later = peer.collect_for(Duration::from_secs(3)).await;
        let heartbeats = later
            .iter()
            .filter(|(_, p)| flags_of(p) & FLAG_HEARTBEAT != 0)
            .count();
        assert_eq!(heartbeats, 0, "HEARTBEAT after DESTROY: {later:?}");
    })
    .await;
}

#[tokio::test]
async fn clean_close_sends_no_destroy() {
    with_timeout(TEST_LIMIT, async {
        // given: both ENDs are exchanged and acknowledged, the adapter has read EOF
        let Link {
            peer,
            stream,
            keep_alive: _keep,
        } = Link::new().await;
        let mut adapter = stream.into_async_stream();
        peer.send(&end_packet(0)).await;
        let mut rest = Vec::new();
        adapter.read_to_end(&mut rest).await.expect("read");
        let ack_our_end = async {
            loop {
                let (_, packet) = peer
                    .recv_within(Duration::from_secs(2))
                    .await
                    .expect("no END from the stream");
                if flags_of(&packet) & FLAG_END != 0 {
                    peer.send(&ack_packet(1, OPEN_WINDOW, &[])).await;
                    break;
                }
            }
        };
        let (shutdown, ()) = tokio::join!(adapter.shutdown(), ack_our_end);
        shutdown.expect("shutdown");
        // when: the adapter is dropped
        drop(adapter);
        // then: no DESTROY follows
        let got = peer.collect_for(Duration::from_secs(3)).await;
        assert!(
            !got.iter().any(is_destroy),
            "DESTROY after a clean close: {got:?}"
        );
    })
    .await;
}
