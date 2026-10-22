//! In-memory framed stream and helpers shared by the blind-relay test files.
// JS: blind-relay/index.js

use super::*;
use crate::protomux::FramedStream;
use tokio::sync::mpsc;

pub(super) struct MemStream {
    rx: mpsc::UnboundedReceiver<Vec<u8>>,
    tx: mpsc::UnboundedSender<Vec<u8>>,
}

impl FramedStream for MemStream {
    async fn read_frame(&mut self) -> std::io::Result<Option<Vec<u8>>> {
        Ok(self.rx.recv().await)
    }

    async fn write_frame(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.tx
            .send(data.to_vec())
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "closed"))
    }
}

pub(super) fn mem_pair() -> (MemStream, MemStream) {
    let (tx_a, rx_b) = mpsc::unbounded_channel();
    let (tx_b, rx_a) = mpsc::unbounded_channel();
    (
        MemStream { rx: rx_a, tx: tx_a },
        MemStream { rx: rx_b, tx: tx_b },
    )
}

pub(super) fn noop_outbound() -> mpsc::UnboundedSender<SessionOutbound> {
    let (tx, _rx) = mpsc::unbounded_channel();
    tx
}
