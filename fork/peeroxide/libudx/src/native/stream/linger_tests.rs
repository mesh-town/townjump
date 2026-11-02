// JS: libudx/src/udx.c
//! The tombstone's decisions without the network: `Tomb::reply`.

use super::*;

const PEER: &str = "127.0.0.1:4000";
const FINAL_SEQ: u32 = 10;
const FINAL_ACK: u32 = 100;

fn peer() -> SocketAddr {
    PEER.parse().expect("addr")
}

fn tomb(end_acked: bool, remote_ended: bool, final_ack: u32) -> Tomb {
    Tomb {
        remote_addr: peer(),
        remote_id: 7,
        udp: Weak::new(),
        final_seq: FINAL_SEQ,
        final_ack,
        end_acked,
        remote_ended,
        rto: Duration::from_millis(1000),
        sent: 0,
        until: tokio::time::Instant::now() + LINGER_MIN,
    }
}

fn packet(flags: u8, seq: u32) -> Header {
    Header {
        type_flags: flags,
        data_offset: 0,
        remote_id: 5,
        recv_window: 0,
        seq,
        ack: 0,
    }
}

fn parsed(reply: &Reply) -> Header {
    match reply {
        Reply::Ack(buf) | Reply::Destroy(buf) => Header::decode(buf).expect("header"),
        Reply::Silent => panic!("silence"),
    }
}

#[tokio::test(start_paused = true)]
async fn foreign_address_gets_silence() {
    let other: SocketAddr = "127.0.0.1:4001".parse().expect("addr");
    for t in [tomb(false, false, FINAL_ACK), tomb(true, true, FINAL_ACK)] {
        for flags in [FLAG_DATA, FLAG_END, FLAG_DATA | FLAG_END] {
            assert_eq!(t.reply(&packet(flags, 0), other), Reply::Silent);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn destroy_ack_and_heartbeat_get_silence() {
    for t in [tomb(false, false, FINAL_ACK), tomb(true, true, FINAL_ACK)] {
        for flags in [
            FLAG_DESTROY,
            FLAG_DESTROY | FLAG_DATA,
            FLAG_ACK_ONLY,
            FLAG_HEARTBEAT,
            FLAG_MESSAGE,
        ] {
            assert_eq!(t.reply(&packet(flags, FINAL_ACK), peer()), Reply::Silent);
        }
    }
}

/// A pure ACK has no flags at all.
const FLAG_ACK_ONLY: u8 = 0;

#[tokio::test(start_paused = true)]
async fn unfinished_stream_answers_data_with_destroy() {
    let reply = tomb(false, false, FINAL_ACK).reply(&packet(FLAG_DATA, 0), peer());
    assert!(matches!(reply, Reply::Destroy(_)));
    let hdr = parsed(&reply);
    assert_eq!(hdr.type_flags, FLAG_DESTROY);
    assert_eq!((hdr.remote_id, hdr.seq, hdr.ack), (7, FINAL_SEQ, FINAL_ACK));
}

#[tokio::test(start_paused = true)]
async fn ended_both_ways_acks_repeated_end() {
    let reply = tomb(true, true, FINAL_ACK).reply(&packet(FLAG_END, FINAL_ACK - 1), peer());
    assert!(matches!(reply, Reply::Ack(_)));
    assert_eq!(parsed(&reply).ack, FINAL_ACK);
}

#[tokio::test(start_paused = true)]
async fn ended_both_ways_acks_data_beyond_final_ack() {
    let reply = tomb(true, true, FINAL_ACK).reply(&packet(FLAG_DATA, FINAL_ACK + 4), peer());
    assert!(matches!(reply, Reply::Ack(_)));
    assert_eq!(parsed(&reply).ack, FINAL_ACK + 5);
}

#[tokio::test(start_paused = true)]
async fn own_end_acked_and_peer_end_in_order_gets_ack() {
    let reply = tomb(true, false, FINAL_ACK).reply(&packet(FLAG_END, FINAL_ACK), peer());
    assert!(matches!(reply, Reply::Ack(_)));
    assert_eq!(parsed(&reply).ack, FINAL_ACK + 1);
}

#[tokio::test(start_paused = true)]
async fn own_end_acked_and_peer_end_with_a_gap_gets_destroy() {
    let reply = tomb(true, false, FINAL_ACK).reply(&packet(FLAG_END, FINAL_ACK + 3), peer());
    assert!(matches!(reply, Reply::Destroy(_)));
}

#[tokio::test(start_paused = true)]
async fn ack_number_wraps_past_u32_max() {
    let t = tomb(true, false, u32::MAX);
    let reply = t.reply(&packet(FLAG_END, u32::MAX), peer());
    assert!(matches!(reply, Reply::Ack(_)));
    assert_eq!(parsed(&reply).ack, 0);
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

/// The debug line that log analysis greps: a DESTROY reply of the tombstone with id, address and kind of reply.
#[tokio::test]
async fn destroy_reply_is_logged_at_debug() {
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
    let mut t = tomb(false, false, FINAL_ACK);
    t.remote_addr = peer.local_addr().expect("peer addr");
    t.udp = Arc::downgrade(&udp);
    let (tx, rx) = mpsc::unbounded_channel();
    let guard = TableEntry {
        streams: Arc::default(),
        local_id: 5,
        tx: tx.downgrade(),
    };
    let data = Header {
        type_flags: FLAG_DATA,
        ..packet(FLAG_DATA, 0)
    };
    let mut bytes = data.encode().to_vec();
    bytes.push(1);
    tx.send(IncomingPacket {
        data: bytes,
        addr: t.remote_addr,
        stamp: None,
    })
    .expect("send");
    drop(tx);
    linger(t, rx, guard).await;
    let mut got = [0u8; 64];
    let (len, _) = peer.recv_from(&mut got).await.expect("DESTROY on the wire");
    assert_eq!(got[2], FLAG_DESTROY, "len {len}");
    let log = String::from_utf8(capture.0.lock().expect("capture").clone()).expect("utf8");
    assert!(
        log.contains("tombstone reply")
            && log.contains("reply=\"destroy\"")
            && log.contains("local_id=5"),
        "log: {log}"
    );
}
