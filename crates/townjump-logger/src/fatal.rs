use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// Sender half of the channel background tasks use to ask for process exit.
#[derive(Clone)]
pub struct Fatal(UnboundedSender<String>);

impl Fatal {
    pub fn channel() -> (Fatal, UnboundedReceiver<String>) {
        let (tx, rx) = unbounded_channel();
        (Fatal(tx), rx)
    }

    /// Delivers `msg` unchanged. A vanished receiver is not an error.
    pub fn raise(&self, msg: impl Into<String>) {
        let _ = self.0.send(msg.into());
    }
}

#[cfg(test)]
#[path = "fatal_tests.rs"]
mod tests;
