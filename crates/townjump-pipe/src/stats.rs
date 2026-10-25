// JS: the `stats` and `opts` arguments in hyper-cmd-lib-net/index.js:connPiper (`locCnt`, `remCnt`, `rejectCnt`, `onDestroy`).
// Related: piper.rs (writes the counters and calls `on_destroy`).
use portable_atomic::AtomicI64;
use std::io;
use std::sync::Arc;
use townjump_logger::Logger;

/// Join counters: local and remote connections right now, rejections in total.
#[derive(Debug, Default)]
pub struct PipeStats {
    pub loc_cnt: AtomicI64,
    pub rem_cnt: AtomicI64,
    pub reject_cnt: AtomicI64,
}

/// Called once when the join is torn down; the argument is the error, if there was one.
pub type OnDestroy = Arc<dyn Fn(Option<&io::Error>) + Send + Sync>;

/// `connPiper` options; JS `isServer` and `compress` are ignored and not ported.
#[derive(Clone, Default)]
pub struct PiperOpts {
    /// Without a logger the join is silent.
    pub logger: Option<Logger>,
    pub on_destroy: Option<OnDestroy>,
}

#[cfg(test)]
#[path = "stats_tests.rs"]
mod tests;
