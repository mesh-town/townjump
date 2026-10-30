//! Ownership and shutdown of every relay-service task.
//!
//! All tasks of the service (accept loop, self-announce, sweep, bridge, each
//! session, its Mux driver and every bridged pair) live in one flat
//! [`TaskSet`]. There is no hierarchy: aborting the set stops everything at
//! once. [`RelayServiceHandle`] owns the set; `close` withdraws the service,
//! `Drop` aborts it without network calls. Siblings: `accept`, `bridge`.
// JS: hyperdht/lib/server.js (`close`)

use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::task::JoinSet;

use crate::hyperdht::{HyperDhtHandle, KeyPair};
use crate::server::Server;

/// How long `close` waits for aborted tasks to unwind.
const DRAIN_LIMIT: Duration = Duration::from_secs(1);
/// How long `close` waits for the self-announce record to be withdrawn.
const UNANNOUNCE_LIMIT: Duration = Duration::from_secs(5);

struct Inner {
    /// `None` once the set is closed: later spawns are dropped unstarted.
    set: Mutex<Option<JoinSet<()>>>,
    /// One clone per live task future; released when the future is dropped.
    alive: Arc<()>,
}

/// Flat, shared set of the service's tasks (cheap to clone).
#[derive(Clone)]
pub(super) struct TaskSet(Arc<Inner>);

impl TaskSet {
    pub(super) fn new() -> Self {
        Self(Arc::new(Inner {
            set: Mutex::new(Some(JoinSet::new())),
            alive: Arc::new(()),
        }))
    }

    fn lock(&self) -> MutexGuard<'_, Option<JoinSet<()>>> {
        self.0.set.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Run `fut` as a task of the service; dropped unstarted once the set is closed.
    pub(super) fn spawn<F>(&self, fut: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let mut guard = self.lock();
        let Some(set) = guard.as_mut() else {
            return;
        };
        reap(set);
        let mark = Arc::clone(&self.0.alive);
        set.spawn(async move {
            let _mark = mark;
            fut.await;
        });
    }

    /// Close the set and hand it over; later spawns are dropped.
    pub(super) fn take(&self) -> Option<JoinSet<()>> {
        self.lock().take()
    }

    /// Close the set and abort every task, without waiting and without network calls.
    pub(super) fn abort_now(&self) {
        drop(self.take());
    }

    /// No task future is alive (an aborted task counts until its future is dropped).
    pub(super) fn idle(&self) -> bool {
        Arc::strong_count(&self.0.alive) == 1
    }

    #[cfg(test)]
    pub(super) fn probe(&self) -> TaskProbe {
        TaskProbe(Arc::downgrade(&self.0.alive))
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.lock().as_ref().map_or(0, JoinSet::len)
    }
}

/// Drop finished entries so the set does not grow with completed tasks.
fn reap(set: &mut JoinSet<()>) {
    while let Some(done) = set.try_join_next() {
        match done {
            Err(e) if e.is_panic() => tracing::warn!(err = %e, "relay: service task panicked"),
            _ => {}
        }
    }
}

/// Test-only view of a [`TaskSet`] that outlives the handle.
#[cfg(test)]
pub(super) struct TaskProbe(std::sync::Weak<()>);

#[cfg(test)]
impl TaskProbe {
    /// Every task future and every clone of the set are gone.
    pub(super) fn is_finished(&self) -> bool {
        self.0.strong_count() == 0
    }
}

/// Stop handle for a relay started with [`super::run_relay_server`].
///
/// Owns every task of the service. `close` stops the tasks, unregisters the
/// relay and withdraws its self-announce; dropping the handle only aborts the
/// tasks. Statistics of the shared engine stay as they were at the stop.
pub struct RelayServiceHandle {
    tasks: TaskSet,
    dht: HyperDhtHandle,
    key_pair: KeyPair,
    target: [u8; 32],
    announce: bool,
    /// The control-connection server, dropped on `close` before the registration is withdrawn.
    server: Option<Arc<Server>>,
}

impl RelayServiceHandle {
    pub(super) fn new(
        tasks: TaskSet,
        dht: HyperDhtHandle,
        key_pair: KeyPair,
        target: [u8; 32],
        announce: bool,
        server: Arc<Server>,
    ) -> Self {
        Self {
            tasks,
            dht,
            key_pair,
            target,
            announce,
            server: Some(server),
        }
    }

    /// The relays the control-connection server currently advertises.
    #[cfg(test)]
    pub(super) async fn server_relays(&self) -> Vec<crate::hyperdht_messages::RelayInfo> {
        match &self.server {
            Some(server) => server.relays().await,
            None => Vec::new(),
        }
    }

    /// Stop every task, unregister the relay, then withdraw the self-announce
    /// (if enabled, at most 5 s; failures are only logged).
    pub async fn close(mut self) {
        if let Some(mut set) = self.tasks.take() {
            set.abort_all();
            let drained = tokio::time::timeout(DRAIN_LIMIT, async {
                while set.join_next().await.is_some() {}
            })
            .await;
            if drained.is_err() {
                tracing::debug!("relay: tasks still unwinding after close");
            }
        }
        self.server.take();
        self.dht.unregister_server(&self.target);
        if self.announce {
            self.withdraw_announce().await;
        }
    }

    async fn withdraw_announce(&self) {
        let unannounce = self.dht.unannounce(self.target, &self.key_pair);
        match tokio::time::timeout(UNANNOUNCE_LIMIT, unannounce).await {
            Ok(Ok(())) => tracing::debug!("relay: self-announce withdrawn"),
            Ok(Err(e)) => tracing::debug!(err = %e, "relay: unannounce failed"),
            Err(_) => tracing::debug!("relay: unannounce timed out"),
        }
    }

    /// Every task of the service has finished (stopped, aborted, or the node died).
    pub fn is_finished(&self) -> bool {
        self.tasks.idle()
    }
}

impl Drop for RelayServiceHandle {
    fn drop(&mut self) {
        self.tasks.abort_now();
    }
}

impl std::fmt::Debug for RelayServiceHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayServiceHandle")
            .field("target", &self.target)
            .field("announce", &self.announce)
            .field("finished", &self.is_finished())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "stop_tests.rs"]
mod stop_tests;
