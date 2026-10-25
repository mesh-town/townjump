//! Tests for [`super`]: LAN candidate selection, the per-candidate deadline and the LAN-versus-holepunch race.

use std::collections::HashMap;
use std::future::pending;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::time::{Instant, sleep};

use super::{LAN_PROBE_DEADLINE, LanProbe, LanRewrite, first_reachable, lan_candidates, lan_for};
use crate::hyperdht::{ConnectOpts, is_addr_private};
use crate::messages::Ipv4Peer;

#[path = "connect_lan_race_tests.rs"]
mod race;

mod lan_candidates_fn {
    use super::*;

    #[test]
    fn subnet_match_comes_first() {
        let advertised = peers(&["8.8.8.8", "10.9.9.9", "192.168.1.20"]);
        // when
        let got = lan_candidates(&advertised, &[net("192.168.1.10", 24)]);
        // then
        assert_eq!(got, peers(&["192.168.1.20", "10.9.9.9"]));
    }

    #[test]
    fn private_addresses_keep_advertised_order() {
        let advertised = peers(&["10.9.9.9", "172.16.0.2"]);
        let got = lan_candidates(&advertised, &[]);
        assert_eq!(got, advertised);
    }

    #[test]
    fn public_addresses_are_never_candidates() {
        let advertised = peers(&["8.8.8.8", "8.8.4.4"]);
        // when: even a subnet shared with a public address does not make it a candidate
        let got = lan_candidates(&advertised, &[net("8.8.8.9", 24)]);
        // then
        assert!(got.is_empty(), "public addresses leaked: {got:?}");
    }

    #[test]
    fn subnet_match_is_not_repeated() {
        let advertised = peers(&["10.9.9.9", "192.168.1.20", "172.16.0.2"]);
        // when
        let got = lan_candidates(&advertised, &[net("192.168.1.10", 24)]);
        // then
        let hits = got.iter().filter(|p| p.host == "192.168.1.20").count();
        assert_eq!(hits, 1, "subnet match listed {hits} times in {got:?}");
        assert_eq!(got.len(), 3, "every private address once: {got:?}");
    }

    #[test]
    fn repeated_advertised_address_appears_once() {
        let got = lan_candidates(&peers(&["10.9.9.9", "10.9.9.9"]), &[]);
        assert_eq!(got, peers(&["10.9.9.9"]));
    }

    #[test]
    fn empty_advertisement_gives_nothing() {
        let got = lan_candidates(&[], &[net("192.168.1.10", 24)]);
        assert!(got.is_empty());
    }
}

mod lan_for_fn {
    use super::*;

    #[test]
    fn other_host_gets_no_candidates() {
        let called = Arc::new(AtomicBool::new(false));
        let opts = rewrite_to(vec![peer("127.0.0.1", 9)], Some(Arc::clone(&called)));
        // when
        let got = lan_for(false, &peers(&["10.9.9.9"]), &opts);
        // then
        assert!(got.is_empty(), "other host got candidates: {got:?}");
        assert!(
            !called.load(Ordering::SeqCst),
            "the rewrite ran for another host"
        );
    }

    #[test]
    fn same_host_takes_the_rewrite() {
        let opts = rewrite_to(vec![peer("127.0.0.1", 9)], None);
        let got = lan_for(true, &peers(&["10.9.9.9"]), &opts);
        assert_eq!(got, vec![peer("127.0.0.1", 9)]);
    }

    #[test]
    fn same_host_never_offers_a_public_address() {
        let advertised = peers(&["8.8.8.8", "10.9.9.9", "8.8.4.4"]);
        // when
        let got = lan_for(true, &advertised, &ConnectOpts::default());
        // then
        assert!(
            got.contains(&peer("10.9.9.9", 7)),
            "private address missing: {got:?}"
        );
        assert!(
            got.iter().all(|p| is_addr_private(&p.host)),
            "public address offered: {got:?}"
        );
    }
}

mod first_reachable_fn {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn answer_at_1_9_s_is_taken() {
        let steps = [(1, Step::AnswerAfter(ms(1900))), (2, Step::Now)];
        // when
        let out = probe_through(&steps, &[1, 2]).await;
        // then
        assert_eq!(out.hit, Some(1));
        assert_eq!(out.called(), vec![1], "the second candidate was asked");
        assert_near(out.elapsed, ms(1900));
    }

    #[tokio::test(start_paused = true)]
    async fn answer_at_2_1_s_moves_on() {
        let steps = [(1, Step::AnswerAfter(ms(2100))), (2, Step::Now)];
        // when
        let out = probe_through(&steps, &[1, 2]).await;
        // then
        assert_eq!(out.hit, Some(2));
        assert_eq!(out.called(), vec![1, 2]);
        assert_near(out.calls[1].1, LAN_PROBE_DEADLINE);
        assert_near(out.elapsed, LAN_PROBE_DEADLINE);
    }

    #[tokio::test(start_paused = true)]
    async fn refused_candidate_moves_on_at_once() {
        let steps = [(1, Step::Refused), (2, Step::Now)];
        // when
        let out = probe_through(&steps, &[1, 2]).await;
        // then
        assert_eq!(out.hit, Some(2));
        assert_near(out.calls[1].1, Duration::ZERO);
        assert_near(out.elapsed, Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn all_silent_give_none_after_a_deadline_each() {
        let steps = [(1, Step::Silent), (2, Step::Silent), (3, Step::Silent)];
        // when
        let out = probe_through(&steps, &[1, 2, 3]).await;
        // then
        assert_eq!(out.hit, None);
        assert_near(out.elapsed, LAN_PROBE_DEADLINE * 3);
    }

    #[tokio::test(start_paused = true)]
    async fn candidates_are_tried_in_advertised_order() {
        // ports chosen so that sorted order differs from advertised order
        let steps = [
            (30, Step::Refused),
            (10, Step::Refused),
            (20, Step::Refused),
        ];
        let out = probe_through(&steps, &[30, 10, 20]).await;
        assert_eq!(out.called(), vec![30, 10, 20]);
    }

    #[tokio::test(start_paused = true)]
    async fn no_candidates_give_none_at_once() {
        let out = probe_through(&[(1, Step::Now)], &[]).await;
        assert_eq!(out.hit, None);
        assert!(out.calls.is_empty(), "probe called: {:?}", out.calls);
        assert_near(out.elapsed, Duration::ZERO);
    }
}

// ---- setup and helpers ----

fn peer(host: &str, port: u16) -> Ipv4Peer {
    Ipv4Peer {
        host: host.to_string(),
        port,
    }
}

/// Advertised addresses, all on port 7.
fn peers(hosts: &[&str]) -> Vec<Ipv4Peer> {
    hosts.iter().map(|h| peer(h, 7)).collect()
}

fn net(ip: &str, prefix: u8) -> (Ipv4Addr, u8) {
    (ip.parse().expect("ipv4"), prefix)
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// Options whose candidate rewrite returns `to`; raises `called` when it runs.
fn rewrite_to(to: Vec<Ipv4Peer>, called: Option<Arc<AtomicBool>>) -> ConnectOpts {
    let rewrite = move |_: &[Ipv4Peer]| {
        if let Some(flag) = &called {
            flag.store(true, Ordering::SeqCst);
        }
        to.clone()
    };
    ConnectOpts {
        lan_rewrite: Some(LanRewrite(Arc::new(rewrite))),
        ..ConnectOpts::default()
    }
}

/// Virtual time is exact, but a timer wheel rounds to the millisecond: allow a small slack.
fn assert_near(actual: Duration, expected: Duration) {
    let slack = ms(20);
    assert!(
        actual + slack >= expected && actual <= expected + slack,
        "elapsed {actual:?}, expected about {expected:?}"
    );
}

/// What a fake candidate does when asked.
#[derive(Clone, Copy)]
enum Step {
    AnswerAfter(Duration),
    Now,
    /// Gives up immediately (a send error).
    Refused,
    Silent,
}

/// A scripted [`LanProbe`] keyed by candidate port; remembers who was asked and when.
struct FakeProbe {
    steps: HashMap<u16, Step>,
    calls: Vec<(u16, Duration)>,
    start: Instant,
}

impl LanProbe for FakeProbe {
    type Hit = u16;

    async fn reach(&mut self, peer: &Ipv4Peer) -> Option<u16> {
        self.calls.push((peer.port, self.start.elapsed()));
        match self.steps[&peer.port] {
            Step::Now => Some(peer.port),
            Step::Refused => None,
            Step::Silent => pending().await,
            Step::AnswerAfter(d) => {
                sleep(d).await;
                Some(peer.port)
            }
        }
    }
}

struct Outcome {
    hit: Option<u16>,
    calls: Vec<(u16, Duration)>,
    elapsed: Duration,
}

impl Outcome {
    fn called(&self) -> Vec<u16> {
        self.calls.iter().map(|c| c.0).collect()
    }
}

/// Runs `first_reachable` over the candidates `order` (ports) against a probe scripted by `steps`.
async fn probe_through(steps: &[(u16, Step)], order: &[u16]) -> Outcome {
    let start = Instant::now();
    let mut probe = FakeProbe {
        steps: steps.iter().copied().collect(),
        calls: Vec::new(),
        start,
    };
    let candidates: Vec<Ipv4Peer> = order.iter().map(|&p| peer("127.0.0.1", p)).collect();
    let hit = first_reachable(&candidates, &mut probe).await;
    Outcome {
        hit,
        calls: probe.calls,
        elapsed: start.elapsed(),
    }
}
