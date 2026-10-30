// JS: hyperdht/lib/server.js
//! Handshake bookkeeping (JS `_connects`, `_clearLater`): deduplication by
//! Noise bytes, slots of pending connections and their deadlines.
//!
//! No async and no clock: time is always passed in by the caller (the actor).

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use libudx::UdxSocket;

/// How long a Noise key is remembered after its slot is done (JS `CLEAR_WAIT`).
pub(super) const CLEAR_WAIT: Duration = Duration::from_secs(10);
/// How long a pending connection waits for the client (JS `HANDSHAKE_INITIAL_TIMEOUT`).
pub(super) const INITIAL_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a slot lives once a round started the puzzle. JS sets no deadline after the
/// punching round: the handshake lives until the probes end (1750 x 20 ms), while the client
/// gives up after 30 s.
pub(super) const PUNCHING_TIMEOUT: Duration = Duration::from_secs(35);

/// Outcome of [`Table::begin`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Begin {
    /// The Noise message was seen before: answer with the stored Noise reply
    /// (`None` = the first attempt was refused, stay silent).
    Repeat(Option<Vec<u8>>),
    /// New handshake: the id of its slot.
    Fresh(u64),
}

struct Slot<T> {
    noise: Vec<u8>,
    born: Instant,
    reply: Option<Vec<u8>>,
    /// Socket the stored reply must leave from (the hole-punch socket), if any.
    via: Option<UdxSocket>,
    item: Option<T>,
    deadline: Option<Instant>,
    forget_at: Option<Instant>,
}

impl<T> Slot<T> {
    fn new(noise: Vec<u8>, born: Instant) -> Self {
        Self {
            noise,
            born,
            reply: None,
            via: None,
            item: None,
            deadline: None,
            forget_at: None,
        }
    }

    /// Takes the item out if its deadline has passed and schedules forgetting.
    fn expire(&mut self, now: Instant) -> Option<T> {
        self.deadline.filter(|deadline| *deadline <= now)?;
        self.deadline = None;
        self.forget_at = Some(now + CLEAR_WAIT);
        self.item.take()
    }

    fn due_to_forget(&self, now: Instant) -> bool {
        self.forget_at.is_some_and(|at| at <= now)
    }
}

/// Slots of handshakes keyed by the Noise bytes of the client's first message.
pub(super) struct Table<T> {
    next_id: u64,
    ids: HashMap<Vec<u8>, u64>,
    slots: BTreeMap<u64, Slot<T>>,
}

impl<T> Default for Table<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Table<T> {
    /// Empty table; slot ids start at 0.
    pub(super) fn new() -> Self {
        Self {
            next_id: 0,
            ids: HashMap::new(),
            slots: BTreeMap::new(),
        }
    }

    /// A known Noise message repeats the stored reply, a new one gets a slot.
    pub(super) fn begin(&mut self, noise: &[u8], now: Instant) -> Begin {
        if let Some(reply) = self.known_reply(noise) {
            return Begin::Repeat(reply);
        }
        Begin::Fresh(self.insert(noise, now))
    }

    /// Stores the Noise reply; `None` (refused) schedules forgetting `CLEAR_WAIT`
    /// after the slot was created (`begin`), the actor has no separate clock here.
    pub(super) fn finish(&mut self, id: u64, reply: Option<Vec<u8>>) {
        let Some(slot) = self.slots.get_mut(&id) else {
            return;
        };
        if reply.is_none() {
            slot.forget_at = Some(slot.born + CLEAR_WAIT);
        }
        slot.reply = reply;
    }

    /// Remembers the socket the reply leaves from, for repeats of the same Noise message.
    pub(super) fn set_via(&mut self, id: u64, socket: UdxSocket) {
        if let Some(slot) = self.slots.get_mut(&id) {
            slot.via = Some(socket);
        }
    }

    /// The socket stored by [`Self::set_via`] for this Noise message.
    pub(super) fn via_of(&self, noise: &[u8]) -> Option<UdxSocket> {
        let id = self.ids.get(noise)?;
        self.slots.get(id)?.via.clone()
    }

    /// Puts the pending connection into the slot (unknown id: the item is dropped).
    pub(super) fn attach(&mut self, id: u64, item: T, deadline: Instant) {
        let Some(slot) = self.slots.get_mut(&id) else {
            return;
        };
        slot.item = Some(item);
        slot.deadline = Some(deadline);
    }

    /// Postpones the deadline of the pending connection in the slot.
    pub(super) fn extend(&mut self, id: u64, deadline: Instant) {
        let Some(slot) = self.slots.get_mut(&id) else {
            return;
        };
        if slot.item.is_some() {
            slot.deadline = Some(deadline);
        }
    }

    /// The pending connection of the slot, if it is still there.
    pub(super) fn get_mut(&mut self, id: u64) -> Option<&mut T> {
        self.slots.get_mut(&id)?.item.as_mut()
    }

    /// Takes the pending connection out; the Noise key stays until `clear_later`.
    pub(super) fn take(&mut self, id: u64) -> Option<T> {
        let slot = self.slots.get_mut(&id)?;
        slot.deadline = None;
        slot.item.take()
    }

    /// Forget the Noise key of the slot at `now + CLEAR_WAIT`.
    pub(super) fn clear_later(&mut self, id: u64, now: Instant) {
        if let Some(slot) = self.slots.get_mut(&id) {
            slot.forget_at = Some(now + CLEAR_WAIT);
        }
    }

    /// Takes out pending connections whose deadline passed (they get
    /// `clear_later`) and forgets keys whose `CLEAR_WAIT` is over.
    pub(super) fn expire(&mut self, now: Instant) -> Vec<T> {
        let mut items: Vec<T> = self
            .slots
            .values_mut()
            .filter_map(|slot| slot.expire(now))
            .collect();
        items.extend(self.forget_due(now));
        items
    }

    /// Empties the table and returns every pending connection.
    pub(super) fn clear_all(&mut self) -> Vec<T> {
        self.ids.clear();
        let slots = std::mem::take(&mut self.slots);
        slots.into_values().filter_map(|slot| slot.item).collect()
    }

    fn known_reply(&self, noise: &[u8]) -> Option<Option<Vec<u8>>> {
        let id = self.ids.get(noise)?;
        self.slots.get(id).map(|slot| slot.reply.clone())
    }

    fn insert(&mut self, noise: &[u8], now: Instant) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.ids.insert(noise.to_vec(), id);
        self.slots.insert(id, Slot::new(noise.to_vec(), now));
        id
    }

    /// Removes slots past `forget_at`; a connection still inside is returned.
    fn forget_due(&mut self, now: Instant) -> Vec<T> {
        let due: Vec<u64> = (self.slots.iter())
            .filter(|(_, slot)| slot.due_to_forget(now))
            .map(|(id, _)| *id)
            .collect();
        due.into_iter().filter_map(|id| self.remove(id)).collect()
    }

    fn remove(&mut self, id: u64) -> Option<T> {
        let slot = self.slots.remove(&id)?;
        self.ids.remove(&slot.noise);
        slot.item
    }
}

#[cfg(test)]
#[path = "table_tests.rs"]
mod tests;
