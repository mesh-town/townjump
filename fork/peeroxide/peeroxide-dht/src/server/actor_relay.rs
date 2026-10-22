// JS: hyperdht/lib/server.js
//! The actor's side of relayed connections: the pair arrived, the connection is served
//! over the relay, the puzzle outlives the pending slot (`State.relayed`).

use std::time::Instant;

use tokio::sync::mpsc::UnboundedSender;

use super::actor::{Internal, State};
use super::conn::Pending;
use super::relay::{self, Relayed};
use super::table::INITIAL_TIMEOUT;
use crate::hyperdht::{HyperDhtError, RelayPaired};

/// The outcome of a relay pairing for the slot `id`.
pub(super) fn on_relay_paired(
    st: &mut State,
    id: u64,
    paired: Result<RelayPaired, HyperDhtError>,
    internal: &UnboundedSender<Internal>,
) {
    match paired {
        Err(e) => {
            tracing::debug!(err = %e, "server: relay aborted");
            if let Some(pending) = st.table.get_mut(id) {
                pending.relay = None;
            }
        }
        Ok(paired) => on_paired(st, id, paired, internal),
    }
}

fn on_paired(st: &mut State, id: u64, paired: RelayPaired, internal: &UnboundedSender<Internal>) {
    let Some(mut pending) = st.table.take(id) else {
        tracing::debug!("server: relay paired after the client arrived, closed");
        return;
    };
    let now = Instant::now();
    st.table.clear_later(id, now);
    pending.relay = None;
    if let Some(punch) = pending.punch.take() {
        let deadline = now + INITIAL_TIMEOUT;
        st.relayed.insert(id, Relayed { punch, deadline });
    }
    let (tx, done) = (st.incoming.clone(), internal.clone());
    tokio::spawn(relay::finish_relayed(pending, paired, id, done, tx));
}

/// The relayed connection is direct or closed: its puzzle is no longer needed.
pub(super) fn on_relay_done(st: &mut State, id: u64) {
    st.relayed
        .remove(&id)
        .into_iter()
        .for_each(|r| r.punch.destroy());
}

/// Destroys the puzzles of relayed connections past their deadline.
pub(super) fn expire_relayed(st: &mut State, now: Instant) {
    let due: Vec<u64> = (st.relayed.iter())
        .filter(|(_, r)| r.deadline <= now)
        .map(|(id, _)| *id)
        .collect();
    due.into_iter().for_each(|id| on_relay_done(st, id));
}

/// Destroys every puzzle of a relayed connection.
pub(super) fn stop_relayed(st: &mut State) {
    let ids: Vec<u64> = st.relayed.keys().copied().collect();
    ids.into_iter().for_each(|id| on_relay_done(st, id));
}

/// A round started the puzzle: the slot (or the relayed record) waits longer.
pub(super) fn extend_slot(st: &mut State, id: u64, now: Instant) {
    let until = now + INITIAL_TIMEOUT;
    if let Some(r) = st.relayed.get_mut(&id) {
        r.deadline = r.deadline.max(until);
        return;
    }
    let held = st.table.get_mut(id).and_then(|p| p.relay.as_ref());
    let until = held.map_or(until, |r| until.max(r.deadline));
    st.table.extend(id, until);
}

/// The client gave up: the puzzle goes; a slot that waits for the relay stays.
pub(super) fn abort_slot(st: &mut State, id: u64, now: Instant) {
    on_relay_done(st, id);
    let Some(pending) = st.table.get_mut(id) else {
        return;
    };
    if pending.relay.is_some() {
        pending.punch.take().into_iter().for_each(|p| p.destroy());
        return;
    }
    st.table.take(id).into_iter().for_each(Pending::destroy);
    st.table.clear_later(id, now);
}
