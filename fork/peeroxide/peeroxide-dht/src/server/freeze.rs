// JS: hyperdht/lib/server.js (suspend, resume) + lib/announcer.js (suspend, resume)
//! The server actor's side of `HyperDhtHandle::suspend`/`resume`.

use super::actor::{State, relay_addresses};
use super::conn::Pending;
use crate::hyperdht::Freeze;

/// One order from the node; the answer tells it that this server is done.
pub(super) async fn on_freeze(st: &mut State, order: Freeze) {
    match order {
        Freeze::Suspend(done) => {
            suspend(st).await;
            let _ = done.send(());
        }
        Freeze::Resume(done) => {
            resume(st).await;
            let _ = done.send(());
        }
    }
}

/// `suspended`, half-open handshakes dropped (JS `_clearAll`), announcement withdrawn.
async fn suspend(st: &mut State) {
    st.suspended = true;
    st.table.clear_all().into_iter().for_each(Pending::destroy);
    let Some(listening) = st.listening.as_ref() else {
        return;
    };
    let relays = relay_addresses(&listening.relays);
    st.dht
        .unannounce_relays(listening.target, &listening.key_pair, &relays)
        .await;
}

/// Serving again, and announced again with fresh relays (JS `announcer.resume` → `refresh`).
async fn resume(st: &mut State) {
    st.suspended = false;
    let Some(listening) = st.listening.as_ref() else {
        return;
    };
    let announced = st
        .dht
        .announce_server(listening.target, &listening.key_pair)
        .await;
    match (announced, st.listening.as_mut()) {
        (Ok(announced), Some(listening)) => listening.relays = announced.relays,
        (Err(e), _) => tracing::debug!(err = %e, "server: announce after resume failed"),
        _ => {}
    }
}
