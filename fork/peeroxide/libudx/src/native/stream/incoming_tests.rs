//! Tests of the stream loop on a live socket: a write from idle goes out without a clock tick, an ACK opens the window via a timer (under `ackstep` — inline).
use super::*;
use crate::native::stream::test_support::stream_inner;
use crate::{UdxRuntime, UdxSocket, UdxStream};
use std::net::UdpSocket as StdUdp;
use std::time::Duration;
use tokio::time::Instant;

const SPINS: usize = 1000;

/// A live stream and its environment: stream address, peer socket, runtime owners.
struct Live {
    stream_addr: SocketAddr,
    peer: StdUdp,
    _keep: (UdxRuntime, UdxSocket),
}

async fn live(local_id: u32) -> (Live, UdxStream) {
    let runtime = UdxRuntime::new().unwrap();
    let socket = runtime.create_socket().await.unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let stream_addr = socket.local_addr().await.unwrap();
    let peer = StdUdp::bind("127.0.0.1:0").unwrap();
    peer.set_nonblocking(true).unwrap();
    let stream = runtime.create_stream(local_id).await.unwrap();
    let peer_addr = peer.local_addr().unwrap();
    stream.connect(&socket, 7, peer_addr).await.unwrap();
    let keep = (runtime, socket);
    (
        Live {
            stream_addr,
            peer,
            _keep: keep,
        },
        stream,
    )
}

/// Up to `spins` attempts to read a datagram; only `yield_now` between them, the clock does not run.
async fn spin_recv(peer: &StdUdp, spins: usize) -> Option<Vec<u8>> {
    let mut buf = [0u8; 2048];
    for _ in 0..spins {
        match peer.recv_from(&mut buf) {
            Ok((len, _)) => return Some(buf[..len].to_vec()),
            Err(_) => tokio::task::yield_now().await,
        }
    }
    None
}

/// Reads a datagram, advancing the clock by a millisecond between attempts (the pacing timer fires only on clock movement).
async fn recv_advancing(peer: &StdUdp, rounds: usize) -> Option<Vec<u8>> {
    for _ in 0..rounds {
        if let Some(packet) = spin_recv(peer, 50).await {
            return Some(packet);
        }
        tokio::time::advance(Duration::from_millis(1)).await;
    }
    None
}

fn is_data(packet: &[u8]) -> bool {
    Header::decode(packet).is_ok_and(|h| h.has_flag(FLAG_DATA))
}

#[tokio::test(start_paused = true)]
async fn write_goes_out_without_clock_moving() {
    tokio::time::advance(Duration::from_micros(500)).await;
    let t0 = Instant::now();
    // given: a live stream, the clock frozen
    let (live, stream) = live(5).await;
    // when
    tokio::spawn(async move { stream.write(b"x").await });
    let got = spin_recv(&live.peer, SPINS).await;
    // then
    assert!(
        got.is_some_and(|p| is_data(&p)),
        "DATA went out without a tick"
    );
    assert_eq!(Instant::now(), t0);
}

#[tokio::test(start_paused = true)]
async fn ack_opens_closed_window_via_timer() {
    tokio::time::advance(Duration::from_micros(500)).await;
    // given: a write of (cwnd + 1) packets, cwnd = 10
    let (live, stream) = live(5).await;
    let len = 11 * stream_inner().max_payload();
    tokio::spawn(async move { stream.write(&vec![1u8; len]).await });
    let mut first_seq = None;
    for _ in 0..10 {
        let packet = spin_recv(&live.peer, SPINS).await.expect("first 10 DATA");
        let seq = Header::decode(&packet).unwrap().seq;
        first_seq.get_or_insert(seq);
    }
    assert!(
        spin_recv(&live.peer, SPINS).await.is_none(),
        "window closed"
    );
    // when: the peer acks all 10
    let ack = wire::ack_packet(5, 0, first_seq.unwrap().wrapping_add(10), &[]);
    live.peer.send_to(&ack, live.stream_addr).unwrap();
    // then: the ACK wakes the sender via the pacing timer — without clock movement the 11th DATA does not go out, with it it does;
    // under `UDX_FIX=pace,ackstep` the step runs inline after the ACK, and the 11th DATA goes out without clock movement
    if crate::native::stream::fix::mode().ackstep {
        let next = spin_recv(&live.peer, SPINS).await;
        assert!(
            next.is_some_and(|p| is_data(&p)),
            "11th DATA went out without a tick"
        );
        return;
    }
    assert!(
        spin_recv(&live.peer, SPINS).await.is_none(),
        "clock stopped"
    );
    let next = recv_advancing(&live.peer, 200).await;
    assert!(next.is_some_and(|p| is_data(&p)), "11th DATA went out");
}

#[tokio::test(start_paused = true)]
async fn write_in_flight_waits_for_timer() {
    tokio::time::advance(Duration::from_micros(500)).await;
    // given: the first write went out and is not acked (one packet in flight)
    let (live, stream) = live(5).await;
    let stream = std::sync::Arc::new(stream);
    let first = stream.clone();
    tokio::spawn(async move { first.write(b"x").await });
    assert!(spin_recv(&live.peer, SPINS).await.is_some(), "first DATA");
    // when: a second write with a packet in flight
    tokio::spawn(async move { stream.write(b"y").await });
    // then: it does not go out without clock movement, with it it does
    assert!(
        spin_recv(&live.peer, SPINS).await.is_none(),
        "clock stopped"
    );
    let second = recv_advancing(&live.peer, 200).await;
    assert!(
        second.is_some_and(|p| is_data(&p)),
        "second DATA went out on the timer"
    );
}
