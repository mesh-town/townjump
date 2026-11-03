#![deny(clippy::all)]

//! The sender knobs: the send window cap and the write threshold, against a raw peer.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use common::raw_peer::{FLAG_DATA, Link, OPEN_WINDOW, RawPeer, ack_packet, flags_of, seq_of};
use common::{create_connected_pair, create_runtime, with_timeout};
use tokio::io::AsyncWriteExt;

const TEST_LIMIT: Duration = Duration::from_secs(60);

/// Distinct DATA numbers that reach the peer within `duration`, and the payload size of the largest packet.
async fn data_seen(peer: &RawPeer, duration: Duration) -> (BTreeMap<u32, usize>, usize) {
    let mut seqs = BTreeMap::new();
    let mut payload = 0;
    for (_, packet) in peer.collect_for(duration).await {
        if flags_of(&packet) & FLAG_DATA != 0 {
            seqs.insert(seq_of(&packet), packet.len() - 20);
            payload = payload.max(packet.len() - 20);
        }
    }
    (seqs, payload)
}

/// The cumulative ack for what has been received: the first missing number.
fn cumulative(received: &BTreeSet<u32>) -> u32 {
    (0..).find(|n| !received.contains(n)).unwrap_or(0)
}

/// Half-open ranges of received numbers above the cumulative ack, at most 16 of them.
fn sack_blocks(received: &BTreeSet<u32>, ack: u32) -> Vec<(u32, u32)> {
    let mut blocks: Vec<(u32, u32)> = Vec::new();
    for &seq in received.range(ack..) {
        match blocks.last_mut() {
            Some(last) if last.1 == seq => last.1 += 1,
            _ => blocks.push((seq, seq + 1)),
        }
    }
    blocks.truncate(16);
    blocks
}

#[tokio::test]
async fn send_window_max_limits_packets_before_the_first_ack() {
    with_timeout(TEST_LIMIT, async {
        // given: an 8 KiB cap, a peer that receives everything and acks nothing
        let link = Link::new().await;
        link.stream.set_send_window_max(8 * 1024);
        let Link {
            peer,
            stream,
            keep_alive: _keep_alive,
        } = link;
        let mut writer = stream.into_async_stream();
        let total = 1 << 20;
        let write = tokio::spawn(async move {
            let data = vec![9u8; total];
            let result = writer.write_all(&data).await;
            (writer, result)
        });
        // when
        let (mut seen, payload) = data_seen(&peer, Duration::from_millis(600)).await;
        // then: no more than ceil(8 KiB / payload) distinct packets
        assert!(!seen.is_empty());
        assert!(
            seen.len() <= (8 * 1024usize).div_ceil(payload),
            "{} packets of {payload}",
            seen.len()
        );
        // and: once acked, the transfer runs to the end
        let deadline = tokio::time::Instant::now() + Duration::from_secs(40);
        while tokio::time::Instant::now() < deadline
            && !(write.is_finished() && seen.values().sum::<usize>() >= total)
        {
            let received: BTreeSet<u32> = seen.keys().copied().collect();
            let ack = cumulative(&received);
            peer.send(&ack_packet(ack, OPEN_WINDOW, &sack_blocks(&received, ack)))
                .await;
            seen.extend(data_seen(&peer, Duration::from_millis(5)).await.0);
        }
        let (_writer, result) = write.await.expect("write task");
        result.expect("write");
        // MTU probes carry padding, so the byte sum may exceed the written amount.
        assert!(seen.values().sum::<usize>() >= total);
    })
    .await;
}

#[tokio::test]
async fn zero_write_watermark_does_not_stall_writes() {
    with_timeout(TEST_LIMIT, async {
        // given: two connected streams, the watermark of the writer is 0
        let runtime = create_runtime();
        let (sa, sb, _a, _b) = create_connected_pair(&runtime).await;
        sa.set_write_high_watermark(0);
        let mut writer = sa.into_async_stream();
        let mut reader = sb.into_async_stream();
        let drain = tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut got = 0usize;
            let mut buf = vec![0u8; 65_536];
            while got < 256 * 1024 {
                match reader.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => got += n,
                }
            }
            got
        });
        // when
        let data = vec![1u8; 256 * 1024];
        tokio::time::timeout(Duration::from_secs(5), writer.write_all(&data))
            .await
            .expect("write finishes in time")
            .expect("write");
        // then
        let got = tokio::time::timeout(Duration::from_secs(5), drain)
            .await
            .expect("reader finishes")
            .expect("reader task");
        assert_eq!(got, 256 * 1024);
    })
    .await;
}
