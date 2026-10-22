//! MESSAGE packets, `await_direct`, the path hook, `closed`: the path side of `StreamControl`, on raw UDP peers.

use super::test_support::*;
use super::*;
use std::task::Poll;
use std::time::Duration;

const QUIET: Duration = Duration::from_millis(150);

async fn fired(rx: &mut oneshot::Receiver<()>, within: Duration) -> bool {
    matches!(tokio::time::timeout(within, rx).await, Ok(Ok(())))
}

async fn read_or_none(stream: &mut UdxStream, within: Duration) -> Option<Vec<u8>> {
    let got = tokio::time::timeout(within, stream.read()).await.ok()?;
    got.ok().flatten()
}

async fn send_to_stream(
    raw: &tokio::net::UdpSocket,
    to: SocketAddr,
    flags: u8,
    seq: u32,
    body: &[u8],
) {
    raw.send_to(&peer_packet(flags, seq, 0, body), to)
        .await
        .unwrap();
}

// ── MESSAGE framing ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn try_send_message_frames_type_0x08() {
    // given: one DATA written and acknowledged (next seq 1), one DATA received from the peer (next remote seq 1)
    let mut rig = connected_pair_two_sockets().await;
    write_acked(&rig.stream, &rig.r1, b"a").await;
    send_to_stream(&rig.r1, rig.addr1, FLAG_DATA, 0, b"x").await;
    assert_eq!(
        read_or_none(&mut rig.stream, Duration::from_secs(1))
            .await
            .unwrap(),
        b"x"
    );
    let _ack = recv_raw(&rig.r1).await.expect("ack of the received DATA");
    let control = rig.stream.control();
    // when: an empty MESSAGE
    assert!(control.try_send_message(&[]).unwrap());
    // then: 20 bytes [255, 1, 0x08, 0, id LE, rwnd, seq, ack]; the sequence number is the next one, unchanged
    let (packet, src) = recv_raw(&rig.r1).await.expect("no MESSAGE");
    assert_eq!(src, rig.addr1);
    assert_eq!(packet.len(), 20);
    assert_eq!(packet[..4], [255, 1, FLAG_MESSAGE, 0]);
    assert_eq!(packet[4..8], REMOTE_ID.to_le_bytes());
    let header = Header::decode(&packet).unwrap();
    assert_eq!(
        (header.recv_window, header.seq, header.ack),
        (DEFAULT_RWND, 1, 1)
    );
    // and the next DATA carries the same seq
    let (data, _) = write_acked(&rig.stream, &rig.r1, b"b").await;
    assert_eq!(data[0..3], [255, 1, FLAG_DATA]);
    assert_eq!(Header::decode(&data).unwrap().seq, 1);
    // payload follows the header
    assert!(control.try_send_message(b"abc").unwrap());
    let (packet, _) = recv_raw(&rig.r1).await.expect("no MESSAGE with payload");
    assert_eq!((packet.len(), &packet[20..]), (23, &b"abc"[..]));
    assert_eq!(Header::decode(&packet).unwrap().seq, 2);
}

#[tokio::test]
async fn try_send_message_is_false_when_closed_or_not_connected() {
    let rig = connected_pair_two_sockets().await;
    let idle = rig.runtime.create_stream(7).await.unwrap();
    assert!(!idle.control().try_send_message(b"x").unwrap());
    let control = rig.stream.control();
    rig.stream.destroy().await.unwrap();
    assert!(!control.try_send_message(b"x").unwrap());
    assert!(
        recv_raw(&rig.r1)
            .await
            .is_none_or(|(p, _)| p[2] != FLAG_MESSAGE)
    );
}

// ── await_direct ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn await_direct_fires_on_current_remote_only() {
    let rig = connected_pair_two_sockets().await;
    let control = rig.stream.control();
    // given: a MESSAGE from the current remote that came before the call
    send_to_stream(&rig.r1, rig.addr1, FLAG_MESSAGE, 0, b"").await;
    tokio::time::sleep(QUIET).await;
    let (mut first, mut second) = (control.await_direct(), control.await_direct());
    // when: a MESSAGE and a DATA from a foreign address, a bare ACK from the current remote
    send_to_stream(&rig.r2, rig.addr1, FLAG_MESSAGE, 0, b"").await;
    send_to_stream(&rig.r2, rig.addr1, FLAG_DATA, 0, b"d").await;
    send_to_stream(&rig.r1, rig.addr1, 0, 0, b"").await;
    // then: nobody wakes, the earlier packet did not count
    assert!(!fired(&mut first, QUIET).await);
    // when: a MESSAGE from the current remote
    send_to_stream(&rig.r1, rig.addr1, FLAG_MESSAGE, 0, b"").await;
    // then: all waiters wake
    assert!(fired(&mut first, Duration::from_secs(1)).await);
    assert!(fired(&mut second, Duration::from_secs(1)).await);
    // a new waiter wakes on DATA
    let mut third = control.await_direct();
    send_to_stream(&rig.r1, rig.addr1, FLAG_DATA, 0, b"d").await;
    assert!(fired(&mut third, Duration::from_secs(1)).await);
}

#[tokio::test]
async fn await_direct_fails_on_a_closed_stream() {
    let rig = connected_pair_two_sockets().await;
    let control = rig.stream.control();
    let mut waiting = control.await_direct();
    rig.stream.destroy().await.unwrap();
    assert!(matches!(
        waiting.try_recv(),
        Err(oneshot::error::TryRecvError::Closed)
    ));
    let late = control.await_direct();
    assert!(tokio::time::timeout(QUIET, late).await.unwrap().is_err());
}

// ── path hook ──────────────────────────────────────────────────────────────────

type Calls = Arc<Mutex<Vec<SocketAddr>>>;

/// A hook that records its argument, asks `remote()` (a lock check) and answers `outcome`.
fn recording_hook(
    control: &StreamControl,
    outcome: impl Fn(SocketAddr) -> bool + Send + Sync + 'static,
) -> Calls {
    let calls = Calls::default();
    let (seen, ctl) = (Arc::clone(&calls), control.clone());
    control.set_path_hook(move |addr| {
        seen.lock().unwrap().push(addr);
        let _ = ctl.remote();
        outcome(addr)
    });
    calls
}

#[tokio::test]
async fn path_hook_drops_foreign_source() {
    let mut rig = connected_pair_two_sockets().await;
    let (r3, raddr3) = raw_peer().await;
    let control = rig.stream.control();
    let r2 = rig.raddr2;
    let calls = recording_hook(&control, move |addr| addr != r2);
    // when: DATA from R2 (hook says no)
    send_to_stream(&rig.r2, rig.addr1, FLAG_DATA, 0, b"no").await;
    // then: not read, not acknowledged to R2, the hook saw R2
    assert!(read_or_none(&mut rig.stream, QUIET).await.is_none());
    assert!(recv_raw(&rig.r2).await.is_none());
    assert_eq!(*calls.lock().unwrap(), vec![rig.raddr2]);
    // when: DATA from the current remote
    send_to_stream(&rig.r1, rig.addr1, FLAG_DATA, 0, b"r1").await;
    // then: processed and the hook was not asked
    assert_eq!(
        read_or_none(&mut rig.stream, Duration::from_secs(1))
            .await
            .unwrap(),
        b"r1"
    );
    assert_eq!(calls.lock().unwrap().len(), 1);
    // when: DATA from R3 (hook says yes; it also calls remote() without a deadlock)
    send_to_stream(&r3, rig.addr1, FLAG_DATA, 1, b"r3").await;
    assert_eq!(
        read_or_none(&mut rig.stream, Duration::from_secs(1))
            .await
            .unwrap(),
        b"r3"
    );
    assert_eq!(calls.lock().unwrap().last(), Some(&raddr3));
}

#[tokio::test]
async fn path_hook_is_replaced_by_a_second_call() {
    let mut rig = connected_pair_two_sockets().await;
    let control = rig.stream.control();
    let first = recording_hook(&control, |_| false);
    let second = recording_hook(&control, |_| true);
    send_to_stream(&rig.r2, rig.addr1, FLAG_DATA, 0, b"yes").await;
    assert_eq!(
        read_or_none(&mut rig.stream, Duration::from_secs(1))
            .await
            .unwrap(),
        b"yes"
    );
    assert_eq!(
        (first.lock().unwrap().len(), second.lock().unwrap().len()),
        (0, 1)
    );
}

#[tokio::test]
async fn foreign_source_is_processed_without_a_hook() {
    let mut rig = connected_pair_two_sockets().await;
    send_to_stream(&rig.r2, rig.addr1, FLAG_DATA, 0, b"as before").await;
    assert_eq!(
        read_or_none(&mut rig.stream, Duration::from_secs(1))
            .await
            .unwrap(),
        b"as before"
    );
}

#[tokio::test]
async fn closed_resolves_on_destroy_and_adapter_drop() {
    // destroy
    let rig = connected_pair_two_sockets().await;
    let mut closed = Box::pin(rig.stream.control().closed());
    assert!(matches!(poll_once(&mut closed), Poll::Pending));
    rig.stream.destroy().await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), closed)
        .await
        .expect("closed after destroy");
    // adapter drop
    let rig = connected_pair_two_sockets().await;
    let mut closed = Box::pin(rig.stream.control().closed());
    let adapter = rig.stream.into_async_stream();
    assert!(matches!(poll_once(&mut closed), Poll::Pending));
    drop(adapter);
    tokio::time::timeout(Duration::from_secs(1), closed)
        .await
        .expect("closed after the adapter drop");
}
