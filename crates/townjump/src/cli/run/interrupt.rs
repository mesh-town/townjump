// JS: goodbye-graceful-shutdown (SIGINT/SIGTERM) in holesail.mjs, the `--filemanager` branch.
// See also: filemanager.rs waits for the signal; `CliIo.interrupt` sets `os_interrupt`, tests — `manual_interrupt`.
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// Exit code on a signal (graceful-goodbye: `exitCode = 130`).
pub(super) const SIGNAL_EXIT: i32 = 130;

/// Subscription to stop signals: called once, after the banner; each signal is a `()` in the channel.
pub type Interrupt = Box<dyn FnOnce() -> UnboundedReceiver<()> + Send>;

/// Real signals: unix — SIGINT and SIGTERM, Windows — Ctrl+C. Registration is lazy, at call time.
pub fn os_interrupt() -> Interrupt {
    Box::new(listen)
}

/// For tests: the sender raises the signal (it must be kept alive while `run` works).
pub fn manual_interrupt() -> (Interrupt, UnboundedSender<()>) {
    let (tx, rx) = unbounded_channel();
    (Box::new(move || rx), tx)
}

/// Waits for a signal. A closed channel (the sender died) is not a signal: wait forever.
pub(super) async fn signalled(rx: &mut UnboundedReceiver<()>) {
    if rx.recv().await.is_none() {
        std::future::pending::<()>().await;
    }
}

#[cfg(unix)]
fn listen() -> UnboundedReceiver<()> {
    use tokio::signal::unix::{SignalKind, signal};
    let (tx, rx) = unbounded_channel();
    let int = signal(SignalKind::interrupt());
    let term = signal(SignalKind::terminate());
    tokio::spawn(async move {
        let (Ok(mut int), Ok(mut term)) = (int, term) else {
            return;
        };
        loop {
            tokio::select! { _ = int.recv() => {}, _ = term.recv() => {} }
            if tx.send(()).is_err() {
                return;
            }
        }
    });
    rx
}

#[cfg(windows)]
fn listen() -> UnboundedReceiver<()> {
    let (tx, rx) = unbounded_channel();
    let ctrl_c = tokio::signal::windows::ctrl_c();
    tokio::spawn(async move {
        let Ok(mut ctrl_c) = ctrl_c else {
            return;
        };
        while ctrl_c.recv().await.is_some() {
            if tx.send(()).is_err() {
                return;
            }
        }
    });
    rx
}

#[cfg(not(any(unix, windows)))]
fn listen() -> UnboundedReceiver<()> {
    unbounded_channel().1
}
