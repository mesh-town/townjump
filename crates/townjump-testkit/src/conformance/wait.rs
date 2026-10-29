// JS: `await dht.connect`, `server.on('connection')` with a limit instead of hanging (hyperdht/lib/hyperdht.js, server.js).
// Related: setup.rs (nodes and servers), data.rs (byte exchange); scenarios panic with the text of the violated condition.
use std::future::Future;
use std::time::Duration;
use townjump_dht::{ConnectOpts, Dht, DhtConnection, Incoming};

/// `connect` and `accept` on the live network: handshake and holepunch.
pub const ACCEPT_WAIT: Duration = Duration::from_secs(30);
/// Data exchange over a stream.
pub const STREAM_WAIT: Duration = Duration::from_secs(60);
/// A `connect` that must end with an error: hanging is a failure too.
pub const REFUSE_WAIT: Duration = Duration::from_secs(60);
/// For this long `Incoming::accept` must yield nothing if the connection is rejected.
pub const EMPTY_WAIT: Duration = Duration::from_millis(300);

fn reusable() -> ConnectOpts {
    ConnectOpts {
        reusable_socket: true,
        ..Default::default()
    }
}

/// Waits for `fut` no longer than `limit`; otherwise panics, naming what was being awaited.
pub async fn within<T>(limit: Duration, what: &str, fut: impl Future<Output = T>) -> T {
    let waited = tokio::time::timeout(limit, fut).await;
    waited.unwrap_or_else(|_| panic!("{what}: no result within {} s", limit.as_secs()))
}

/// `connect` must succeed.
pub async fn connect_ok<D: Dht>(dht: &D, remote: [u8; 32], what: &str) -> DhtConnection {
    let done = within(ACCEPT_WAIT, what, dht.connect(remote, reusable())).await;
    done.unwrap_or_else(|e| panic!("{what}: connect ended with an error: {e}"))
}

/// `connect` must end with an error, and no later than `REFUSE_WAIT`.
pub async fn connect_refused<D: Dht>(dht: &D, remote: [u8; 32], what: &str) {
    let done = within(REFUSE_WAIT, what, dht.connect(remote, reusable())).await;
    assert!(
        done.is_err(),
        "{what}: connect must have ended with an error"
    );
}

/// Next incoming connection.
pub async fn accept_one(incoming: &mut Incoming, what: &str) -> DhtConnection {
    let next = within(ACCEPT_WAIT, what, incoming.accept()).await;
    next.unwrap_or_else(|| panic!("{what}: server closed, no incoming connection"))
}

/// Nothing reached the incoming queue within `EMPTY_WAIT`.
pub async fn assert_empty(incoming: &mut Incoming, what: &str) {
    let next = tokio::time::timeout(EMPTY_WAIT, incoming.accept()).await;
    assert!(
        next.is_err(),
        "{what}: a rejected connection reached the incoming queue"
    );
}
