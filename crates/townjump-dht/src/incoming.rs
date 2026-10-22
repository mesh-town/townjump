// JS: the hyperdht server `connection` event (`server.on('connection', c => …)`) as a queue.
// Related: types.rs (`DhtConnection`), traits.rs (`Dht::create_server` returns `Incoming`).
use crate::types::DhtConnection;
use tokio::sync::mpsc;

/// Queue of a server's incoming connections.
#[derive(Debug)]
pub struct Incoming(mpsc::Receiver<DhtConnection>);

impl Incoming {
    /// A "sender for the server implementation, queue for the consumer" pair.
    pub fn channel(capacity: usize) -> (mpsc::Sender<DhtConnection>, Incoming) {
        let (tx, rx) = mpsc::channel(capacity);
        (tx, Incoming(rx))
    }

    /// Next incoming connection; `None` means the server is closed.
    pub async fn accept(&mut self) -> Option<DhtConnection> {
        self.0.recv().await
    }
}

#[cfg(test)]
#[path = "incoming_tests.rs"]
mod tests;
