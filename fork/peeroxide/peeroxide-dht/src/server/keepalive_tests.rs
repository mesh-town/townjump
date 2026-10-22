//! Keep-alive of a server's relays (`keepalive`): the re-announce decision, the pings against a
//! local cluster and the whole `Keepalive::run` loop against a fake actor.
//!
//! No `start_paused`: the cluster has real UDP sockets, and fast-forwarded time would burn the
//! 1 s request timeouts before the replies arrive. Short real periods (100-200 ms) go in through
//! the fields of `Keepalive`.

use std::net::UdpSocket;
use std::time::{Duration, Instant};

use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use super::{Keepalive, needs_reannounce, ping_all};
use crate::crypto::hash;
use crate::hyperdht_messages::RelayInfo;
use crate::messages::Ipv4Peer;
use crate::server::actor::Command;
use crate::server::test_net::{Cluster, Node, cluster, key, node, within};

mod decision {
    use super::*;

    /// Without relays there is nothing to lose: no re-announce, as in JS.
    #[test]
    fn no_relays_never_reannounce() {
        assert!(
            !needs_reannounce(0, 0),
            "0 of 0 relays must not re-announce"
        );
    }

    /// All three relays answer: the announcement stays.
    #[test]
    fn all_three_answer_keeps_relays() {
        assert!(!needs_reannounce(3, 3), "3 of 3 must keep the relays");
    }

    /// One of three relays is lost: that is already too few, re-announce.
    #[test]
    fn one_of_three_lost_reannounces() {
        assert!(needs_reannounce(2, 3), "2 of 3 must re-announce");
    }

    /// With a single relay the bar is that one relay: `min(1, 3) = 1`.
    #[test]
    fn single_relay() {
        assert!(!needs_reannounce(1, 1), "1 of 1 must keep the relay");
        assert!(needs_reannounce(0, 1), "0 of 1 must re-announce");
    }

    /// The bar is capped at `MIN_ACTIVE = 3` however many relays there are.
    #[test]
    fn more_relays_than_min_active() {
        assert!(!needs_reannounce(3, 5), "3 of 5 is enough");
        assert!(needs_reannounce(2, 5), "2 of 5 is too few");
    }
}

mod pings {
    use super::*;

    /// Two cluster nodes answer, a silent socket does not: the count is 2.
    #[tokio::test]
    async fn counts_only_answering_relays() {
        within(async {
            // given
            let cl = cluster(3).await;
            let pinger = node(&cl, false).await;
            let (_silent_socket, silent) = mute();
            let relays = [relay(&cl.addrs[0]), relay(&cl.addrs[1]), silent];
            // when
            let alive = ping_all(pinger.handle.dht(), &relays).await;
            // then
            assert_eq!(alive, 2, "2 answering relays out of 3, one is silent");
        })
        .await
    }

    /// No relays, no pings, nothing answers.
    #[tokio::test]
    async fn empty_list_is_zero() {
        within(async {
            // given
            let cl = cluster(2).await;
            let pinger = node(&cl, false).await;
            // when
            let alive = ping_all(pinger.handle.dht(), &[]).await;
            // then
            assert_eq!(alive, 0, "an empty relay list has no live relays");
        })
        .await
    }
}

mod rounds {
    use super::*;

    /// Healthy relays are asked for and pinged round after round, once per period, never re-announced.
    #[tokio::test]
    async fn healthy_relays_are_pinged_every_period() {
        within(async {
            // given
            let cl = cluster(3).await;
            let pinger = node(&cl, false).await;
            let stop = Stop::after(Duration::from_secs(2));
            // when
            let seen = watch(&pinger, Some(live(&cl)), ms(200), 1000, stop).await;
            // then
            assert!(
                seen.asks.len() >= 5,
                "only {} rounds in 2 s",
                seen.asks.len()
            );
            for gap in seen.gaps() {
                assert!(
                    (ms(160)..=Duration::from_secs(1)).contains(&gap),
                    "a round came {gap:?} after the previous one, the period is 200 ms"
                );
            }
            assert!(
                seen.sets.is_empty(),
                "healthy relays must not be re-announced"
            );
        })
        .await
    }

    /// One silent relay of three: one early re-announce with fresh relays, then healthy rounds again.
    #[tokio::test]
    async fn mute_relay_gives_one_early_reannounce() {
        within(async {
            // given
            let cl = cluster(3).await;
            let pinger = node(&cl, false).await;
            let (_silent_socket, silent) = mute();
            let silent_port = silent.relay_address.port;
            let relays = vec![relay(&cl.addrs[0]), relay(&cl.addrs[1]), silent];
            let stop = Stop::after_first_set(Duration::from_secs(15), ms(1500));
            // when
            let seen = watch(&pinger, Some(relays), ms(100), 1000, stop).await;
            // then
            assert_eq!(seen.sets.len(), 1, "exactly one early re-announce expected");
            let fresh = &seen.sets[0].relays;
            assert!(
                (1..=3).contains(&fresh.len()),
                "{} relays announced",
                fresh.len()
            );
            assert!(
                fresh.iter().all(|r| r.relay_address.port != silent_port),
                "the silent relay must not come back: {fresh:?}"
            );
            let after = seen.asks.len() - seen.sets[0].asks_before;
            assert!(after >= 3, "only {after} rounds after the re-announce");
        })
        .await
    }

    /// `rounds` healthy rounds end the cycle: a full re-announce after the last round's check.
    #[tokio::test]
    async fn exhausted_rounds_reannounce_fully() {
        within(async {
            // given
            let cl = cluster(3).await;
            let pinger = node(&cl, false).await;
            let stop = Stop::after_first_set(Duration::from_secs(10), Duration::ZERO);
            // when
            let seen = watch(&pinger, Some(live(&cl)), ms(100), 3, stop).await;
            // then
            assert!(!seen.sets.is_empty(), "no full re-announce in 10 s");
            let before = seen.sets[0].asks_before;
            assert_eq!(
                before, 4,
                "3 rounds and the check before re-announcing, got {before}"
            );
        })
        .await
    }

    /// A suspended actor answers `None`: rounds go on being asked, nothing is pinged or re-announced.
    #[tokio::test]
    async fn suspended_actor_gets_no_reannounce() {
        within(async {
            // given
            let cl = cluster(3).await;
            let pinger = node(&cl, false).await;
            let stop = Stop::after(ms(1500));
            // when
            let seen = watch(&pinger, None, ms(100), 2, stop).await;
            // then
            assert!(
                seen.asks.len() >= 5,
                "only {} asks in 1.5 s",
                seen.asks.len()
            );
            assert!(
                seen.sets.is_empty(),
                "a suspended server must not re-announce"
            );
        })
        .await
    }

    /// The actor's command channel is closed: `run` ends instead of looping.
    #[tokio::test]
    async fn actor_gone_ends_run() {
        within(async {
            // given
            let cl = cluster(2).await;
            let pinger = node(&cl, false).await;
            let (tx, rx) = mpsc::unbounded_channel();
            drop(rx);
            // when
            let run = keepalive(&pinger, tx, ms(100), 3).run();
            let ended = tokio::time::timeout(Duration::from_secs(2), run).await;
            // then
            assert!(
                ended.is_ok(),
                "run must end within 2 s once the actor is gone"
            );
        })
        .await
    }

    /// The actor drops the question unanswered (it died mid-way): `run` ends too.
    #[tokio::test]
    async fn dropped_question_ends_run() {
        within(async {
            // given
            let cl = cluster(2).await;
            let pinger = node(&cl, false).await;
            let (tx, mut rx) = mpsc::unbounded_channel();
            let task = tokio::spawn(keepalive(&pinger, tx, ms(100), 3).run());
            // when
            drop(rx.recv().await.expect("the task asks for the relays"));
            // then
            let ended = tokio::time::timeout(Duration::from_secs(2), task).await;
            assert!(
                ended.is_ok(),
                "run must end within 2 s when its question is dropped"
            );
        })
        .await
    }
}

/// What the fake actor saw from the keep-alive task.
#[derive(Default)]
struct Seen {
    /// When the task asked for the relay list (`Command::Relays`).
    asks: Vec<Instant>,
    /// The re-announces handed over (`Command::SetRelays`), in order.
    sets: Vec<Set>,
}

/// One `Command::SetRelays`.
struct Set {
    /// How many `Relays` questions came before it.
    asks_before: usize,
    relays: Vec<RelayInfo>,
}

impl Seen {
    /// Pauses between consecutive questions.
    fn gaps(&self) -> Vec<Duration> {
        self.asks.windows(2).map(|w| w[1] - w[0]).collect()
    }
}

/// When the fake actor stops listening.
struct Stop {
    /// Hard limit from the start.
    limit: Duration,
    /// After the first `SetRelays`: stop this much later (not past `limit`).
    tail: Option<Duration>,
}

impl Stop {
    fn after(limit: Duration) -> Self {
        Self { limit, tail: None }
    }

    fn after_first_set(limit: Duration, tail: Duration) -> Self {
        Self {
            limit,
            tail: Some(tail),
        }
    }
}

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

/// A relay whose own address and our address as it sees us are both `addr`.
fn relay(addr: &Ipv4Peer) -> RelayInfo {
    RelayInfo {
        relay_address: addr.clone(),
        peer_address: addr.clone(),
    }
}

/// The cluster nodes as relays: all of them answer pings.
fn live(cl: &Cluster) -> Vec<RelayInfo> {
    cl.addrs.iter().map(relay).collect()
}

/// A bound UDP socket that never answers, and its address as a relay; keep the socket alive.
fn mute() -> (UdpSocket, RelayInfo) {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("bind the silent socket");
    let port = socket.local_addr().expect("local_addr").port();
    let addr = Ipv4Peer {
        host: "127.0.0.1".into(),
        port,
    };
    (socket, relay(&addr))
}

/// The task for the key `key(1)` on `pinger` with the given timings.
fn keepalive(
    pinger: &Node,
    cmd_tx: UnboundedSender<Command>,
    period: Duration,
    rounds: u32,
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

/// The fake actor: answers `Relays` with the current `relays` (`None` = suspended), takes a
/// `SetRelays` as the new list (unless suspended), ignores the rest; ends by `stop` or when
/// the channel closes.
async fn play_actor(
    mut rx: UnboundedReceiver<Command>,
    mut relays: Option<Vec<RelayInfo>>,
    stop: Stop,
) -> Seen {
    let mut seen = Seen::default();
    let mut deadline = tokio::time::Instant::now() + stop.limit;
    while let Ok(Some(command)) = tokio::time::timeout_at(deadline, rx.recv()).await {
        match command {
            Command::Relays(reply) => {
                seen.asks.push(Instant::now());
                let _ = reply.send(relays.clone());
            }
            Command::SetRelays(list) => {
                let asks_before = seen.asks.len();
                seen.sets.push(Set {
                    asks_before,
                    relays: list.clone(),
                });
                relays = relays.map(|_| list);
                if let (1, Some(tail)) = (seen.sets.len(), stop.tail) {
                    deadline = deadline.min(tokio::time::Instant::now() + tail);
                }
            }
            _ => {}
        }
    }
    seen
}

/// Runs the keep-alive task of `pinger` against the fake actor until `stop`; what the actor saw.
async fn watch(
    pinger: &Node,
    relays: Option<Vec<RelayInfo>>,
    period: Duration,
    rounds: u32,
    stop: Stop,
) -> Seen {
    let (tx, rx) = mpsc::unbounded_channel();
    let task = tokio::spawn(keepalive(pinger, tx, period, rounds).run());
    let seen = play_actor(rx, relays, stop).await;
    task.abort();
    seen
}
