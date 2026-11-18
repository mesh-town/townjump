//! A server whose announcement was accepted by fewer than `MIN_ACTIVE` relays re-announces early
//! with a growing pause instead of waiting for the five-minute cycle.
//!
//! The timings are scaled: `period` stands for the 3 s ping pause, so the pauses 3, 6, 12, 24,
//! 48, 60 s of the real task are `period` times 1, 2, 4, 8, 16, 20 here. The node has no peers,
//! so an announce ends at once with no relays and the timestamps are those of the retries.

use std::time::Duration;

use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::time::Instant;

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
}

/// Instants of the `SetRelays` commands (one per finished re-announce) within `window`.
async fn retries(pinger: &Node, script: Script, rounds: u32, window: Duration) -> Vec<Instant> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let task = tokio::spawn(keepalive(pinger, tx, rounds).run());
    let relays = script.relays;
    let mut sets = Vec::new();
    let deadline = Instant::now() + window;
    while let Ok(Some(command)) = tokio::time::timeout_at(deadline, rx.recv()).await {
        match command {
            Command::Relays(reply) => {
                let _ = reply.send(relays.clone());
            }
            Command::SetRelays(list) => {
                sets.push(Instant::now());
                drop(list);
            }
            _ => {}
        }
    }
    task.abort();
    sets
}

fn keepalive(pinger: &Node, cmd_tx: UnboundedSender<Command>, rounds: u32) -> Keepalive {
    let key_pair = key(1);
    Keepalive {
        dht: pinger.handle.clone(),
        cmd_tx,
        target: hash(&key_pair.public_key),
        key_pair,
        period: PERIOD,
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
        let script = Script { relays: Some(live) };
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
        let script = Script { relays: Some(live) };
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
        let script = Script { relays: None };
        let sets = retries(&pinger, script, 100, Duration::from_millis(1000)).await;
        assert!(
            sets.is_empty(),
            "a suspended server re-announced {} times",
            sets.len()
        );
    })
    .await
}
