// JS: libudx/src/udx.c
//! Tombstone lifetime: `tomb_ttl` and the reply to a late END inside / past the lifetime (paused clock).

use super::*;
use std::sync::Mutex;

const PEER_END_SEQ: u32 = 100;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[test]
fn ttl_floor_20s() {
    assert_eq!(tomb_ttl(ms(100)), Duration::from_secs(20));
}

#[test]
fn ttl_14_rto() {
    assert_eq!(tomb_ttl(ms(2000)), Duration::from_secs(28));
}

#[test]
fn ttl_cap_60s() {
    assert_eq!(tomb_ttl(ms(10_000)), Duration::from_secs(60));
    assert_eq!(tomb_ttl(Duration::MAX), Duration::from_secs(60));
}

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("capture").extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A tombstone of a stream with both ENDs exchanged (our final ACK was lost); the peer repeats its END
/// `delay` after the close. Returns the ACK number the peer got (if any) and the debug log.
async fn late_end(delay: Duration) -> (Option<u32>, String) {
    let capture = Capture::default();
    let sink = capture.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .with_writer(move || sink.clone())
        .finish();
    let _scope = tracing::subscriber::set_default(subscriber);
    let udp = Arc::new(
        tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("bind"),
    );
    let peer = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("bind peer");
    let rto = ms(1000);
    let t = Tomb {
        remote_addr: peer.local_addr().expect("peer addr"),
        remote_id: 7,
        udp: Arc::downgrade(&udp),
        final_seq: 10,
        final_ack: PEER_END_SEQ + 1,
        end_acked: true,
        remote_ended: true,
        rto,
        sent: 0,
        until: tokio::time::Instant::now() + tomb_ttl(rto),
    };
    let (tx, rx) = mpsc::unbounded_channel();
    let guard = TableEntry {
        streams: Arc::default(),
        local_id: 5,
        tx: tx.downgrade(),
    };
    let remote_addr = t.remote_addr;
    let task = tokio::spawn(linger(t, rx, guard));
    // let the task register its expiry timer, then run the clock and let the task see the expiry
    tokio::task::yield_now().await;
    tokio::time::advance(delay).await;
    tokio::task::yield_now().await;
    let end = Header {
        type_flags: FLAG_END,
        data_offset: 0,
        remote_id: 5,
        recv_window: 0,
        seq: PEER_END_SEQ,
        ack: 0,
    };
    let _ = tx.send(IncomingPacket {
        data: end.encode().to_vec(),
        addr: remote_addr,
        stamp: None,
    });
    // loopback delivery is real time; the paused clock must not run ahead, so poll with plain sleeps
    let mut got = [0u8; 64];
    let mut ack = None;
    for _ in 0..40 {
        tokio::task::yield_now().await;
        if let Ok((len, _)) = peer.try_recv_from(&mut got) {
            ack = Header::decode(&got[..len]).ok().map(|h| h.ack);
            break;
        }
        std::thread::sleep(ms(5));
    }
    drop(tx);
    task.abort();
    let log = String::from_utf8(capture.0.lock().expect("capture").clone()).expect("utf8");
    (ack, log)
}

#[tokio::test(start_paused = true)]
async fn reply_after_19s() {
    let (ack, log) = late_end(Duration::from_secs(19)).await;
    assert!(ack.is_some_and(|a| a > PEER_END_SEQ), "ack {ack:?}");
    assert!(log.contains("sent=1"), "log: {log}");
}

#[tokio::test(start_paused = true)]
async fn silence_after_21s() {
    let (ack, log) = late_end(Duration::from_secs(21)).await;
    assert_eq!(ack, None);
    assert!(!log.contains("tombstone reply"), "log: {log}");
}
