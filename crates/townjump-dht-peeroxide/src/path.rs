// Relay path label: bridges the fork's `watch` of the connection path into the adapter-neutral `RelayPath`.
// See also: tunnels.rs and stream.rs (attach the label to every connection handed upward).
use peeroxide_dht::hyperdht::RelayPath as ForkRelayPath;
use tokio::sync::watch;
use townjump_dht::RelayPath;

/// Fork enum -> trait enum; the fork enum is `non_exhaustive`, anything but `Relayed` counts as direct.
fn map_path(path: ForkRelayPath) -> RelayPath {
    match path {
        ForkRelayPath::Relayed => RelayPath::Relayed,
        _ => RelayPath::Direct,
    }
}

/// Copies the current value at once; a task then carries changes until the source closes or all receivers are gone.
/// Must be called inside a tokio runtime.
pub(crate) fn bridge_path(mut src: watch::Receiver<ForkRelayPath>) -> watch::Receiver<RelayPath> {
    let (tx, rx) = watch::channel(map_path(*src.borrow_and_update()));
    tokio::spawn(async move {
        loop {
            tokio::select! {
                changed = src.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    let _ = tx.send(map_path(*src.borrow_and_update()));
                }
                () = tx.closed() => break,
            }
        }
    });
    rx
}

#[cfg(test)]
#[path = "relay_path_tests.rs"]
mod relay_path_tests;
