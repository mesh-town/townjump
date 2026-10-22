//! Rate limit for the "packet for unknown stream id" debug line of the receive loop (one line per id per second).

use std::collections::HashMap;
use std::time::{Duration, Instant};

const EVERY: Duration = Duration::from_secs(1);
const MAX_IDS: usize = 1024;
const STALE: Duration = Duration::from_secs(10);

#[derive(Default)]
pub(super) struct UnknownIdLog {
    last: HashMap<u32, Instant>,
}

impl UnknownIdLog {
    /// `true` if `id` was not seen or the last line for it was at least 1 s ago (the time is then updated).
    pub(super) fn note(&mut self, id: u32, now: Instant) -> bool {
        if self
            .last
            .get(&id)
            .is_some_and(|at| now.saturating_duration_since(*at) < EVERY)
        {
            return false;
        }
        if self.last.len() >= MAX_IDS {
            self.last
                .retain(|_, at| now.saturating_duration_since(*at) <= STALE);
            if self.last.len() >= MAX_IDS {
                self.last.clear();
            }
        }
        self.last.insert(id, now);
        true
    }
}

#[cfg(test)]
impl UnknownIdLog {
    pub(super) fn len(&self) -> usize {
        self.last.len()
    }
}
