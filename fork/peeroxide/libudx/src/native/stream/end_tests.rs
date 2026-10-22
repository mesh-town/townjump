//! D8/END tests: bookkeeping of our END's number, "exactly once" and the reply ACK (when it goes out, when we stay silent).
use super::*;
use crate::native::stream::test_support::stream_inner;
use std::time::Duration;
use tokio::net::UdpSocket;

mod end_seq {
    use super::*;

    #[test]
    fn reserve_takes_the_next_number_and_remembers_it() {
        // given: next_seq = 4
        let mut inner = stream_inner();
        inner.next_seq = 4;
        // when
        let seq = inner.reserve_end_seq();
        // then
        assert_eq!((seq, inner.next_seq, inner.end_seq), (4, 5, Some(4)));
    }

    #[test]
    fn not_acked_before_the_ack_passes_it() {
        // given: END with number 4, acked up to 4 (the END itself not yet received)
        let mut inner = ended_stream(4);
        inner.remote_acked = 4;
        // when, then
        assert!(!inner.take_end_acked());
    }

    #[test]
    fn acked_once_when_the_ack_passes_it() {
        // given: END with number 4
        let mut inner = ended_stream(4);
        inner.remote_acked = 5;
        // when: first time yes, repeat no
        assert!(inner.take_end_acked());
        assert!(!inner.take_end_acked());
    }

    #[test]
    fn never_acked_without_our_end() {
        // given: END not ordered
        let mut inner = stream_inner();
        inner.remote_acked = 9;
        // when, then
        assert!(!inner.take_end_acked());
    }
}

mod final_ack {
    use super::*;

    #[tokio::test]
    async fn bare_ack_of_our_end_gets_one_ack() {
        // given: the peer has not finished yet
        let (inner, peer) = wired_stream().await;
        // when: the confirmation arrived in a packet without DATA and END
        inner.ack_confirmed_end(0);
        // then
        let packet = received(&peer).await.expect("no ACK arrived");
        assert_eq!(Header::decode(&packet).unwrap().type_flags, 0);
        assert!(received(&peer).await.is_none(), "no second ACK");
    }

    #[tokio::test]
    async fn packet_with_data_or_end_adds_no_second_ack() {
        // given: the ACK for DATA/END already went out from the data branch
        let (inner, peer) = wired_stream().await;
        // when
        for flags in [FLAG_DATA, FLAG_END, FLAG_DATA | FLAG_END] {
            inner.ack_confirmed_end(flags);
        }
        // then
        assert!(received(&peer).await.is_none());
    }

    #[tokio::test]
    async fn ended_peer_gets_no_ack() {
        // given: the peer finished earlier (JS closes the stream at this point)
        let (mut inner, peer) = wired_stream().await;
        inner.remote_ended = true;
        // when
        inner.ack_confirmed_end(0);
        // then
        assert!(received(&peer).await.is_none());
    }
}

mod remote_end {
    use super::*;

    #[test]
    fn delivered_end_marks_the_peer_as_ended() {
        // given: waiting for seq 0
        let mut inner = stream_inner();
        // when: END arrived in order
        inner.receive_data(0, Vec::new(), FLAG_END);
        // then
        assert!(inner.remote_ended);
    }

    #[test]
    fn buffered_end_does_not_yet() {
        // given: waiting for seq 0; END with seq 1 went into the buffer
        let mut inner = stream_inner();
        inner.receive_data(1, Vec::new(), FLAG_END);
        // when, then: while there is a hole, the end is not delivered
        assert!(!inner.remote_ended);
    }
}

// ── setup and helpers ────────────────────────────────────────────────────────

/// A stream that ordered END with number `seq`.
fn ended_stream(seq: u32) -> StreamInner {
    let mut inner = stream_inner();
    inner.next_seq = seq;
    inner.reserve_end_seq();
    inner
}

/// A stream with `remote_id = 7` with a real socket and a peer that reads what was sent.
async fn wired_stream() -> (StreamInner, UdpSocket) {
    let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let own = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut inner = stream_inner();
    inner.remote_id = 7;
    inner.remote_addr = Some(peer.local_addr().unwrap());
    inner.udp = Some(Arc::new(own));
    (inner, peer)
}

/// A packet that reached the peer within 200 ms; `None` means silence.
async fn received(peer: &UdpSocket) -> Option<Vec<u8>> {
    let mut buf = [0u8; 2048];
    let read = tokio::time::timeout(Duration::from_millis(200), peer.recv_from(&mut buf));
    let (len, _) = read.await.ok()?.unwrap();
    Some(buf[..len].to_vec())
}
