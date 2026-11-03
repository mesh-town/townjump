#![deny(clippy::all)]

//! Memory held by streams whose reader is slow: `cargo test --release -p libudx --test recv_memory -- --ignored --nocapture`.
//! Uses only the public read API, so the same file runs on any revision.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use common::{create_connected_pair, create_runtime, random_payload};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const PAIRS: usize = 5;
const WRITE_TOTAL: usize = 64 * 1024 * 1024;
const READ_RATE: usize = 60_000;

fn rss_kib() -> u64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("run ps");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn slow_readers_rss() {
    let runtime = create_runtime();
    let data = Arc::new(random_payload(WRITE_TOTAL));
    let before = rss_kib();
    let mut keep = Vec::new();
    let mut counters = Vec::new();
    for _ in 0..PAIRS {
        let (a, b, sa, sb) = create_connected_pair(&runtime).await;
        keep.push((sa, sb));
        let (mut writer, mut reader) = (a.into_async_stream(), b.into_async_stream());
        let (sent, read) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        counters.push((Arc::clone(&sent), Arc::clone(&read)));
        let data = Arc::clone(&data);
        tokio::spawn(async move {
            let mut offset = 0;
            while offset < WRITE_TOTAL {
                let end = (offset + 64 * 1024).min(WRITE_TOTAL);
                let write = writer.write(&data[offset..end]);
                if let Ok(Ok(n)) = tokio::time::timeout(Duration::from_secs(1), write).await {
                    offset += n;
                    sent.store(offset, Ordering::SeqCst);
                }
            }
            writer
        });
        tokio::spawn(async move {
            let mut buf = vec![0u8; READ_RATE / 10];
            loop {
                tokio::time::sleep(Duration::from_millis(100)).await;
                match reader.read(&mut buf).await {
                    Ok(n) if n > 0 => {
                        read.fetch_add(n, Ordering::SeqCst);
                    }
                    _ => break,
                }
            }
        });
    }
    tokio::time::sleep(Duration::from_secs(10)).await;
    let after = rss_kib();
    println!("rss before: {} KiB, after: {} KiB", before, after);
    for (i, (sent, read)) in counters.iter().enumerate() {
        let (sent, read) = (sent.load(Ordering::SeqCst), read.load(Ordering::SeqCst));
        println!(
            "stream {i}: written {sent}, read {read}, accepted - read {}",
            sent.saturating_sub(read)
        );
    }
}
