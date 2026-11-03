//! Dropped adapters with unsent data must release their queues and their state.

use std::sync::{Mutex, Weak};
use std::time::Duration;

use tokio::io::AsyncWriteExt;

use super::UdxAsyncStream;
use crate::native::runtime::UdxRuntime;
use crate::native::stream::StreamInner;
use crate::native::stream::test_support::{bound_socket, queued_bytes};

const PAIRS: u32 = 5;
const CHUNK: usize = 65_536;

fn report(label: &str, weaks: &[Weak<Mutex<StreamInner>>]) -> usize {
    let mut alive = 0;
    let mut bytes = 0;
    for weak in weaks {
        if let Some(inner) = weak.upgrade() {
            alive += 1;
            bytes += queued_bytes(&inner.lock().unwrap_or_else(|e| e.into_inner()));
        }
    }
    println!("{label}: alive={alive} queued_bytes={bytes}");
    alive
}

#[tokio::test(start_paused = true)]
async fn dropped_streams_release_queues() {
    // given: five writers whose peers never read and advertise the smallest window
    let runtime = UdxRuntime::new().expect("runtime");
    let (sock_a, addr_a) = bound_socket(&runtime).await;
    let (sock_b, addr_b) = bound_socket(&runtime).await;
    let mut readers = Vec::new();
    let mut writers: Vec<UdxAsyncStream> = Vec::new();
    for i in 0..PAIRS {
        let a = runtime.create_stream(1 + i).await.expect("stream a");
        let b = runtime.create_stream(100 + i).await.expect("stream b");
        a.connect(&sock_a, 100 + i, addr_b)
            .await
            .expect("connect a");
        b.connect(&sock_b, 1 + i, addr_a).await.expect("connect b");
        b.set_recv_window_max(65_536);
        readers.push(b.into_async_stream());
        writers.push(a.into_async_stream());
    }
    for writer in &mut writers {
        for _ in 0..8 {
            let write = writer.write_all(&[7u8; CHUNK]);
            if tokio::time::timeout(Duration::from_millis(200), write)
                .await
                .is_err()
            {
                break;
            }
        }
    }
    let weaks: Vec<_> = writers
        .iter()
        .map(|w| std::sync::Arc::downgrade(&w.inner))
        .collect();
    // when: the writers are dropped
    drop(writers);
    // then: the queues are released by 11 s
    report("t=0s", &weaks);
    tokio::time::sleep(Duration::from_secs(5)).await;
    report("t=5s", &weaks);
    tokio::time::sleep(Duration::from_secs(6)).await;
    let alive = report("t=11s", &weaks);
    assert_eq!(alive, 0, "streams still alive at 11 s");
    drop(readers);
}

#[test]
fn drain_limit_is_four_rto_within_two_and_ten_seconds() {
    use super::super::close::drain_limit;
    let ms = Duration::from_millis;
    assert_eq!(drain_limit(ms(100)), Duration::from_secs(2));
    assert_eq!(drain_limit(ms(1000)), Duration::from_secs(4));
    assert_eq!(drain_limit(ms(30_000)), Duration::from_secs(10));
}
