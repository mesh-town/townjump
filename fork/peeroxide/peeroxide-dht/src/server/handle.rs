// JS: hyperdht/lib/server.js
//! `Server`: the public handle. `listen` announces the key and starts serving,
//! `close` withdraws the announcement (JS `Server.listen` / `Server.close`).

use std::sync::{Mutex, MutexGuard};

use libudx::UdxRuntime;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::actor::{self, Command, State, relay_addresses};
use super::keepalive::Keepalive;
use super::opts::ServerOpts;
use crate::crypto::hash;
use crate::hyperdht::{HyperDhtError, HyperDhtHandle, KeyPair, PeerConnection, ServerEvent};
use crate::hyperdht_messages::RelayInfo;

/// Capacity of the channel of incoming connections; a full channel drops new ones.
const INCOMING_CAPACITY: usize = 16;

enum Phase {
    Idle,
    Starting,
    Listening(KeyPair),
    Closed,
}

/// A DHT server: accepts connections to the announced key pair.
///
/// Build it with [`Server::new`] from the event receiver of `hyperdht::spawn`,
/// call [`Server::listen`], and read connections from the returned receiver.
pub struct Server {
    cmd_tx: UnboundedSender<Command>,
    dht: HyperDhtHandle,
    state: Mutex<Phase>,
    refresh: Mutex<Option<JoinHandle<()>>>,
}

impl Server {
    /// Starts the server task (needs a running tokio runtime). Nothing is
    /// announced until [`Server::listen`]; `events` must be the receiver that
    /// belongs to `dht`.
    pub fn new(
        dht: HyperDhtHandle,
        events: UnboundedReceiver<ServerEvent>,
        rt: UdxRuntime,
        opts: ServerOpts,
    ) -> (Server, mpsc::Receiver<PeerConnection>) {
        let (incoming_tx, incoming_rx) = mpsc::channel(INCOMING_CAPACITY);
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let state = State::new(dht.clone(), rt, opts, incoming_tx);
        let freeze = dht.add_freezer();
        tokio::spawn(actor::run(
            state,
            events,
            cmd_rx,
            mpsc::unbounded_channel(),
            freeze,
        ));
        let server = Server {
            cmd_tx,
            dht,
            state: Mutex::new(Phase::Idle),
            refresh: Mutex::new(None),
        };
        (server, incoming_rx)
    }

    /// Announces `key_pair` and starts accepting connections to it.
    ///
    /// Fails with `AlreadyListening` on a second call and with `ServerClosed`
    /// after [`Server::close`]; an announce error leaves the server idle.
    pub async fn listen(&self, key_pair: KeyPair) -> Result<(), HyperDhtError> {
        self.enter_starting()?;
        let target = hash(&key_pair.public_key);
        self.dht.register_server(&target);
        let _ = self.cmd_tx.send(Command::Listen(key_pair.clone(), target));
        match self.dht.announce_server(target, &key_pair).await {
            Ok(announced) => {
                let _ = self
                    .cmd_tx
                    .send(Command::SetRelays(announced.relays.clone()));
                self.promote(key_pair, &announced.relays).await
            }
            Err(e) => {
                self.rollback(&target);
                Err(e)
            }
        }
    }

    /// Starts accepting connections to `key_pair` without announcing it and without the
    /// relay keep-alive: the caller owns the announcement (and its relays, see
    /// [`Server::set_relays`]). `HyperDhtHandle::resume()` still announces such a server itself.
    ///
    /// Fails like [`Server::listen`]: `AlreadyListening`, `ServerClosed`.
    pub(crate) fn listen_unannounced(&self, key_pair: KeyPair) -> Result<(), HyperDhtError> {
        self.enter_starting()?;
        let target = hash(&key_pair.public_key);
        self.dht.register_server(&target);
        let _ = self.cmd_tx.send(Command::Listen(key_pair.clone(), target));
        if !self.try_promote(&key_pair, false) {
            self.dht.unregister_server(&target);
            return Err(HyperDhtError::ServerClosed);
        }
        Ok(())
    }

    /// Sets the relays that go into the holepunch info of replies (those of the caller's announcement).
    pub(crate) fn set_relays(&self, relays: Vec<RelayInfo>) {
        let _ = self.cmd_tx.send(Command::SetRelays(relays));
    }

    /// Withdraws the announcement and stops accepting connections. Repeating it is harmless.
    pub async fn close(&self) -> Result<(), HyperDhtError> {
        let old = std::mem::replace(&mut *self.phase(), Phase::Closed);
        if matches!(old, Phase::Closed) {
            return Ok(());
        }
        self.stop_refresh();
        if let Phase::Listening(key_pair) = old {
            let relays = self.relays().await;
            self.withdraw(&key_pair, &relays).await;
        }
        let _ = self.cmd_tx.send(Command::Close);
        Ok(())
    }

    fn phase(&self) -> MutexGuard<'_, Phase> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn enter_starting(&self) -> Result<(), HyperDhtError> {
        let mut phase = self.phase();
        match *phase {
            Phase::Idle => *phase = Phase::Starting,
            Phase::Starting | Phase::Listening(_) => return Err(HyperDhtError::AlreadyListening),
            Phase::Closed => return Err(HyperDhtError::ServerClosed),
        }
        Ok(())
    }

    /// The announce succeeded: `Starting` becomes `Listening`. If `close`
    /// came in meanwhile, the announcement is withdrawn again.
    async fn promote(&self, key_pair: KeyPair, relays: &[RelayInfo]) -> Result<(), HyperDhtError> {
        if self.try_promote(&key_pair, true) {
            return Ok(());
        }
        self.withdraw(&key_pair, relays).await;
        Err(HyperDhtError::ServerClosed)
    }

    fn try_promote(&self, key_pair: &KeyPair, keepalive: bool) -> bool {
        let mut phase = self.phase();
        if !matches!(*phase, Phase::Starting) {
            return false;
        }
        *phase = Phase::Listening(key_pair.clone());
        if keepalive {
            let task = self.spawn_refresh(key_pair.clone());
            *self.refresh.lock().unwrap_or_else(|e| e.into_inner()) = Some(task);
        }
        true
    }

    /// The announce failed: the server goes back to `Idle` (unless closed).
    fn rollback(&self, target: &[u8; 32]) {
        self.dht.unregister_server(target);
        let _ = self.cmd_tx.send(Command::Close);
        let mut phase = self.phase();
        if matches!(*phase, Phase::Starting) {
            *phase = Phase::Idle;
        }
    }

    /// The relays of the latest announcement, from the actor; empty when suspended or gone.
    pub(crate) async fn relays(&self) -> Vec<RelayInfo> {
        let (tx, rx) = oneshot::channel();
        if self.cmd_tx.send(Command::Relays(tx)).is_err() {
            return vec![];
        }
        rx.await.ok().flatten().unwrap_or_default()
    }

    /// Stops routing handshakes to us, withdraws the record from `relays` (bounded by
    /// `UNANNOUNCE_DEADLINE`; errors are ignored), stops the actor's serving.
    async fn withdraw(&self, key_pair: &KeyPair, relays: &[RelayInfo]) {
        let target = hash(&key_pair.public_key);
        self.dht.unregister_server(&target);
        let addresses = relay_addresses(relays);
        self.dht
            .unannounce_relays(target, key_pair, &addresses)
            .await;
        let _ = self.cmd_tx.send(Command::Close);
    }

    fn stop_refresh(&self) {
        let task = self
            .refresh
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(task) = task {
            task.abort();
        }
    }

    /// Starts the relay keep-alive: pings, early and periodic re-announces (`keepalive`).
    fn spawn_refresh(&self, key_pair: KeyPair) -> JoinHandle<()> {
        let keepalive = Keepalive::new(self.dht.clone(), self.cmd_tx.clone(), key_pair);
        tokio::spawn(keepalive.run())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop_refresh();
    }
}

#[cfg(test)]
#[path = "handle_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "handle_unannounced_tests.rs"]
mod unannounced_tests;
