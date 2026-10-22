//! `RelayLink`: the handle a relayed connection keeps on its relay (current path, session upkeep).
// JS: hyperdht/lib/connect.js:793-838, hyperdht/lib/relay-connection.js:20-72

use libudx::StreamControl;
use tokio::sync::watch;

use super::RelayPath;
use super::pair::RelaySession;

/// The relay side of a connection: reports which path the data takes now.
pub(crate) struct RelayLink {
    path: watch::Receiver<RelayPath>,
}

impl RelayLink {
    /// A link over a path channel whose sender a supervisor task keeps.
    pub(super) fn from_path(path: watch::Receiver<RelayPath>) -> Self {
        Self { path }
    }

    /// The current path; changes are observable through the receiver.
    pub(crate) fn path(&self) -> watch::Receiver<RelayPath> {
        self.path.clone()
    }

    /// A link that stays on the relay: a detached task keeps `session` until the stream closes.
    pub(crate) fn relayed_only(control: StreamControl, session: RelaySession) -> Self {
        let (tx, path) = watch::channel(RelayPath::Relayed);
        tokio::spawn(async move {
            let _tx = tx;
            control.closed().await;
            session.close().await;
        });
        Self { path }
    }
}

#[cfg(test)]
#[path = "link_tests.rs"]
mod tests;
