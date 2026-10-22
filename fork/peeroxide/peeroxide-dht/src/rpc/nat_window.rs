// JS: nat-sampler/index.js
//! Node-level NAT sampler (dht-rpc `NatSampler`): ring of the last 16 `to` samples;
//! host/port follow the window. Not the holepunch `Nat` (`nat.rs`).

const SLOTS: usize = 32;

struct Entry {
    host: String,
    port: u16,
    hits: u32,
}

pub(super) struct NatWindow {
    host: Option<String>,
    port: u16,
    size: u32,
    threshold: u32,
    top: usize,
    a: Option<usize>,
    b: Option<usize>,
    slots: Vec<usize>,
    entries: Vec<Entry>,
}

impl NatWindow {
    pub(super) fn new() -> Self {
        Self {
            host: None,
            port: 0,
            size: 0,
            threshold: 0,
            top: 0,
            a: None,
            b: None,
            slots: Vec::with_capacity(SLOTS),
            entries: Vec::with_capacity(SLOTS + 4),
        }
    }

    pub(super) fn host(&self) -> Option<&str> {
        self.host.as_deref()
    }

    pub(super) fn port(&self) -> u16 {
        self.port
    }

    pub(super) fn size(&self) -> u32 {
        self.size
    }

    #[cfg(test)]
    pub(super) fn entries_len(&self) -> usize {
        self.entries.len()
    }

    /// Adds one `to` sample; returns the hits of its full-address entry (JS `add` return).
    pub(super) fn add(&mut self, host: &str, port: u16) -> u32 {
        let a = self.bump(host, port, 2);
        let b = self.bump(host, 0, 1);
        self.place(a, b);
        self.elect(a, b);
        self.entries[a].hits
    }

    /// Fills the ring while it is short, then overwrites the oldest pair of slots.
    fn place(&mut self, a: usize, b: usize) {
        if self.slots.len() < SLOTS {
            self.size += 1;
            let tolerated = match self.size {
                0..=3 => 0,
                4..=7 => 1,
                8..=11 => 2,
                _ => 3,
            };
            self.threshold = self.size - tolerated;
            self.slots.push(a);
            self.slots.push(b);
            self.top += 2;
            return;
        }
        if self.top == SLOTS {
            self.top = 0;
        }
        let oa = self.slots[self.top];
        self.slots[self.top] = a;
        self.top += 1;
        self.entries[oa].hits -= 1;
        let ob = self.slots[self.top];
        self.slots[self.top] = b;
        self.top += 1;
        self.entries[ob].hits -= 1;
    }

    /// Updates the two leaders and the reported host/port.
    fn elect(&mut self, a: usize, b: usize) {
        if self
            .a
            .is_none_or(|x| self.entries[x].hits < self.entries[a].hits)
        {
            self.a = Some(a);
        }
        if self
            .b
            .is_none_or(|x| self.entries[x].hits < self.entries[b].hits)
        {
            self.b = Some(b);
        }
        let (la, lb) = (self.a.expect("set above"), self.b.expect("set above"));
        if self.entries[la].hits >= self.threshold {
            self.host = Some(self.entries[la].host.clone());
            self.port = self.entries[la].port;
        } else if self.entries[lb].hits >= self.threshold {
            self.host = Some(self.entries[lb].host.clone());
            self.port = 0;
        } else {
            self.host = None;
            self.port = 0;
        }
    }

    fn bump(&mut self, host: &str, port: u16, inc: usize) -> usize {
        for i in 0..4 {
            // JS `& 31` on a negative number wraps modulo 32.
            let j = (self.top + 64 - inc - 2 * i) & (SLOTS - 1);
            if j >= self.slots.len() {
                return self.alloc(host, port);
            }
            let s = self.slots[j];
            if self.entries[s].port == port && self.entries[s].host == host {
                self.entries[s].hits += 1;
                return s;
            }
        }
        self.alloc(host, port)
    }

    /// New entry with hits 1: reuses an entry no slot points at (hits 0) unless it is a leader; else pushes.
    fn alloc(&mut self, host: &str, port: u16) -> usize {
        let free = (0..self.entries.len())
            .find(|&i| self.entries[i].hits == 0 && self.a != Some(i) && self.b != Some(i));
        let entry = Entry {
            host: host.to_owned(),
            port,
            hits: 1,
        };
        match free {
            Some(i) => {
                self.entries[i] = entry;
                i
            }
            None => {
                self.entries.push(entry);
                self.entries.len() - 1
            }
        }
    }
}

#[cfg(test)]
#[path = "nat_window_tests.rs"]
mod tests;
