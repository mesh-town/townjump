// JS: holesail-logger/index.js (log lines), holesail/src/bin/holesail.mjs (unhandled exception, exit code 1).
// `Logger` is passed explicitly; `Fatal` is the channel for failures that crash the process in JS.
pub mod fatal;
pub mod logger;

pub use fatal::Fatal;
pub use logger::{Logger, SharedOut};
