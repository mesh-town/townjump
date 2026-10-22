// Absent from the JS mock: gates (`listen` and `mutable_put` wait for a release) and injected errors - for tests where order matters.
// Related: config.rs sets the settings, shared.rs stores `Switches`, server.rs and dht.rs call `before_listen`/`before_put`.
use crate::mock::config::MockConfig;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Semaphore;
use townjump_dht::DhtError;

/// Permits per release: with a margin so that waiting calls do not run out before all of them pass.
const RELEASE_PERMITS: usize = 1 << 16;
/// Error text of `mutable_put` from `failing_put_from`.
const PUT_FAILED: &str = "put failed";

/// Gate: calls wait until it is opened; without a gate they pass at once.
#[derive(Default)]
struct Gate(Option<Semaphore>);

impl Gate {
    fn closed_if(hold: bool) -> Self {
        Gate(hold.then(|| Semaphore::new(0)))
    }

    fn open(&self) {
        let Some(semaphore) = &self.0 else { return };
        semaphore.add_permits(RELEASE_PERMITS);
    }

    /// The permit is not returned (`forget`): an opened gate stays open.
    async fn pass(&self) {
        let Some(semaphore) = &self.0 else { return };
        // Nobody closes the semaphore, `acquire` does not fail; `else` only to avoid a panic.
        let Ok(permit) = semaphore.acquire().await else {
            return;
        };
        permit.forget();
    }
}

/// Gates, errors and the `mutable_put` counter of one network; by default the mock does not change.
#[derive(Default)]
pub(crate) struct Switches {
    listen_gate: Gate,
    put_gate: Gate,
    listen_error: Option<String>,
    put_error_from: Option<usize>,
    puts: AtomicUsize,
}

impl Switches {
    pub(crate) fn new(config: &MockConfig) -> Self {
        Switches {
            listen_gate: Gate::closed_if(config.hold_listen),
            put_gate: Gate::closed_if(config.hold_put),
            listen_error: config.listen_error.clone(),
            put_error_from: config.put_error_from,
            puts: AtomicUsize::new(0),
        }
    }

    /// Gate -> error; the real work of `listen` comes after.
    pub(crate) async fn before_listen(&self) -> Result<(), DhtError> {
        self.listen_gate.pass().await;
        self.listen_failure()
    }

    /// The counter grows before the gate: a stuck call is already counted. Then gate -> error.
    pub(crate) async fn before_put(&self) -> Result<(), DhtError> {
        let call = self.puts.fetch_add(1, Ordering::SeqCst) + 1;
        self.put_gate.pass().await;
        self.put_failure(call)
    }

    pub(crate) fn release_listen(&self) {
        self.listen_gate.open();
    }

    pub(crate) fn release_put(&self) {
        self.put_gate.open();
    }

    pub(crate) fn put_calls(&self) -> usize {
        self.puts.load(Ordering::SeqCst)
    }

    fn listen_failure(&self) -> Result<(), DhtError> {
        let Some(text) = &self.listen_error else {
            return Ok(());
        };
        Err(DhtError::Backend(text.clone()))
    }

    fn put_failure(&self, call: usize) -> Result<(), DhtError> {
        if self.put_error_from.is_some_and(|from| call >= from) {
            return Err(DhtError::Backend(PUT_FAILED.to_string()));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "knobs_tests.rs"]
mod tests;
