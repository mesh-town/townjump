// JS: testvectors/holesail-cli/run-matrix.mjs (waits for output silence, a time ceiling, then kills the process).
// Related: capture.rs (`last_write`). Synchronous: builds its own runtime, call from a plain `#[test]`.
use crate::capture::Capture;
use std::future::Future;
use std::pin::{Pin, pin};
use std::time::{Duration, Instant};
use tokio::runtime::{Builder, Runtime};

/// Silence after which a case counts as "works": 15 × 20 ms with a margin for a loaded `cargo test`.
/// `run-matrix.mjs` waited 1200 ms, but that included the node start; in a Rust process lines come in a few ms.
pub const QUIET: Duration = Duration::from_millis(300);
/// Polling: how often we look at the output and at the completion of `run`.
const TICK: Duration = Duration::from_millis(10);
/// Ceiling of one case, as in `run-matrix.mjs`.
const MAX_RUN: Duration = Duration::from_secs(8);
/// How long we wait for the tasks spawned by `run` to close sockets after the case ends.
const SHUTDOWN_GRACE: Duration = Duration::from_millis(200);
/// How long we wait for `run` to end after the signal.
const AFTER_SIGNAL: Duration = Duration::from_millis(2500);

/// Drives `run` to the end, to silence in all `captures` for `quiet`, or to the ceiling. `None` means `run` is still running.
/// Tasks spawned by `run` (client proxy, server accept) and their sockets are closed, as when a process is killed.
pub fn run_until_quiet<Fut: Future<Output = i32>>(
    run: Fut,
    captures: &[&Capture],
    quiet: Duration,
) -> Option<i32> {
    run_capped(run, captures, quiet, MAX_RUN)
}

fn run_capped<Fut: Future<Output = i32>>(
    run: Fut,
    captures: &[&Capture],
    quiet: Duration,
    max: Duration,
) -> Option<i32> {
    let runtime = runtime();
    let outcome = runtime.block_on(wait(run, captures, quiet, max));
    runtime.shutdown_timeout(SHUTDOWN_GRACE);
    outcome
}

fn runtime() -> Runtime {
    Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runtime for run_until_quiet")
}

/// Like `run_until_quiet`, but on silence sends the signal (`interrupt`) and waits up to 2500 ms for `run` to end.
/// The first value is the code before the signal (`Some` means the signal was not sent), the second is the code after it (`None` means it did not exit).
pub fn run_until_quiet_then<Fut: Future<Output = i32>>(
    run: Fut,
    captures: &[&Capture],
    quiet: Duration,
    interrupt: impl FnOnce(),
) -> (Option<i32>, Option<i32>) {
    let runtime = runtime();
    let outcome = runtime.block_on(async {
        let mut run = pin!(run);
        let before = wait_pinned(&mut run, captures, quiet, MAX_RUN).await;
        if before.is_some() {
            return (before, None);
        }
        interrupt();
        (
            None,
            tokio::time::timeout(AFTER_SIGNAL, &mut run).await.ok(),
        )
    });
    runtime.shutdown_timeout(SHUTDOWN_GRACE);
    outcome
}

async fn wait<Fut: Future<Output = i32>>(
    run: Fut,
    captures: &[&Capture],
    quiet: Duration,
    max: Duration,
) -> Option<i32> {
    let mut run = pin!(run);
    wait_pinned(&mut run, captures, quiet, max).await
}

async fn wait_pinned<Fut: Future<Output = i32>>(
    run: &mut Pin<&mut Fut>,
    captures: &[&Capture],
    quiet: Duration,
    max: Duration,
) -> Option<i32> {
    let start = Instant::now();
    while !waited_enough(start, captures, quiet, max) {
        tokio::select! {
            code = &mut *run => return Some(code),
            () = tokio::time::sleep(TICK) => {}
        }
    }
    None
}

/// The ceiling was reached or `quiet` elapsed after the last write (or the start).
fn waited_enough(start: Instant, captures: &[&Capture], quiet: Duration, max: Duration) -> bool {
    let now = Instant::now();
    let last = captures
        .iter()
        .filter_map(|capture| capture.last_write())
        .fold(start, Instant::max);
    now - start >= max || now - last >= quiet
}

#[cfg(test)]
#[path = "quiet_tests.rs"]
mod tests;
