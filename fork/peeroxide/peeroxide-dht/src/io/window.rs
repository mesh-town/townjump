// JS: dht-rpc/lib/io.js
//! `CongestionWindow`, port of the JS CongestionWindow class.
//! Neighbours: `state` holds it inside `Io`.

// ── CongestionWindow ──────────────────────────────────────────────────────────

/// Congestion window — direct port of JS CongestionWindow class.
pub struct CongestionWindow {
    i: usize,
    pub(super) total: i32,
    pub(super) window: [i32; 4],
    max_window: i32,
}

impl CongestionWindow {
    pub fn new(max_window: usize) -> Self {
        Self {
            i: 0,
            total: 0,
            window: [0; 4],
            max_window: max_window as i32,
        }
    }

    /// Returns true if the window is full and no more sends should be attempted.
    pub fn is_full(&self) -> bool {
        self.total >= 2 * self.max_window || self.window[self.i] >= self.max_window
    }

    /// Decrement the current quarter (called on response received).
    pub fn recv(&mut self) {
        if self.window[self.i] > 0 {
            self.window[self.i] -= 1;
            self.total -= 1;
        }
    }

    /// Increment the current quarter (called on send).
    pub fn send(&mut self) {
        self.total += 1;
        self.window[self.i] += 1;
    }

    /// Advance to the next quarter, clearing the oldest.
    pub fn drain(&mut self) {
        self.i = (self.i + 1) & 3;
        self.total -= self.window[self.i];
        self.window[self.i] = 0;
    }

    /// Reset all counters.
    pub fn clear(&mut self) {
        self.i = 0;
        self.total = 0;
        self.window = [0; 4];
    }
}
