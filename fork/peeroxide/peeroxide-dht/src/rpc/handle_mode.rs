// JS: dht-rpc/index.js
//! `impl DhtHandle`: reading and watching the node mode, switching adaptive mode.
//! Neighbours: `node_adaptive` publishes the mode; `command` carries `SetAdaptive`.

use tokio::sync::{oneshot, watch};

use super::command::DhtCommand;
use super::handle::DhtHandle;
use super::types::{DhtError, DhtMode};

impl DhtHandle {
    /// Current mode of the node (read without a trip through the node loop).
    pub fn mode(&self) -> DhtMode {
        self.mode.borrow().clone()
    }

    /// Receiver that is notified on every mode change.
    pub fn watch_mode(&self) -> watch::Receiver<DhtMode> {
        self.mode.clone()
    }

    /// Turns adaptive mode on or off; see `DhtNode::set_adaptive` for the exact effect.
    pub async fn set_adaptive(&self, on: bool) -> Result<(), DhtError> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(DhtCommand::SetAdaptive { on, reply_tx: tx })
            .map_err(|_| DhtError::ChannelClosed)?;
        rx.await.map_err(|_| DhtError::ChannelClosed)
    }
}

#[cfg(test)]
#[path = "handle_mode_tests.rs"]
mod tests;
