// JS: dht-rpc/index.js
//! Adaptive-mode countdown (`_updateNetworkState` scheduling): pure state, no sockets.
//! Neighbours: `node_adaptive` drives it from ticks; `nat_check` holds the check round.

/// Tick countdown that decides when an ephemeral node should try to become persistent.
pub(super) struct Adaptive {
    on: bool,
    stable_ticks: u64,
    countdown: u64,
    last_host: Option<String>,
}

impl Adaptive {
    pub(super) fn new(on: bool, stable_ticks: u64) -> Self {
        Self {
            on,
            stable_ticks,
            countdown: if on { stable_ticks } else { 0 },
            last_host: None,
        }
    }

    /// Back-off after a check round: three times the base interval.
    pub(super) fn more_stable(&self) -> u64 {
        3 * self.stable_ticks
    }

    /// Returns true when the caller should run a network-state check now.
    pub(super) fn on_tick(&mut self, ephemeral: bool, nat_host: Option<&str>) -> bool {
        if !self.on || !ephemeral {
            return false;
        }
        self.countdown = self.countdown.saturating_sub(1);
        if self.countdown > 0 {
            return false;
        }
        // Same host as the last round (including none == none, like JS `null === null`): nothing new to learn.
        if self.last_host.as_deref() == nat_host {
            self.countdown = self.more_stable();
            return false;
        }
        true
    }

    pub(super) fn begin_check(&mut self, nat_host: Option<&str>) {
        self.countdown = self.more_stable();
        self.last_host = nat_host.map(str::to_owned);
    }

    /// Resets the schedule after a pause; true means the node must drop back to ephemeral.
    pub(super) fn on_wakeup(&mut self, ephemeral: bool) -> bool {
        self.countdown = self.more_stable();
        self.last_host = None;
        self.on && !ephemeral
    }

    pub(super) fn enable(&mut self) {
        self.on = true;
        self.countdown = self.stable_ticks;
        self.last_host = None;
    }

    /// Returns whether adaptive mode was on before.
    pub(super) fn disable(&mut self) -> bool {
        std::mem::replace(&mut self.on, false)
    }

    pub(super) fn is_on(&self) -> bool {
        self.on
    }
}

#[cfg(test)]
#[path = "adaptive_tests.rs"]
mod tests;
