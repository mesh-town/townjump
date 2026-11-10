//! Leveled log with colored lines, and the channel for fatal errors.
pub mod fatal;
pub mod logger;
mod time;

pub use fatal::Fatal;
pub use logger::{Logger, SharedOut};
