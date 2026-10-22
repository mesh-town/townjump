//! Route change (`change_remote`): immediate and deferred completion, in-flight packets keep the old route, errors, socket tables.

use super::test_support::*;
use super::*;
use crate::error::UdxError;
use crate::native::runtime::UdxRuntime;
use crate::native::socket::UdxSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::Poll;
use std::time::Duration;

fn remote_id_of(packet: &[u8]) -> u32 {
    u32::from_le_bytes(packet[4..8].try_into().unwrap())
}

fn err_text<T>(r: Result<T>) -> String {
    r.err().map(|e| e.to_string()).unwrap_or_default()
}

/// Writes `data` without an answer: the packet reaches `raw` and stays unacknowledged.
async fn write_unanswered(rig: &Rig, data: &[u8]) {
    let mut write = Box::pin(rig.stream.write(data));
    let got = tokio::select! {
        got = recv_raw(&rig.r1) => got,
        _ = &mut write => None,
    };
    assert!(got.is_some(), "the packet did not reach the peer");
}

// ── T-udx-1 ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn change_applies_at_once_when_nothing_in_flight() {
    // given: everything written is acknowledged
    let rig = connected_pair_two_sockets().await;
    write_acked(&rig.stream, &rig.r1, b"a").await;
    let control = rig.stream.control();
    // when
    let mut changed = control.change_remote(&rig.s2, 200, rig.raddr2).unwrap();
    // then: ready on the first poll, the route is new
    assert!(matches!(poll_once(&mut changed), Poll::Ready(Ok(()))));
    assert_eq!(control.remote(), Some((rig.raddr2, 200)));
    assert_eq!(rig.stream.effective_mtu(), MTU_BASE);
    let mut write = Box::pin(rig.stream.write(b"b"));
    let got = tokio::select! {
        got = recv_raw(&rig.r2) => got,
        _ = &mut write => None,
    };
    let (packet, src) = got.expect("no DATA on the new peer");
    assert_eq!(src, rig.addr2);
    assert_eq!(remote_id_of(&packet), 200);
}

// ── T-udx-2 ──────────────────────────────────────────────────────────────────

fn route_of(inner: &StreamInner, seq: u32) -> Option<SocketAddr> {
    inner.outgoing.get(&Seq(seq)).and_then(|p| p.remote_addr)
}

async fn keeps_old_route(first: u32) {
    let old: SocketAddr = "127.0.0.1:1111".parse().unwrap();
    let new: SocketAddr = "127.0.0.1:2222".parse().unwrap();
    let udp = Arc::new(tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let mut inner = stream_inner();
    inner.remote_addr = Some(old);
    inner.remote_id = 5;
    // given: 3 sent, 1 queued (old route)
    with_sent(&mut inner, first, 3);
    let queued = first.wrapping_add(3);
    let packet = build_data_packet(5, queued, 0, b"q");
    inner.queue_for_send(vec![(queued, packet)], old);
    inner.next_seq = queued.wrapping_add(1);
    // when: the route changes, then 1 more is queued for the new one
    let change_seq = inner.next_seq;
    let mut done = inner.begin_change(udp, 9, new).expect("deferred");
    let after = change_seq;
    inner.queue_for_send(vec![(after, build_data_packet(9, after, 0, b"n"))], new);
    inner.next_seq = after.wrapping_add(1);
    let batch = inner.take_send_batch();
    let lost = inner
        .outgoing
        .mark_rto_lost(Duration::ZERO, Instant::now() + Duration::from_secs(3600));
    let rtx = inner.take_send_batch();
    // then: the queue went by its own addresses, no MTU probe during the change
    let sent: Vec<_> = batch.packets.iter().map(|p| p.1).collect();
    assert_eq!(sent, vec![old, new]);
    assert_eq!(inner.mtu_state, MtuState::Base);
    assert!(!inner.outgoing.get(&Seq(after)).unwrap().is_mtu_probe);
    // and the retransmissions: the old four to the old address, the new one to the new
    assert_eq!(lost.len(), 5);
    let routes: Vec<_> = rtx.packets.iter().map(|p| p.1).collect();
    assert_eq!(routes, vec![old, old, old, old, new]);
    assert_eq!(remote_id_of(&rtx.packets[0].0), 5);
    assert_eq!(route_of(&inner, after), None);
    // the change completes on the ACK of `seq_of_change`, not before
    inner.on_ack_packet(change_seq.wrapping_sub(1), DEFAULT_RWND, &[]);
    assert!(inner.change.is_some());
    assert!(done.try_recv().is_err());
    inner.on_ack_packet(change_seq, DEFAULT_RWND, &[]);
    assert!(inner.change.is_none());
    assert!(done.try_recv().is_ok());
}

#[tokio::test]
async fn in_flight_packets_keep_old_route() {
    keeps_old_route(0).await;
}

#[tokio::test]
async fn in_flight_packets_keep_old_route_across_wraparound() {
    keeps_old_route(u32::MAX - 1).await;
}

// ── T-udx-3 ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn second_change_while_changing_fails() {
    let rig = connected_pair_two_sockets().await;
    write_unanswered(&rig, b"a").await;
    let control = rig.stream.control();
    let _first = control.change_remote(&rig.s2, 200, rig.raddr2).unwrap();
    let second = control.change_remote(&rig.s1, 300, rig.raddr1);
    assert!(err_text(second).contains("Remote already changing"));
    assert_eq!(control.remote(), Some((rig.raddr2, 200)));
}

#[tokio::test]
async fn change_before_connect_fails_not_connected() {
    let runtime = UdxRuntime::new().unwrap();
    let (s1, _) = bound_socket(&runtime).await;
    let stream = runtime.create_stream(7).await.unwrap();
    let control = stream.control();
    let got = control.change_remote(&s1, 1, "127.0.0.1:5000".parse().unwrap());
    assert!(err_text(got).contains("Not yet connected"));
}

#[tokio::test]
async fn port_zero_is_invalid_input() {
    let rig = connected_pair_two_sockets().await;
    let control = rig.stream.control();
    let got = control.change_remote(&rig.s2, 200, "127.0.0.1:0".parse().unwrap());
    assert!(matches!(got, Err(UdxError::Io(e)) if e.kind() == std::io::ErrorKind::InvalidInput));
    assert_eq!(control.remote(), Some((rig.raddr1, REMOTE_ID)));
}

#[tokio::test]
async fn change_after_close_is_stream_closed() {
    let rig = connected_pair_two_sockets().await;
    let control = rig.stream.control();
    rig.stream.destroy().await.unwrap();
    let got = control.change_remote(&rig.s2, 200, rig.raddr2);
    assert!(matches!(got, Err(UdxError::StreamClosed)));
}

#[tokio::test]
async fn pending_change_fails_on_destroy() {
    let rig = connected_pair_two_sockets().await;
    write_unanswered(&rig, b"a").await;
    let control = rig.stream.control();
    let mut changed = control.change_remote(&rig.s2, 200, rig.raddr2).unwrap();
    assert!(matches!(poll_once(&mut changed), Poll::Pending));
    rig.stream.destroy().await.unwrap();
    assert!(matches!(changed.await, Err(UdxError::StreamClosed)));
}

#[tokio::test]
async fn dropped_remote_changed_still_settles() {
    let rig = connected_pair_two_sockets().await;
    write_unanswered(&rig, b"a").await;
    let control = rig.stream.control();
    drop(control.change_remote(&rig.s2, 200, rig.raddr2).unwrap());
    assert!(control.change_remote(&rig.s1, 300, rig.raddr1).is_err());
    rig.r1
        .send_to(&peer_packet(0, 0, 1, b""), rig.addr1)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(control.change_remote(&rig.s1, 300, rig.raddr1).is_ok());
}

// ── T-udx-4 ──────────────────────────────────────────────────────────────────

async fn read_one(stream: &mut UdxStream) -> Vec<u8> {
    let got = tokio::time::timeout(Duration::from_secs(1), stream.read()).await;
    got.expect("read timed out").unwrap().expect("eof")
}

async fn wait_tables(sockets: &[&UdxSocket], n: usize) -> bool {
    for _ in 0..100 {
        if sockets.iter().all(|s| table_len(s) == n) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    false
}

#[tokio::test]
async fn old_socket_still_delivers_entries_dropped_on_close() {
    let mut rig = connected_pair_two_sockets().await;
    let control = rig.stream.control();
    drop(control.change_remote(&rig.s2, 200, rig.raddr2).unwrap());
    assert_eq!((table_len(&rig.s1), table_len(&rig.s2)), (1, 1));
    rig.r1
        .send_to(&peer_packet(FLAG_DATA, 0, 0, b"old"), rig.addr1)
        .await
        .unwrap();
    rig.r2
        .send_to(&peer_packet(FLAG_DATA, 1, 0, b"new"), rig.addr2)
        .await
        .unwrap();
    assert_eq!(read_one(&mut rig.stream).await, b"old");
    assert_eq!(read_one(&mut rig.stream).await, b"new");
    // the same socket again: no new entries
    drop(control.change_remote(&rig.s1, 300, rig.raddr1).unwrap());
    assert_eq!((table_len(&rig.s1), table_len(&rig.s2)), (1, 1));
    rig.stream.destroy().await.unwrap();
    assert!(
        wait_tables(&[&rig.s1, &rig.s2], 0).await,
        "entries left after destroy"
    );
}

#[tokio::test]
async fn entries_dropped_with_the_adapter() {
    let rig = connected_pair_two_sockets().await;
    let control = rig.stream.control();
    drop(control.change_remote(&rig.s2, 200, rig.raddr2).unwrap());
    let adapter = rig.stream.into_async_stream();
    assert_eq!((table_len(&rig.s1), table_len(&rig.s2)), (1, 1));
    drop(adapter);
    assert!(
        wait_tables(&[&rig.s1, &rig.s2], 0).await,
        "entries left after the adapter drop"
    );
}

#[tokio::test]
async fn connect_after_firewall_hook_moves_socket() {
    let rig = connected_pair_two_sockets().await;
    let (s3, _) = bound_socket(&rig.runtime).await;
    let (s4, addr4) = bound_socket(&rig.runtime).await;
    let stream = rig.runtime.create_stream(5).await.unwrap();
    let fired = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&fired);
    stream
        .set_firewall_hook(&s3, 100, move |_, _, _| {
            flag.store(true, Ordering::SeqCst);
            true
        })
        .unwrap();
    let mut write = Box::pin(stream.write(b"early"));
    tokio::select! { _ = &mut write => panic!("write finished"), _ = tokio::time::sleep(Duration::from_millis(50)) => {} }
    // when: connect on another socket
    stream
        .connect(&s4, 100, rig.raddr1)
        .await
        .expect("connect moves the stream");
    // then: the queued data leaves through the new socket, the old entry stays, the hook never ran
    let (packet, src) = recv_raw(&rig.r1).await.expect("no packet");
    assert_eq!(src, addr4);
    assert_eq!(remote_id_of(&packet), 100);
    assert!(!fired.load(Ordering::SeqCst));
    assert_eq!((table_len(&s3), table_len(&s4)), (1, 1));
}

#[tokio::test]
async fn foreign_id_on_new_socket_fails() {
    let rig = connected_pair_two_sockets().await;
    let other = rig.runtime.create_stream(LOCAL_ID).await.unwrap();
    other.register(&rig.s2).unwrap();
    let control = rig.stream.control();
    let got = control.change_remote(&rig.s2, 200, rig.raddr2);
    assert!(err_text(got).contains("stream id taken on target socket"));
    assert_eq!(control.remote(), Some((rig.raddr1, REMOTE_ID)));
    assert_eq!(table_len(&rig.s2), 1);
}

// ── peer address of an async stream ──────────────────────────────────────────

#[tokio::test]
async fn async_stream_peer_follows_change_remote() {
    // given: a connected stream turned into an async stream, with a peer handle taken first
    let rig = connected_pair_two_sockets().await;
    let control = rig.stream.control();
    let raddr1 = control.remote().map(|(addr, _)| addr);
    let async_stream = rig.stream.into_async_stream();
    let peer = async_stream.peer();
    assert_eq!(peer.remote().map(|(addr, _)| addr), raddr1);
    // when: the route changes
    let _changed = control.change_remote(&rig.s2, 200, rig.raddr2).unwrap();
    // then: the handle shows the new address while the stream is owned elsewhere
    assert_eq!(peer.remote(), Some((rig.raddr2, 200)));
    assert_eq!(peer.local().map(|a| a.port()), Some(rig.addr2.port()));
}
