//! Repeated `PEER_HANDSHAKE` detection for the relay service: the same Noise
//! bytes are answered with the stored reply and never start a second session.
//! Siblings: `handshake`, `session`, `bridge`.
// JS: hyperdht/lib/server.js (`_connects`, `_clearLater`)

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long a Noise key is remembered after it was answered (JS `CLEAR_WAIT`).
const CLEAR_WAIT: Duration = Duration::from_secs(10);

struct Entry {
    reply: Option<Vec<u8>>,
    forget_at: Instant,
}

/// Answered Noise messages, keyed by the bytes of the client's first message.
#[derive(Default)]
pub(super) struct Dedup {
    seen: HashMap<Vec<u8>, Entry>,
}

impl Dedup {
    /// The stored reply if `noise` was answered before (`Some(None)` = it was
    /// refused, stay silent); forgets keys whose time is up.
    pub(super) fn known(&mut self, noise: &[u8], now: Instant) -> Option<Option<Vec<u8>>> {
        self.seen.retain(|_, e| e.forget_at > now);
        self.seen.get(noise).map(|e| e.reply.clone())
    }

    /// Stores the reply for `noise` until `now + CLEAR_WAIT`.
    pub(super) fn remember(&mut self, noise: Vec<u8>, reply: Option<Vec<u8>>, now: Instant) {
        let forget_at = now + CLEAR_WAIT;
        self.seen.insert(noise, Entry { reply, forget_at });
    }
}
