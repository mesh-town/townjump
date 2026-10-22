// JS: hyperdht/index.js (suspend, resume, `listening`)
//! `HyperDhtHandle::suspend`/`resume`: the servers go first (they withdraw their
//! announcement while the node still talks), then the node itself is frozen.

use std::time::Duration;

use tokio::sync::{mpsc, oneshot};

use super::{DhtError, HyperDhtError, HyperDhtHandle};

/// How long a server actor may take to answer an order before the node goes on without it.
const FREEZE_WAIT: Duration = Duration::from_secs(10);

/// An order from the node to each server actor; the actor answers on the channel when done.
pub(crate) enum Freeze {
    Suspend(oneshot::Sender<()>),
    Resume(oneshot::Sender<()>),
}

impl HyperDhtHandle {
    /// Like JS `dht.suspend()`: servers unannounce and drop half-open handshakes, then the
    /// node stops answering and its requests in flight fail with `Dht(DhtError::Suspended)`.
    /// New `connect`s fail the same way. Established connections are NOT torn down (the caller
    /// owns them). Repeating it is harmless.
    pub async fn suspend(&self) -> Result<(), HyperDhtError> {
        if self.is_suspended() {
            return Ok(());
        }
        self.freeze_servers(Freeze::Suspend).await;
        Ok(self.dht.suspend().await?)
    }

    /// Like JS `dht.resume()`: the node answers and refreshes its table, then servers announce
    /// again. Harmless when the node is not suspended.
    pub async fn resume(&self) -> Result<(), HyperDhtError> {
        if !self.is_suspended() {
            return Ok(());
        }
        self.dht.resume().await?;
        self.freeze_servers(Freeze::Resume).await;
        Ok(())
    }

    /// Whether [`HyperDhtHandle::suspend`] is in force.
    pub fn is_suspended(&self) -> bool {
        self.dht.is_suspended()
    }

    /// The error of an outgoing action while suspended (JS `SUSPENDED`).
    pub(super) fn ensure_running(&self) -> Result<(), HyperDhtError> {
        match self.is_suspended() {
            true => Err(DhtError::Suspended.into()),
            false => Ok(()),
        }
    }

    /// A server actor asks for orders (JS: the server joins `dht.listening`).
    pub(crate) fn add_freezer(&self) -> mpsc::UnboundedReceiver<Freeze> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.freezers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(tx);
        rx
    }

    async fn freeze_servers(&self, order: fn(oneshot::Sender<()>) -> Freeze) {
        for ack in self.send_orders(order) {
            let _ = tokio::time::timeout(FREEZE_WAIT, ack).await;
        }
    }

    /// Sends one order to every live server actor and forgets the dead ones.
    fn send_orders(&self, order: fn(oneshot::Sender<()>) -> Freeze) -> Vec<oneshot::Receiver<()>> {
        let mut acks = Vec::new();
        let mut freezers = self.freezers.lock().unwrap_or_else(|e| e.into_inner());
        freezers.retain(|tx| {
            let (ack_tx, ack_rx) = oneshot::channel();
            let sent = tx.send(order(ack_tx)).is_ok();
            if sent {
                acks.push(ack_rx);
            }
            sent
        });
        acks
    }
}
