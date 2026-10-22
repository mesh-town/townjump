// JS: hyperdht/lib/connect.js:findAndConnect (Semaphore(2), first `c.connect` wins)
//! Runs handshake attempts at most `limit` at a time; the first success wins and the attempts
//! still running are dropped (JS destroys their requests: here the session of the connect cancels
//! their network requests). Neighbours: `connect_flow.rs` (what an attempt is), `connect_error.rs`
//! (which failures end the race at once). `first_success_fed` takes the later candidates from a
//! `Feed` (a stream of batches), `first_success` is the one-batch wrapper over it.

use std::collections::VecDeque;
use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::task::{Context, Poll};

use super::HyperDhtError;
use super::connect_error::{is_terminal, keep_terminal};

pub(super) type Attempt<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, HyperDhtError>> + Send + 'a>>;

pub(super) enum Raced<T> {
    Won(T),
    /// A failure JS does not swallow; the rest of the candidates is not tried.
    Stopped(HyperDhtError),
    /// Every candidate failed; the remembered error (`connect_error::keep_terminal`).
    Exhausted(HyperDhtError),
}

struct Lanes<'a, C, T> {
    limit: usize,
    queue: VecDeque<C>,
    running: Vec<Attempt<'a, T>>,
    last_err: HyperDhtError,
}

impl<'a, C, T> Lanes<'a, C, T> {
    fn fill(&mut self, attempt: &impl Fn(C) -> Attempt<'a, T>) {
        while self.running.len() < self.limit {
            let Some(candidate) = self.queue.pop_front() else {
                return;
            };
            self.running.push(attempt(candidate));
        }
    }

    fn idle(&self) -> bool {
        self.running.is_empty() && self.queue.is_empty()
    }

    fn settle(&mut self, index: usize, done: Result<T, HyperDhtError>) -> Option<Raced<T>> {
        drop(self.running.swap_remove(index));
        let err = match done {
            Ok(won) => return Some(Raced::Won(won)),
            Err(e) if is_terminal(&e) => return Some(Raced::Stopped(e)),
            Err(e) => e,
        };
        let current = std::mem::replace(&mut self.last_err, HyperDhtError::NoRelayNodes);
        self.last_err = keep_terminal(current, err);
        None
    }
}

async fn any_done<T>(running: &mut [Attempt<'_, T>]) -> (usize, Result<T, HyperDhtError>) {
    poll_fn(|cx| {
        for (index, lane) in running.iter_mut().enumerate() {
            if let Poll::Ready(done) = lane.as_mut().poll(cx) {
                return Poll::Ready((index, done));
            }
        }
        Poll::Pending
    })
    .await
}

/// Source of candidates that arrive while the race runs (the FIND_PEER holders, one by one).
pub(super) trait Feed<C> {
    /// `Some(batch)` (may be empty) or the end (`None`, then always `None`); cancel-safe.
    fn poll_more(&mut self, cx: &mut Context<'_>) -> Poll<Option<Vec<C>>>;
}

/// One batch from a future, then the end.
struct OnceFeed<'f, F> {
    later: Pin<&'f mut F>,
    done: bool,
}

impl<C, F: Future<Output = Vec<C>>> Feed<C> for OnceFeed<'_, F> {
    fn poll_more(&mut self, cx: &mut Context<'_>) -> Poll<Option<Vec<C>>> {
        if self.done {
            return Poll::Ready(None);
        }
        let batch = std::task::ready!(self.later.as_mut().poll(cx));
        self.done = true;
        Poll::Ready(Some(batch))
    }
}

/// `first` candidates start at once (up to `limit`); `later` resolves to more candidates (the
/// FIND_PEER holders) that join the queue while the first ones are already running.
pub(super) async fn first_success<'a, C, T>(
    first: Vec<C>,
    later: impl Future<Output = Vec<C>>,
    limit: usize,
    attempt: impl Fn(C) -> Attempt<'a, T>,
    last_err: HyperDhtError,
) -> Raced<T> {
    let later = std::pin::pin!(later);
    let mut feed = OnceFeed { later, done: false };
    first_success_fed(first, &mut feed, limit, attempt, last_err).await
}

/// Like `first_success`, but the later candidates come as a stream of batches: each one joins the
/// queue on arrival (JS `for await (const data of c.query)`), `Exhausted` only after the feed ended.
pub(super) async fn first_success_fed<'a, C, T>(
    first: Vec<C>,
    later: &mut impl Feed<C>,
    limit: usize,
    attempt: impl Fn(C) -> Attempt<'a, T>,
    last_err: HyperDhtError,
) -> Raced<T> {
    let mut lanes = Lanes {
        limit,
        queue: first.into(),
        running: Vec::new(),
        last_err,
    };
    let mut open = true;
    loop {
        lanes.fill(&attempt);
        if lanes.idle() && !open {
            return Raced::Exhausted(lanes.last_err);
        }
        tokio::select! {
            more = poll_fn(|cx| later.poll_more(cx)), if open => match more {
                Some(batch) => lanes.queue.extend(batch),
                None => open = false,
            },
            (index, done) = any_done(&mut lanes.running), if !lanes.running.is_empty() => {
                if let Some(raced) = lanes.settle(index, done) {
                    return raced;
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "connect_race_tests.rs"]
mod tests;
