// JS: unhandled exception or rejected top-level promise in holesail.mjs (exit code 1, stack in stderr).
// Related: logger.rs; read by the `cli::run` channel in the holesail crate.
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

/// Sender of "fatal" texts: server and client tasks send here whatever kills the process in JS. `Clone`.
#[derive(Clone)]
pub struct Fatal(UnboundedSender<String>);

impl Fatal {
    /// "Sender, receiver" pair; the receiver is read by `cli::run`.
    pub fn channel() -> (Fatal, UnboundedReceiver<String>) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (Fatal(tx), rx)
    }

    /// Reports the error text upward (first line of the JS stack); a lost receiver is not a panic.
    pub fn raise(&self, msg: impl Into<String>) {
        let _ = self.0.send(msg.into());
    }
}

#[cfg(test)]
#[path = "fatal_tests.rs"]
mod tests;
