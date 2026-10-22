//! Windowed max filter for BBR bandwidth estimation.

/// 3-entry windowed max filter for bandwidth estimation.
/// Window is measured in rounds (BBR_BW_FILTER_CYCLES = 10).
/// Matches the TCP BBR windowed max filter used in C libudx.
#[derive(Debug, Clone)]
pub(crate) struct WindowedMaxFilter {
    entries: [(f64, u32); 3],
}

impl WindowedMaxFilter {
    /// Create a new filter with all entries zeroed.
    pub(crate) fn new() -> Self {
        Self {
            entries: [(0.0, 0); 3],
        }
    }

    /// Get the current maximum value.
    pub(crate) fn get(&self) -> f64 {
        self.entries[0].0
    }

    /// Update the filter with a new sample at the given round.
    /// Window is the max age in rounds (typically BBR_BW_FILTER_CYCLES = 10).
    pub(crate) fn update(&mut self, val: f64, round_count: u32, window: u32) {
        let new = (val, round_count);
        // New max or entire window expired — forget everything. C: win_filter_f64.c:73-92.
        if val >= self.entries[0].0 || self.age(2, round_count) > window {
            self.entries = [new; 3];
            return;
        }
        self.keep_runner_up(new);
        self.apply_common(new, window);
    }

    fn keep_runner_up(&mut self, new: (f64, u32)) {
        if new.0 >= self.entries[1].0 {
            self.entries[1] = new;
            self.entries[2] = new;
            return;
        }
        if new.0 >= self.entries[2].0 {
            self.entries[2] = new;
        }
    }

    /// Rounds since entry `i` was taken.
    fn age(&self, i: usize, round_count: u32) -> u32 {
        round_count.wrapping_sub(self.entries[i].1)
    }

    /// Age out the best entry or refresh stale sub-window entries. C: win_filter_f64.c:38-63.
    fn apply_common(&mut self, new: (f64, u32), window: u32) {
        let dt = self.age(0, new.1);
        if dt > window {
            self.expire(new, window);
            return;
        }
        if self.entries[1].1 == self.entries[0].1 && dt > window / 4 {
            self.entries[1] = new;
            self.entries[2] = new;
            return;
        }
        if self.entries[2].1 == self.entries[1].1 && dt > window / 2 {
            self.entries[2] = new;
        }
    }

    /// Drop the best entry, twice when the next one is out of the window too.
    fn expire(&mut self, new: (f64, u32), window: u32) {
        self.shift(new);
        if self.age(0, new.1) > window {
            self.shift(new);
        }
    }

    fn shift(&mut self, new: (f64, u32)) {
        self.entries = [self.entries[1], self.entries[2], new];
    }

    /// Reset all entries to zero.
    pub(crate) fn reset(&mut self) {
        self.entries = [(0.0, 0); 3];
    }
}

#[cfg(test)]
mod tests {
    use super::WindowedMaxFilter;

    #[test]
    fn test_new_returns_zero() {
        let filter = WindowedMaxFilter::new();
        assert_eq!(filter.get(), 0.0);
    }

    #[test]
    fn test_single_update() {
        let mut filter = WindowedMaxFilter::new();
        filter.update(5.0, 0, 10);
        assert_eq!(filter.get(), 5.0);
    }

    #[test]
    fn test_larger_replaces_max() {
        let mut filter = WindowedMaxFilter::new();
        filter.update(5.0, 0, 10);
        filter.update(10.0, 1, 10);
        assert_eq!(filter.get(), 10.0);
    }

    #[test]
    fn test_expiry() {
        let mut filter = WindowedMaxFilter::new();
        filter.update(5.0, 0, 10);
        filter.update(3.0, 11, 10);
        assert_eq!(filter.get(), 3.0);
    }

    #[test]
    fn test_three_entries_tracked() {
        let mut filter = WindowedMaxFilter::new();
        filter.update(2.0, 0, 10);
        filter.update(7.0, 1, 10);
        filter.update(5.0, 2, 10);

        assert_eq!(filter.get(), 7.0);

        filter.update(8.0, 3, 10);
        assert_eq!(filter.get(), 8.0);
    }

    #[test]
    fn test_reset() {
        let mut filter = WindowedMaxFilter::new();
        filter.update(5.0, 0, 10);
        filter.update(10.0, 1, 10);
        filter.reset();
        assert_eq!(filter.get(), 0.0);
    }
}
