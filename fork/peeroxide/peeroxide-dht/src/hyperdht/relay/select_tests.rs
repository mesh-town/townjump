//! Tests for relay choice, the role in a pair and the secrecy of the token.
// JS: hyperdht/lib/connect.js:53, 498-536

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

fn key(n: u8) -> [u8; 32] {
    [n; 32]
}

fn info(pk: u8, tok: u8) -> RelayThroughInfo {
    RelayThroughInfo {
        version: 1,
        public_key: key(pk),
        token: key(tok),
    }
}

fn ask(pk: u8, tok: u8) -> RelayAsk {
    RelayAsk {
        public_key: key(pk),
        token: key(tok),
    }
}

#[test]
fn select_key() {
    assert_eq!(RelayThrough::Key(key(1)).select(), Some(key(1)));
}

#[test]
fn select_list_spreads() {
    // given: three relays
    let rt = RelayThrough::List(vec![key(1), key(2), key(3)]);
    let mut counts = [0usize; 3];
    // when: selecting 300 times
    for _ in 0..300 {
        let k = rt.select().expect("a key");
        counts[usize::from(k[0]) - 1] += 1;
    }
    // then: every relay is picked often
    assert!(counts.iter().all(|&c| c >= 50), "counts {counts:?}");
}

#[test]
fn select_empty_list_none() {
    assert_eq!(RelayThrough::List(vec![]).select(), None);
}

#[test]
fn select_fn_none_and_called_each_time() {
    // given: a function that answers None
    let calls = Arc::new(AtomicUsize::new(0));
    let c = Arc::clone(&calls);
    let rt = RelayThrough::Select(Arc::new(move || {
        c.fetch_add(1, Ordering::SeqCst);
        None
    }));
    // when: asked twice
    assert_eq!(rt.select(), None);
    assert_eq!(rt.select(), None);
    // then: it ran twice
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn resolve_none_without_relay() {
    assert!(resolve(None).is_none());
}

#[test]
fn resolve_fresh_token_each_time() {
    // given: one fixed relay
    let rt = RelayThrough::Key(key(9));
    // when: resolved twice
    let (a, b) = (resolve(Some(&rt)).unwrap(), resolve(Some(&rt)).unwrap());
    // then: same relay, different tokens
    assert_eq!(a.public_key, key(9));
    assert_eq!(b.public_key, key(9));
    assert_ne!(a.token, b.token);
}

#[test]
fn client_role_server_wins() {
    let role = client_role(Some(&info(1, 2)), None).unwrap();
    assert_eq!((role.public_key, role.token), (key(1), key(2)));
    assert!(!role.is_initiator);
}

#[test]
fn client_role_ours_initiator() {
    let role = client_role(None, Some(&ask(3, 4))).unwrap();
    assert_eq!((role.public_key, role.token), (key(3), key(4)));
    assert!(role.is_initiator);
}

#[test]
fn client_role_both_server_non_initiator() {
    let role = client_role(Some(&info(1, 2)), Some(&ask(3, 4))).unwrap();
    assert_eq!((role.public_key, role.token), (key(1), key(2)));
    assert!(!role.is_initiator);
}

#[test]
fn client_role_none() {
    assert!(client_role(None, None).is_none());
}

#[test]
fn debug_hides_token() {
    let text = format!("{:?}", ask(3, 0xAB));
    assert!(!text.contains("171"), "token leaked: {text}");
    assert!(!text.to_lowercase().contains("ab"), "token leaked: {text}");
}

mod gate {
    use std::sync::Mutex;

    use libudx::UdxRuntime;
    use tokio::sync::mpsc;

    use super::super::select::gate_punch;
    use super::*;
    use crate::holepuncher::Holepuncher;
    use crate::hyperdht::types::HyperDhtError;
    use crate::messages::Ipv4Peer;
    use crate::socket_pool::SocketPool;

    type Seen = Arc<Mutex<Option<(u64, Vec<Ipv4Peer>)>>>;

    async fn puncher_with_remote(rt: &UdxRuntime) -> Holepuncher {
        let pool = SocketPool::new("0.0.0.0".into());
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut p = Holepuncher::new(&pool, rt, true, true, 2, tx)
            .await
            .expect("puncher");
        let addr = Ipv4Peer {
            host: "10.0.0.7".into(),
            port: 4242,
        };
        p.update_remote(false, 3, &[addr], None);
        p
    }

    #[tokio::test]
    async fn filter_rejecting_aborts_with_holepunch_aborted() {
        let rt = UdxRuntime::new().unwrap();
        let mut p = puncher_with_remote(&rt).await;
        let seen: Seen = Arc::default();
        let sink = Arc::clone(&seen);
        let filter = HolepunchFilter::new(move |i| {
            *sink.lock().unwrap() = Some((i.remote_firewall, i.remote_addresses.to_vec()));
            false
        });
        let r = gate_punch(Some(&filter), &mut p);
        assert!(matches!(r, Err(HyperDhtError::HolepunchAborted)));
        assert!(p.destroyed, "puncher must be destroyed on refusal");
        let (fw, addrs) = seen.lock().unwrap().take().expect("filter called");
        assert_eq!(fw, 3);
        assert_eq!(addrs[0].port, 4242);
    }

    #[tokio::test]
    async fn filter_none_behaves_as_before() {
        let rt = UdxRuntime::new().unwrap();
        let mut p = puncher_with_remote(&rt).await;
        assert!(gate_punch(None, &mut p).is_ok());
        let allow = HolepunchFilter::new(|_| true);
        assert!(gate_punch(Some(&allow), &mut p).is_ok());
        assert!(!p.destroyed);
    }
}

/// `candidates(start)`: the keys a connect tries, in order (the `List` retry order).
mod candidates {
    use super::*;

    fn list(keys: &[u8]) -> RelayThrough {
        RelayThrough::List(keys.iter().map(|&n| key(n)).collect())
    }

    fn keys(ns: &[u8]) -> Vec<[u8; 32]> {
        ns.iter().map(|&n| key(n)).collect()
    }

    mod list {
        use super::*;

        #[test]
        fn rotates_from_start() {
            // given: three relays
            let rt = list(&[1, 2, 3]);
            // when: starting at the second one
            let from_one = rt.candidates(1);
            // then: the order wraps around
            assert_eq!(from_one, keys(&[2, 3, 1]));
        }

        #[test]
        fn start_past_the_end_wraps_by_length() {
            // given: three relays
            let rt = list(&[1, 2, 3]);
            // when: start is 4 (= 1 mod 3)
            let wrapped = rt.candidates(4);
            // then: same as start 1
            assert_eq!(wrapped, keys(&[2, 3, 1]));
        }

        #[test]
        fn start_zero_keeps_the_given_order() {
            assert_eq!(list(&[1, 2, 3]).candidates(0), keys(&[1, 2, 3]));
        }

        #[test]
        fn drops_repeated_keys_keeping_the_first_after_rotation() {
            // given: key 1 listed twice
            let rt = list(&[1, 2, 1]);
            // when: rotating to start at the last entry (order 1, 1, 2)
            let got = rt.candidates(2);
            // then: every key appears once
            assert_eq!(got, keys(&[1, 2]));
        }

        #[test]
        fn empty_list_gives_no_candidates_for_any_start() {
            // given: no relays; when: any start, including one that would divide by zero
            let rt = RelayThrough::List(vec![]);
            // then: nothing, and no panic
            assert!(rt.candidates(0).is_empty());
            assert!(rt.candidates(5).is_empty());
        }
    }

    #[test]
    fn key_is_the_single_candidate_whatever_the_start() {
        assert_eq!(RelayThrough::Key(key(5)).candidates(7), keys(&[5]));
    }

    mod select {
        use super::*;

        fn counting(answer: Option<[u8; 32]>) -> (RelayThrough, Arc<AtomicUsize>) {
            let calls = Arc::new(AtomicUsize::new(0));
            let c = Arc::clone(&calls);
            let rt = RelayThrough::Select(Arc::new(move || {
                c.fetch_add(1, Ordering::SeqCst);
                answer
            }));
            (rt, calls)
        }

        #[test]
        fn some_key_is_the_single_candidate_and_the_function_runs_once() {
            // given: a function answering key 4
            let (rt, calls) = counting(Some(key(4)));
            // when: asking for candidates
            let got = rt.candidates(3);
            // then: that key alone, one call
            assert_eq!(got, keys(&[4]));
            assert_eq!(
                calls.load(Ordering::SeqCst),
                1,
                "Select must run exactly once"
            );
        }

        #[test]
        fn none_gives_no_candidates_after_one_call() {
            // given: a function answering None
            let (rt, calls) = counting(None);
            // when: asking for candidates
            let got = rt.candidates(0);
            // then: nothing, one call
            assert!(got.is_empty());
            assert_eq!(
                calls.load(Ordering::SeqCst),
                1,
                "Select must run exactly once"
            );
        }
    }
}
