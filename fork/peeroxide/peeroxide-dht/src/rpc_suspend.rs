// JS: dht-rpc/index.js (suspend, resume, _onwakeup)
//! Freezing the DHT node. `DhtHandle::suspend`/`resume` are the user-facing calls;
//! the `DhtNode` methods run inside the actor. See `io_suspend.rs` for the socket side.

use std::sync::atomic::Ordering;

use tokio::sync::oneshot;
use tokio::time::Instant;

use super::{DhtCommand, DhtError, DhtHandle, DhtNode, OLD_NODE, StandaloneRequest};

impl DhtHandle {
    /// Freezes the node like JS `dht.suspend()`: incoming datagrams are dropped, requests in
    /// flight fail with [`DhtError::Suspended`], ticks stop. The sockets stay bound (same port
    /// after resume). Repeating it is harmless.
    pub async fn suspend(&self) -> Result<(), DhtError> {
        self.ask(|reply_tx| DhtCommand::Suspend { reply_tx }).await
    }

    /// Thaws the node like JS `dht.resume()`: traffic flows, the routing table is
    /// refreshed at once. Harmless when the node is not suspended.
    pub async fn resume(&self) -> Result<(), DhtError> {
        self.ask(|reply_tx| DhtCommand::Resume { reply_tx }).await
    }

    /// Whether the node is suspended (updated before `suspend` returns).
    pub fn is_suspended(&self) -> bool {
        self.suspended.load(Ordering::SeqCst)
    }

    async fn ask(
        &self,
        order: impl FnOnce(oneshot::Sender<()>) -> DhtCommand,
    ) -> Result<(), DhtError> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(order(tx))
            .map_err(|_| DhtError::ChannelClosed)?;
        rx.await.map_err(|_| DhtError::ChannelClosed)
    }
}

/// What a query gets when its result channel was dropped without a result.
pub(super) fn dropped_query_error(suspended: bool) -> DhtError {
    match suspended {
        true => DhtError::Suspended,
        false => DhtError::ChannelClosed,
    }
}

impl DhtNode {
    pub(super) fn is_suspended(&self) -> bool {
        self.suspended.load(Ordering::SeqCst)
    }

    /// Why a request could not be sent: the node is suspended, or gone.
    pub(super) fn unavailable(&self) -> DhtError {
        match self.is_suspended() {
            true => DhtError::Suspended,
            false => DhtError::Destroyed,
        }
    }

    pub(super) fn suspend_node(&mut self) {
        if self.destroyed || self.is_suspended() {
            return;
        }
        self.suspended.store(true, Ordering::SeqCst);
        self.io.suspend();
        self.forget_requests();
    }

    /// Everything that waited for an answer is failed; queries are dropped (their waiters
    /// see `Suspended`). Pings of the routing table keep the table as it is.
    fn forget_requests(&mut self) {
        self.active_queries.clear();
        self.tid_to_query.clear();
        self.bootstrap_query_id = None;
        self.repinging = 0;
        self.nat_check = None;
        let waiting: Vec<_> = self.standalone_tids.drain().map(|(_, r)| r).collect();
        waiting.into_iter().for_each(fail_suspended);
    }

    pub(super) fn resume_node(&mut self) {
        if self.destroyed || !self.is_suspended() {
            return;
        }
        self.io.resume();
        self.suspended.store(false, Ordering::SeqCst);
        self.last_tick_time = Instant::now();
        self.wake_up();
        self.restart_work();
    }

    /// JS `_onwakeup`: everything in the table looks old, so pings and a refresh follow.
    pub(super) fn wake_up(&mut self) {
        self.tick += 2 * OLD_NODE;
        self.tick += 8 - (self.tick & 7);
        self.refresh_ticks = 1;
        if self.adaptive.on_wakeup(self.io.ephemeral) {
            self.demote();
            self.restore_firewalled();
        }
    }

    /// After resume: finish the bootstrap that suspend cut off, or refresh the table.
    fn restart_work(&mut self) {
        if self.bootstrapped {
            return self.run_refresh();
        }
        let Ok(own_id) = self.table.lock().map(|t| *t.id()) else {
            return;
        };
        self.start_bootstrap(own_id);
    }
}

fn fail_suspended(request: StandaloneRequest) {
    match request {
        StandaloneRequest::Ping(tx) | StandaloneRequest::PingViaSocket(tx) => {
            let _ = tx.send(Err(DhtError::Suspended));
        }
        StandaloneRequest::UserRequest(tx) => {
            let _ = tx.send(Err(DhtError::Suspended));
        }
        StandaloneRequest::NatCheck
        | StandaloneRequest::Reping { .. }
        | StandaloneRequest::Check { .. } => {}
    }
}

#[cfg(test)]
#[path = "rpc_suspend_tests.rs"]
mod tests;
