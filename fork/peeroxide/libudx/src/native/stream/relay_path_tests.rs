//! Relay fast path: no lock is held while waiting for the target lock; unconnected targets drop with a result.

use super::super::test_support::stream_inner;
use super::*;

fn pair() -> (Arc<Mutex<StreamInner>>, Arc<Mutex<StreamInner>>) {
    let r1 = Arc::new(Mutex::new(stream_inner()));
    let r2 = Arc::new(Mutex::new(stream_inner()));
    r2.lock().unwrap().remote_id = 77;
    r1.lock().unwrap().relay_target = Some(Arc::downgrade(&r2));
    r2.lock().unwrap().relay_target = Some(Arc::downgrade(&r1));
    (r1, r2)
}

#[test]
fn own_lock_is_free_while_waiting_for_target_lock() {
    let (r1, r2) = pair();
    let held = r2.lock().unwrap();
    let r1c = r1.clone();
    let worker = std::thread::spawn(move || pick_dest(&r1c).map(|d| d.remote_id));
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(
        !worker.is_finished(),
        "worker must wait for the target lock"
    );
    assert!(
        r1.try_lock().is_ok(),
        "own lock must not be held while the target lock is awaited"
    );
    drop(held);
    assert_eq!(worker.join().unwrap(), Some(77));
}

#[tokio::test]
async fn drops_with_debug_when_target_is_not_connected() {
    let (r1, _r2) = pair();
    let dest = pick_dest(&r1).expect("relay target configured");
    assert_eq!(
        (dest.remote_id, dest.addr.is_none(), dest.udp.is_none()),
        (77, true, true)
    );
    let mut pkt = vec![0u8; 20];
    assert!(
        !forward(&dest, &mut pkt).await,
        "packet to an unconnected target is dropped"
    );
    assert_eq!(
        pkt[4..8],
        77u32.to_le_bytes(),
        "destination id is rewritten even on drop"
    );
    assert!(r1.try_lock().is_ok());
}

#[test]
fn no_relay_target_gives_no_dest() {
    let r = Mutex::new(stream_inner());
    assert!(pick_dest(&r).is_none());
}
