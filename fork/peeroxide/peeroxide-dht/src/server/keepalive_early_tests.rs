//! A server whose announcement was accepted by fewer than `MIN_ACTIVE` relays re-announces early
//! with a growing pause instead of waiting for the five-minute cycle.
//!
//! The timings are scaled: `period` stands for the 3 s ping pause, so the pauses 3, 6, 12, 24,
//! 48, 60 s of the real task are `period` times 1, 2, 4, 8, 16, 20 here. The pinging node sits
//! on a small cluster (`cluster(2)`), so an announce is accepted by at most two relays and the
//! series grows; on `cluster(8)` three relays accept and the series resets. The timestamps are
//! those of the retries. The grid is made of thresholds on the clock: the pause between two
//! ping rounds is always `period`, only the point where a step ends moves.

use std::time::Duration;

use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::time::Instant;

use std::net::UdpSocket;

use super::Keepalive;
use crate::crypto::hash;
use crate::hyperdht_messages::RelayInfo;
use crate::messages::Ipv4Peer;
use crate::server::actor::Command;
use crate::server::test_net::{Node, cluster, key, node, within};

const PERIOD: Duration = Duration::from_millis(50);

/// What the fake actor answers to `Relays`; announced lists are ignored, so the actor never
/// learns of new relays.
struct Script {
    /// The relay list answered to `Relays`; `None` = suspended.
    relays: Option<Vec<RelayInfo>>,
    /// After this many `SetRelays` the actor answers with the second list instead.
    switch: Option<(usize, Vec<RelayInfo>)>,
}

/// Instants of the `Relays` commands (with the `SetRelays` count at that time) and of `SetRelays`.
struct Trace {
    relays: Vec<(Instant, usize)>,
    sets: Vec<Instant>,
}

/// Instants of the `SetRelays` commands (one per finished re-announce) within `window`.
async fn retries(pinger: &Node, script: Script, rounds: u32, window: Duration) -> Vec<Instant> {
    trace(pinger, script, rounds, window).await.sets
}

async fn trace(pinger: &Node, script: Script, rounds: u32, window: Duration) -> Trace {
    trace_with(pinger, script, rounds, window, PERIOD).await
}

async fn trace_with(
    pinger: &Node,
    script: Script,
    rounds: u32,
    window: Duration,
    period: Duration,
) -> Trace {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let task = tokio::spawn(keepalive_with(pinger, tx, rounds, period).run());
    let mut relays = script.relays;
    let mut seen = Vec::new();
    let mut sets = Vec::new();
    let deadline = Instant::now() + window;
    while let Ok(Some(command)) = tokio::time::timeout_at(deadline, rx.recv()).await {
        match command {
            Command::Relays(reply) => {
                seen.push((Instant::now(), sets.len()));
                let _ = reply.send(relays.clone());
            }
            Command::SetRelays(list) => {
                sets.push(Instant::now());
                drop(list);
                if let Some((n, later)) = &script.switch {
                    if sets.len() >= *n {
                        relays = Some(later.clone());
                    }
                }
            }
            _ => {}
        }
    }
    task.abort();
    Trace { relays: seen, sets }
}

fn keepalive_with(
    pinger: &Node,
    cmd_tx: UnboundedSender<Command>,
    rounds: u32,
    period: Duration,
) -> Keepalive {
    let key_pair = key(1);
    Keepalive {
        dht: pinger.handle.clone(),
        cmd_tx,
        target: hash(&key_pair.public_key),
        key_pair,
        period,
        rounds,
    }
}

fn gaps(sets: &[Instant]) -> Vec<Duration> {
    sets.windows(2).map(|w| w[1] - w[0]).collect()
}

/// No relay accepted: the first re-announce comes after one ping period, not after 100 rounds.
#[tokio::test]
async fn no_relays_reannounces_after_first_period() {
    within(async {
        let cl = cluster(2).await;
        let pinger = node(&cl, false).await;
        let start = Instant::now();
        let script = Script {
            relays: Some(vec![]),
            switch: None,
        };
        let sets = retries(&pinger, script, 100, Duration::from_millis(400)).await;
        assert!(
            !sets.is_empty(),
            "no re-announce within 400 ms with 0 relays"
        );
        let first = sets[0] - start;
        assert!(
            first < Duration::from_millis(300),
            "first retry after {first:?}"
        );
    })
    .await
}

/// The pause doubles: 1, 2, 4, 8, 16 periods, then stays at 20 (3, 6, 12, 24, 48, 60, 60 s).
#[tokio::test]
async fn backoff_doubles_up_to_a_minute() {
    within(async {
        let cl = cluster(2).await;
        let pinger = node(&cl, false).await;
        let script = Script {
            relays: Some(vec![]),
            switch: None,
        };
        let sets = retries(&pinger, script, 100, Duration::from_millis(4300)).await;
        let got: Vec<u128> = gaps(&sets).iter().map(|g| g.as_millis() / 50).collect();
        assert!(got.len() >= 6, "only {} gaps: {got:?}", got.len());
        for (g, want) in got.iter().zip([2u128, 4, 8, 16, 20, 20]) {
            assert!(
                *g >= want && *g <= want + 1,
                "gap of {g} periods, want {want}: {got:?}"
            );
        }
    })
    .await
}

/// One or two relays are below `MIN_ACTIVE` too, even when they answer pings.
#[tokio::test]
async fn two_relays_also_retry_early() {
    within(async {
        let cl = cluster(2).await;
        let pinger = node(&cl, false).await;
        let live: Vec<RelayInfo> = cl
            .addrs
            .iter()
            .map(|a| RelayInfo {
                relay_address: a.clone(),
                peer_address: a.clone(),
            })
            .collect();
        let script = Script {
            relays: Some(live),
            switch: None,
        };
        let sets = retries(&pinger, script, 100, Duration::from_millis(1500)).await;
        assert!(
            !sets.is_empty(),
            "2 answering relays must still retry early"
        );
    })
    .await
}

/// Three relays: the old cycle, no announce before the cycle ends (here: well past 5 periods).
#[tokio::test]
async fn three_relays_keep_the_five_minute_cycle() {
    within(async {
        let cl = cluster(3).await;
        let pinger = node(&cl, false).await;
        let live: Vec<RelayInfo> = cl
            .addrs
            .iter()
            .map(|a| RelayInfo {
                relay_address: a.clone(),
                peer_address: a.clone(),
            })
            .collect();
        let script = Script {
            relays: Some(live),
            switch: None,
        };
        let sets = retries(&pinger, script, 1000, Duration::from_millis(1500)).await;
        assert!(
            sets.is_empty(),
            "3 live relays re-announced: {} times",
            sets.len()
        );
    })
    .await
}

/// Three relays accept each announce, though the actor still reports none: the pause stays one period.
#[tokio::test]
async fn backoff_resets_once_three_relays_accept() {
    within(async {
        let cl = cluster(8).await;
        let pinger = node(&cl, false).await;
        let script = Script {
            relays: Some(vec![]),
            switch: None,
        };
        let sets = retries(&pinger, script, 100, Duration::from_millis(1500)).await;
        let got: Vec<u128> = gaps(&sets).iter().map(|g| g.as_millis() / 50).collect();
        assert!(got.len() >= 4, "gaps: {got:?}");
        assert!(
            got.iter().all(|g| *g <= 3),
            "the pause grew despite 3 accepted relays: {got:?}"
        );
    })
    .await
}

/// A suspended actor (`None`) gets no retry however few relays there are.
#[tokio::test]
async fn suspended_server_does_not_retry() {
    within(async {
        let cl = cluster(2).await;
        let pinger = node(&cl, false).await;
        let script = Script {
            relays: None,
            switch: None,
        };
        let sets = retries(&pinger, script, 100, Duration::from_millis(1000)).await;
        assert!(
            sets.is_empty(),
            "a suspended server re-announced {} times",
            sets.len()
        );
    })
    .await
}

/// After a long series, three relays set elsewhere end the backoff: the pings go on every
/// period, and the next re-announce (after `Expired`) restarts the series at one period.
#[tokio::test]
async fn backoff_gives_way_to_three_relays_set_elsewhere() {
    within(async {
        let cl = cluster(2).await;
        let other = cluster(3).await;
        let pinger = node(&cl, false).await;
        let live: Vec<RelayInfo> = other
            .addrs
            .iter()
            .map(|a| RelayInfo {
                relay_address: a.clone(),
                peer_address: a.clone(),
            })
            .collect();
        let script = Script {
            relays: Some(vec![]),
            switch: Some((5, live)),
        };
        let t = trace(&pinger, script, 3, Duration::from_millis(4300)).await;
        let late: Vec<Instant> = t.relays.iter().filter(|r| r.1 >= 5).map(|r| r.0).collect();
        assert!(
            late.len() >= 5,
            "only {} relay queries after the switch",
            late.len()
        );
        let got: Vec<u128> = gaps(&late).iter().map(|g| g.as_millis() / 50).collect();
        assert!(
            got.iter().all(|g| *g <= 2),
            "pause stayed long after three relays: {got:?}"
        );
    })
    .await
}

/// Between two re-announces of a server with two answering relays the relays are pinged every
/// period (one `Relays` question per round), not once per step.
#[tokio::test]
async fn one_or_two_relays_are_pinged_between_retries() {
    within(async {
        let cl = cluster(2).await;
        let pinger = node(&cl, false).await;
        let live: Vec<RelayInfo> = cl.addrs.iter().map(relay).collect();
        let script = Script {
            relays: Some(live),
            switch: None,
        };
        let t = trace(&pinger, script, 100, Duration::from_millis(1000)).await;
        let mut checked = 0;
        for w in t.sets.windows(2) {
            let steps = ((w[1] - w[0]).as_millis() / 50) as usize;
            if steps < 8 {
                continue;
            }
            let rounds = t.relays.iter().filter(|r| r.0 > w[0] && r.0 <= w[1]).count();
            assert!(
                rounds + 2 >= steps,
                "{rounds} relay questions in a step of {steps} periods"
            );
            checked += 1;
        }
        assert!(checked >= 1, "no step of 8 periods or more in 1 s");
    })
    .await
}

/// One of two relays is lost: the re-announce follows the failed ping round, it does not wait
/// for the end of the retry step.
#[tokio::test]
async fn lost_relay_of_two_reannounces_at_once() {
    within(async {
        let period = Duration::from_millis(800);
        let cl = cluster(2).await;
        let pinger = node(&cl, false).await;
        let socket = UdpSocket::bind("127.0.0.1:0").expect("bind the silent socket");
        let port = socket.local_addr().expect("local_addr").port();
        let silent = Ipv4Peer {
            host: "127.0.0.1".into(),
            port,
        };
        let one_dead = vec![relay(&cl.addrs[0]), relay(&silent)];
        // After the third retry (at 1, 3, 7 periods) the next step is 8 periods.
        let script = Script {
            relays: Some(vec![]),
            switch: Some((3, one_dead)),
        };
        let t = trace_with(&pinger, script, 100, period * 17, period).await;
        assert!(t.sets.len() >= 4, "only {} re-announces", t.sets.len());
        let gap = t.sets[3] - t.sets[2];
        assert!(
            gap < period * 7,
            "re-announce {gap:?} after the third, the step is 8 periods"
        );
        drop(socket);
    })
    .await
}

/// After an `Expired` cycle the series restarts: with 3 rounds the 4-period step always ends by
/// expiry, and the gap that follows must be short again (a leftover `retries` would keep it at 3).
#[tokio::test]
async fn expired_cycle_restarts_the_series() {
    within(async {
        let cl = cluster(2).await;
        let pinger = node(&cl, false).await;
        let script = Script {
            relays: Some(vec![]),
            switch: None,
        };
        let sets = retries(&pinger, script, 3, Duration::from_millis(1000)).await;
        let got: Vec<u128> = gaps(&sets).iter().map(|g| g.as_millis() / 50).collect();
        assert!(got.len() >= 3, "only {} gaps: {got:?}", got.len());
        assert!(
            got[2] <= 2,
            "after the expired cycle the series did not restart: {got:?}"
        );
    })
    .await
}

fn relay(a: &Ipv4Peer) -> RelayInfo {
    RelayInfo {
        relay_address: a.clone(),
        peer_address: a.clone(),
    }
}
