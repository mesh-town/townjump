//! Keepalive (empty frame) and idle-timeout timers of `SecretStream`, served from the read path.
//!
//! The timers are polled by `poll_read`/`read()` only: a stream nobody reads from sends no keepalive.
//! `into_split` halves get no timers.
// JS: @hyperswarm/secret-stream/index.js

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::{Instant, Sleep};

use super::frame::encode_frame;
use super::session::SecretStream;

/// Keepalive period of a new stream: hyperdht's `connectionKeepAlive` (5000 ms) for every connection.
pub const DEFAULT_KEEP_ALIVE: Duration = Duration::from_secs(5);

/// One timer that restarts its full period on `refresh` (JS `Timeout.refresh()`).
struct Ticker {
    period: Duration,
    sleep: Pin<Box<Sleep>>,
}

impl Ticker {
    /// Zero switches the timer off (JS `setKeepAlive(0)`, `setTimeout(0)`).
    fn start(period: Duration) -> Option<Self> {
        (!period.is_zero()).then(|| Self {
            period,
            sleep: Box::pin(tokio::time::sleep(period)),
        })
    }

    fn refresh(&mut self) {
        self.sleep.as_mut().reset(Instant::now() + self.period);
    }

    fn is_expired(&mut self, cx: &mut Context<'_>) -> bool {
        self.sleep.as_mut().poll(cx).is_ready()
    }

    /// Periodic timer (JS `Timeout.on`): an expiry starts the next period.
    fn poll_tick(&mut self, cx: &mut Context<'_>) -> bool {
        if !self.is_expired(cx) {
            return false;
        }
        self.refresh();
        // The fired timer has dropped our waker: register it for the new deadline.
        let _ = self.is_expired(cx);
        true
    }
}

/// The two timers of one stream; `None` is off.
pub(super) struct Timers {
    keep_alive: Option<Ticker>,
    timeout: Option<Ticker>,
}

/// Whether an empty keepalive frame has to go out now.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum KeepAliveDue {
    Yes,
    No,
}

impl From<bool> for KeepAliveDue {
    fn from(due: bool) -> Self {
        match due {
            true => Self::Yes,
            false => Self::No,
        }
    }
}

/// The idle timeout expired.
pub(super) struct TimedOut;

impl Timers {
    /// Keepalive [`DEFAULT_KEEP_ALIVE`], no timeout (JS: timeout 0).
    pub(super) fn new() -> Self {
        Self {
            keep_alive: Ticker::start(DEFAULT_KEEP_ALIVE),
            timeout: None,
        }
    }

    pub(super) fn set_keep_alive(&mut self, period: Duration) {
        self.keep_alive = Ticker::start(period);
    }

    pub(super) fn set_timeout(&mut self, period: Duration) {
        self.timeout = Ticker::start(period);
    }

    /// Nothing may follow the END of our side: no more keepalives.
    pub(super) fn stop_keep_alive(&mut self) {
        self.keep_alive = None;
    }

    /// Something left our side (JS `_write`: `_keepAliveTimer.refresh()`).
    pub(super) fn tx_sent(&mut self) {
        if let Some(ticker) = &mut self.keep_alive {
            ticker.refresh();
        }
    }

    /// Raw bytes arrived (JS `_onrawdata`: `_timeoutTimer.refresh()`).
    pub(super) fn rx_seen(&mut self) {
        if let Some(ticker) = &mut self.timeout {
            ticker.refresh();
        }
    }

    pub(super) fn poll(&mut self, cx: &mut Context<'_>) -> Result<KeepAliveDue, TimedOut> {
        let timed_out = self.timeout.as_mut().is_some_and(|t| t.is_expired(cx));
        if timed_out {
            return Err(TimedOut);
        }
        let due = self.keep_alive.as_mut().is_some_and(|t| t.poll_tick(cx));
        Ok(due.into())
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> SecretStream<T> {
    /// Send an empty message after `period` without outgoing traffic; zero switches keepalive off.
    ///
    /// New streams start with [`DEFAULT_KEEP_ALIVE`]. Call from within a Tokio runtime.
    pub fn set_keep_alive(&mut self, period: Duration) {
        self.timers.set_keep_alive(period);
    }

    /// Fail reads with `TimedOut` after `period` without incoming bytes; zero (the default)
    /// switches the timeout off. Call from within a Tokio runtime.
    pub fn set_timeout(&mut self, period: Duration) {
        self.timers.set_timeout(period);
    }

    /// Next frame, timers first: expired timeout → `TimedOut`, keepalive due → sent.
    pub(super) fn poll_frame_timed(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<Option<Vec<u8>>>> {
        self.poll_timers(cx)?;
        self.poll_frame(cx)
    }

    fn poll_timers(&mut self, cx: &mut Context<'_>) -> io::Result<()> {
        let due = self
            .timers
            .poll(cx)
            .map_err(|TimedOut| io::Error::new(io::ErrorKind::TimedOut, "Stream timed out"))?;
        match due {
            KeepAliveDue::Yes => self.send_keepalive(cx),
            KeepAliveDue::No => Ok(()),
        }
    }

    /// Best effort, like a write nobody waits for: what the transport does not take stays in `tx`.
    fn send_keepalive(&mut self, cx: &mut Context<'_>) -> io::Result<()> {
        // A frame `poll_write` has not finished is traffic already, and no second one fits in `tx`.
        let Poll::Ready(Ok(())) = self.poll_drain_tx(cx) else {
            return Ok(());
        };
        let frame = encode_frame(&self.encrypt.next(&[])).map_err(io::Error::other)?;
        self.io.set_tx(frame);
        let _ = self.poll_drain_tx(cx);
        let _ = Pin::new(&mut self.raw).poll_flush(cx);
        Ok(())
    }
}
