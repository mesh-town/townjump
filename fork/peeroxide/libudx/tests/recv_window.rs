#![deny(clippy::all)]

//! The receive window advertised by a stream reflects the bytes received and not yet read.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use common::raw_peer::{
    FLAG_SACK, Link, RawPeer, ack_of, data_packet, flags_of, heartbeat_packet, window_of,
};
use common::{create_connected_pair, create_runtime, with_timeout};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const MAX: u32 = 4_194_304;
const CHUNK: usize = 1000;
const FLOOR: u32 = 65_536;
const TEST_LIMIT: Duration = Duration::from_secs(60);

/// Sends `count` DATA packets of `CHUNK` bytes with numbers `first..first + count`, draining the
/// peer's socket between batches. Returns the highest `ack` and the last window seen.
async fn flood(peer: &RawPeer, first: u32, count: u32) -> (u32, u32) {
    let payload = vec![7u8; CHUNK];
    let (mut ack, mut window) = (0, MAX);
    for batch in (first..first + count).collect::<Vec<_>>().chunks(32) {
        for &seq in batch {
            peer.send(&data_packet(seq, &payload)).await;
        }
        for (_, packet) in peer.collect_for(Duration::from_millis(3)).await {
            ack = ack.max(ack_of(&packet));
            window = window_of(&packet);
        }
    }
    while let Some((_, packet)) = peer.recv_within(Duration::from_millis(300)).await {
        ack = ack.max(ack_of(&packet));
        window = window_of(&packet);
    }
    (ack, window)
}

/// Packets that arrive at the peer within `duration`.
async fn drain(peer: &RawPeer, duration: Duration) -> Vec<Vec<u8>> {
    let all = peer.collect_for(duration).await;
    all.into_iter().map(|(_, packet)| packet).collect()
}

/// Closes the window of a stream that never reads: more than `MAX` bytes arrive in order.
async fn fill_window(peer: &RawPeer) -> u32 {
    let count = MAX / CHUNK as u32 + 5;
    let (ack, _) = flood(peer, 0, count).await;
    assert_eq!(ack, count, "all packets were received in order");
    count
}

#[tokio::test]
async fn unread_bytes_shrink_advertised_window() {
    with_timeout(TEST_LIMIT, async {
        // given: 64 DATA packets by one thousand bytes, in order, the stream does not read
        let link = Link::new().await;
        // when: the last ACK arrives
        let (ack, window) = flood(&link.peer, 0, 64).await;
        // then: the window is the maximum less the unread bytes
        assert_eq!(ack, 64);
        assert_eq!(window, MAX - 64_000);
    })
    .await;
}

#[tokio::test]
async fn out_of_order_bytes_shrink_window_too() {
    with_timeout(TEST_LIMIT, async {
        // given: packets 1..=10 without packet 0
        let link = Link::new().await;
        let payload = vec![7u8; CHUNK];
        for seq in 1..=10 {
            link.peer.send(&data_packet(seq, &payload)).await;
        }
        // when: the ACKs arrive
        let packets = drain(&link.peer, Duration::from_millis(400)).await;
        // then: the last ACK carries a SACK and the window less the buffered bytes
        let last = packets.last().expect("an ACK arrives");
        assert_eq!(flags_of(last) & FLAG_SACK, FLAG_SACK);
        assert_eq!(ack_of(last), 0);
        assert_eq!(window_of(last), MAX - 10_000);
    })
    .await;
}

#[tokio::test]
async fn reading_reopens_window_with_pure_ack() {
    with_timeout(TEST_LIMIT, async {
        // given: the peer filled the window to zero and is silent
        let mut link = Link::new().await;
        let count = fill_window(&link.peer).await;
        let _ = drain(&link.peer, Duration::from_millis(300)).await;
        // when: the application reads more than a megabyte
        for _ in 0..1154 {
            let chunk = link.stream.read().await.expect("read").expect("data");
            assert_eq!(chunk.len(), CHUNK);
        }
        // then: an ACK without DATA with a window of at least a megabyte arrives
        let packets = drain(&link.peer, Duration::from_millis(500)).await;
        let reopened = packets
            .iter()
            .any(|packet| flags_of(packet) & 0x01 == 0 && window_of(packet) >= 1 << 20);
        assert!(
            reopened,
            "no window update after {count} packets: {packets:?}"
        );
    })
    .await;
}

#[tokio::test]
async fn heartbeat_at_zero_window_answers_current_window() {
    with_timeout(TEST_LIMIT, async {
        // given: the window is closed
        let mut link = Link::new().await;
        let count = fill_window(&link.peer).await;
        let _ = drain(&link.peer, Duration::from_millis(300)).await;
        // when: the peer probes
        link.peer.send(&heartbeat_packet(0, 0)).await;
        let first = link.peer.recv_within(Duration::from_secs(1)).await;
        // then: the ACK says the window is closed
        let (_, packet) = first.expect("an ACK arrives");
        assert_eq!(window_of(&packet), 0);
        // when: the application read everything and the peer probes again
        for _ in 0..count {
            link.stream.read().await.expect("read").expect("data");
        }
        let _ = drain(&link.peer, Duration::from_millis(300)).await;
        link.peer.send(&heartbeat_packet(0, 0)).await;
        let second = link.peer.recv_within(Duration::from_secs(1)).await;
        // then: the ACK says the window is open
        let (_, packet) = second.expect("an ACK arrives");
        assert_eq!(window_of(&packet), MAX);
    })
    .await;
}

#[tokio::test]
async fn slow_reader_bounds_writer() {
    with_timeout(TEST_LIMIT, async {
        // given: a pair of streams, the reader stands still
        let runtime = create_runtime();
        let (a, b, _sa, _sb) = create_connected_pair(&runtime).await;
        let (mut writer, mut reader) = (a.into_async_stream(), b.into_async_stream());
        let total = 32 * 1024 * 1024;
        let data = Arc::new(common::random_payload(total));
        let accepted = Arc::new(AtomicUsize::new(0));
        let task = {
            let (data, accepted) = (Arc::clone(&data), Arc::clone(&accepted));
            tokio::spawn(async move {
                let mut offset = 0;
                while offset < total {
                    let end = (offset + 64 * 1024).min(total);
                    let write = writer.write(&data[offset..end]);
                    if let Ok(Ok(n)) = tokio::time::timeout(Duration::from_secs(1), write).await {
                        offset += n;
                        accepted.store(offset, Ordering::SeqCst);
                    }
                }
                writer.shutdown().await.expect("shutdown");
                writer
            })
        };
        // when: three seconds pass without reading
        tokio::time::sleep(Duration::from_secs(3)).await;
        // then: the writer was held back by the window
        let held = accepted.load(Ordering::SeqCst);
        assert!(
            held <= 4_194_304 + 1_572_864,
            "writer got {held} bytes accepted"
        );
        // and: after reading, everything arrives intact
        let mut received = Vec::with_capacity(total);
        reader.read_to_end(&mut received).await.expect("read all");
        let _writer = task.await.expect("writer task");
        assert_eq!(received.len(), total);
        assert_eq!(Sha256::digest(&received), Sha256::digest(&data[..]));
    })
    .await;
}

#[tokio::test]
async fn out_of_order_past_window_is_dropped_in_order_is_kept() {
    with_timeout(TEST_LIMIT, async {
        // given: a window maximum of 64 KiB, filled with unread bytes
        let mut link = Link::new().await;
        link.stream.set_recv_window_max(FLOOR);
        let payload = vec![7u8; CHUNK];
        for seq in 0..66 {
            link.peer.send(&data_packet(seq, &payload)).await;
        }
        let acks = drain(&link.peer, Duration::from_millis(400)).await;
        assert_eq!(ack_of(acks.last().expect("ACKs")), 66);
        // when: an out-of-order packet arrives
        link.peer.send(&data_packet(71, &payload)).await;
        let acks = drain(&link.peer, Duration::from_millis(300)).await;
        // then: it is not acknowledged with a SACK
        let last = acks.last().expect("an ACK");
        assert_eq!(
            flags_of(last) & FLAG_SACK,
            0,
            "dropped packet is not SACKed"
        );
        // when: the packet in order arrives at the full window
        link.peer.send(&data_packet(66, &payload)).await;
        let acks = drain(&link.peer, Duration::from_millis(300)).await;
        // then: it is taken
        assert_eq!(ack_of(acks.last().expect("an ACK")), 67);
        assert_eq!(link.stream.stats().recv_queued, 67 * CHUNK as u64);
        let _ = link.stream.read().await;
    })
    .await;
}

#[tokio::test]
async fn recv_window_max_has_floor() {
    with_timeout(TEST_LIMIT, async {
        // given: a maximum below the floor
        let link = Link::new().await;
        link.stream.set_recv_window_max(1000);
        // when: the peer sends DATA
        link.peer.send(&data_packet(0, &[7u8; 100])).await;
        let acks = drain(&link.peer, Duration::from_millis(300)).await;
        // then: the window is the floor less the unread bytes
        assert_eq!(window_of(acks.last().expect("an ACK")), FLOOR - 100);
    })
    .await;
}

#[tokio::test]
async fn stats_recv_queued_tracks_unread() {
    with_timeout(TEST_LIMIT, async {
        // given: 64 packets unread
        let mut link = Link::new().await;
        flood(&link.peer, 0, 64).await;
        // then: all of it is queued
        assert_eq!(link.stream.stats().recv_queued, 64_000);
        // when: read
        for _ in 0..64 {
            link.stream.read().await.expect("read").expect("data");
        }
        // then: nothing is queued
        assert_eq!(link.stream.stats().recv_queued, 0);
    })
    .await;
}
