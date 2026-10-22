// JS: `server.listen(keyPair).then(...)` in holesail-server/index.js:start (without `await`); related: start.rs.
// `listen` starts synchronously (first poll in the calling thread) so that in the trace it comes before `mutableGet`.
use std::future::Future;
use std::task::{Context, Poll, Waker};
use tokio::task::JoinHandle;
use townjump_dht::{DhtError, DhtServer, KeyPair};

/// The first poll of `listen` is here; unfinished work continues as a task. `done` receives the result.
pub(crate) fn listen_now<S: DhtServer>(
    server: S,
    key_pair: KeyPair,
    done: impl FnOnce(Result<(), DhtError>) + Send + 'static,
) -> Option<JoinHandle<()>> {
    let mut listening = Box::pin(async move { server.listen(key_pair).await });
    let mut cx = Context::from_waker(Waker::noop());
    if let Poll::Ready(result) = listening.as_mut().poll(&mut cx) {
        done(result);
        return None;
    }
    Some(tokio::spawn(async move { done(listening.await) }))
}
