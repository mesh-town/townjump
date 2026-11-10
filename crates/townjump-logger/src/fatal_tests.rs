use super::*;
use tokio::sync::mpsc::error::TryRecvError;

#[test]
fn texts_arrive_verbatim_and_in_order() {
    let (f, mut rx) = Fatal::channel();
    for t in ["", "a\nb", "κλ", "\x1b[31mx"] {
        f.raise(t);
    }
    f.raise(String::from("owned"));
    for t in ["", "a\nb", "κλ", "\x1b[31mx", "owned"] {
        assert_eq!(rx.try_recv().unwrap(), t);
    }
    assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));
}

#[test]
fn clones_from_threads_lose_nothing() {
    let (f, mut rx) = Fatal::channel();
    let hs: Vec<_> = (0..4)
        .map(|i| {
            let f = f.clone();
            std::thread::spawn(move || (0..50).for_each(|j| f.raise(format!("{i}-{j}"))))
        })
        .collect();
    hs.into_iter().for_each(|h| h.join().unwrap());
    let mut n = 0;
    while rx.try_recv().is_ok() {
        n += 1;
    }
    assert_eq!(n, 200);
}

#[test]
fn raise_after_receiver_dropped_is_silent() {
    let (f, rx) = Fatal::channel();
    drop(rx);
    f.raise("gone");
}

#[test]
fn closed_only_when_all_senders_dropped() {
    let (f, mut rx) = Fatal::channel();
    let g = f.clone();
    f.raise("last");
    drop(f);
    assert_eq!(rx.try_recv().unwrap(), "last");
    assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));
    drop(g);
    assert_eq!(rx.try_recv(), Err(TryRecvError::Disconnected));
}

#[test]
fn sender_is_send_sync_static() {
    fn ok<T: Send + Sync + 'static>() {}
    ok::<Fatal>();
}
