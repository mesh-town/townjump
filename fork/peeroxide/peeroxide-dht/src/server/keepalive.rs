// JS: hyperdht/lib/announcer.js (`_background`, `MIN_ACTIVE`)
//! Keeps the NAT mappings between a listening server and its relays alive: pings the relays of the
//! latest announcement, re-announces early when too few answer and fully every ~5 min. The relay list
//! is held by the actor (`Command::Relays`); `handle` spawns this task, `freeze` re-announces on resume.

use std::time::Duration;

use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;
use tokio::task::JoinSet;

use super::actor::{Command, relay_addresses};
use crate::crypto::hash;
use crate::hyperdht::{HyperDhtHandle, KeyPair};
use crate::hyperdht_messages::RelayInfo;
use crate::messages::Ipv4Peer;
use crate::rpc::DhtHandle;

/// Pause before each ping round (JS `this._sleeper.pause(3000)`).
const PING_PERIOD: Duration = Duration::from_secs(3);
/// Ping rounds between full re-announces, ~5 min (JS `for (let i = 0; i < 100; ...)`).
const PINGS_PER_REFRESH: u32 = 100;
/// Fewer answers than `min(relays, MIN_ACTIVE)` mean the relays are lost (JS `MIN_ACTIVE`).
const MIN_ACTIVE: usize = 3;

/// Why a cycle of ping rounds ended.
#[derive(Debug, PartialEq, Eq)]
enum End {
    /// Too few relays answered: re-announce now.
    Lost,
    /// `rounds` healthy rounds: the periodic re-announce.
    Expired,
    /// The actor answered `None`: no pings, no re-announce (`freeze` announces on resume).
    Suspended,
    /// The actor is gone.
    Closed,
}

/// The keep-alive task of one listening server; `rounds >= 1`.
pub(super) struct Keepalive {
    dht: HyperDhtHandle,
    cmd_tx: UnboundedSender<Command>,
    key_pair: KeyPair,
    target: [u8; 32],
    period: Duration,
    rounds: u32,
}

/// JS `active < Math.min(pings.length, MIN_ACTIVE)`: never true without relays.
fn needs_reannounce(alive: usize, total: usize) -> bool {
    alive < total.min(MIN_ACTIVE)
}

impl Keepalive {
    /// The task for `key_pair` with the JS timings.
    pub(super) fn new(
        dht: HyperDhtHandle,
        cmd_tx: UnboundedSender<Command>,
        key_pair: KeyPair,
    ) -> Self {
        let target = hash(&key_pair.public_key);
        Self {
            dht,
            cmd_tx,
            key_pair,
            target,
            period: PING_PERIOD,
            rounds: PINGS_PER_REFRESH,
        }
    }

    /// Runs until the actor is gone; `Server` aborts it on `close` and drop.
    pub(super) async fn run(self) {
        loop {
            match self.cycle().await {
                End::Closed => return,
                End::Suspended => {}
                End::Lost | End::Expired => self.reannounce().await,
            }
        }
    }

    /// Up to `rounds` rounds, each after a pause of `period`; ends at the first unhealthy one.
    async fn cycle(&self) -> End {
        for _ in 0..self.rounds {
            tokio::time::sleep(self.period).await;
            if let Err(end) = self.round().await {
                return end;
            }
        }
        End::Expired
    }

    /// Pings every relay the actor holds; `Err(Lost)` when too few answered.
    async fn round(&self) -> Result<(), End> {
        let relays = self.relays().await?;
        let alive = ping_all(self.dht.dht(), &relays).await;
        if !needs_reannounce(alive, relays.len()) {
            return Ok(());
        }
        tracing::debug!(alive, total = relays.len(), "server: early re-announce");
        Err(End::Lost)
    }

    /// The actor's relay list, or `Err(Suspended)` / `Err(Closed)`.
    async fn relays(&self) -> Result<Vec<RelayInfo>, End> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(Command::Relays(tx))
            .map_err(|_| End::Closed)?;
        rx.await.map_err(|_| End::Closed)?.ok_or(End::Suspended)
    }

    /// Announces again unless suspended or closed meanwhile, hands the new relays to the actor and
    /// withdraws the record from the relays that dropped out (in the background).
    async fn reannounce(&self) {
        let Ok(old) = self.relays().await else {
            return;
        };
        match self.dht.announce_server(self.target, &self.key_pair).await {
            Ok(announced) => {
                tracing::debug!(relays = announced.relays.len(), "server: re-announce");
                let gone = dropped(&old, &announced.relays);
                let _ = self.cmd_tx.send(Command::SetRelays(announced.relays));
                self.withdraw_dropped(gone);
            }
            Err(e) => tracing::debug!(err = %e, "server: re-announce failed"),
        }
    }

    /// Withdraws the record from `gone` without waiting: the ping rounds must not stall
    /// (the task ends by itself within `UNANNOUNCE_DEADLINE`).
    fn withdraw_dropped(&self, gone: Vec<Ipv4Peer>) {
        if gone.is_empty() {
            return;
        }
        let (dht, key_pair, target) = (self.dht.clone(), self.key_pair.clone(), self.target);
        tokio::spawn(async move {
            dht.unannounce_relays(target, &key_pair, &gone).await;
        });
    }
}

/// Relays of `old` that are not in `new` (same host and port), in the order of `old`.
fn dropped(old: &[RelayInfo], new: &[RelayInfo]) -> Vec<Ipv4Peer> {
    let kept = |a: &Ipv4Peer| {
        new.iter()
            .any(|r| r.relay_address.host == a.host && r.relay_address.port == a.port)
    };
    relay_addresses(old)
        .into_iter()
        .filter(|a| !kept(a))
        .collect()
}

/// Pings all relays at once (JS `resolved(pings)`); how many answered.
async fn ping_all(dht: &DhtHandle, relays: &[RelayInfo]) -> usize {
    let mut pings = JoinSet::new();
    for relay in relays {
        pings.spawn(ping(dht.clone(), relay.relay_address.clone()));
    }
    pings.join_all().await.into_iter().filter(|ok| *ok).count()
}

/// One ping (JS `dht.ping(node)`: 1 s × 4 sends); whether it was answered.
async fn ping(dht: DhtHandle, relay: Ipv4Peer) -> bool {
    let ok = dht.ping(&relay.host, relay.port).await.is_ok();
    tracing::debug!(relay = %format!("{}:{}", relay.host, relay.port), ok, "server: relay ping");
    ok
}

#[cfg(test)]
#[path = "keepalive_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "keepalive_drop_tests.rs"]
mod drop_tests;
