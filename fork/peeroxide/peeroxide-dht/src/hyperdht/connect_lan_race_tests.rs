//! Tests for the LAN-versus-holepunch race (`first_some`, `lan_or_punch`) in `connect_lan`.

use std::future::{Future, pending};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::time::{Instant, sleep};

use super::super::{first_some, lan_or_punch};
use super::{assert_near, ms};
use crate::hyperdht::HyperDhtError;

mod first_some_fn {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn first_answer_wins_and_the_other_is_dropped() {
        let dropped = Arc::new(AtomicBool::new(false));
        let got = first_some(
            after(ms(100), Some("a")),
            hang_with_guard::<Option<&str>>(&dropped),
        )
        .await;
        assert_eq!(got, Some("a"));
        assert!(dropped.load(Ordering::SeqCst), "the loser was not dropped");
    }

    #[tokio::test(start_paused = true)]
    async fn none_waits_for_the_other() {
        let start = Instant::now();
        let got = first_some(async { None::<&str> }, after(ms(1000), Some("b"))).await;
        assert_eq!(got, Some("b"));
        assert_near(start.elapsed(), ms(1000));
    }

    #[tokio::test(start_paused = true)]
    async fn both_none_give_none() {
        let got = first_some(after(ms(100), None::<&str>), async { None }).await;
        assert_eq!(got, None);
    }
}

mod lan_or_punch_fn {
    use super::*;

    type Punch = Result<&'static str, HyperDhtError>;

    #[tokio::test(start_paused = true)]
    async fn lan_first_wins_and_the_punch_is_dropped() {
        let dropped = Arc::new(AtomicBool::new(false));
        let got = lan_or_punch(
            after(ms(100), Some("lan")),
            hang_with_guard::<Punch>(&dropped),
        )
        .await;
        assert!(matches!(got, Ok("lan")), "got {got:?}");
        assert!(dropped.load(Ordering::SeqCst), "the punch was not dropped");
    }

    #[tokio::test(start_paused = true)]
    async fn punch_first_wins_and_the_lan_is_dropped() {
        let dropped = Arc::new(AtomicBool::new(false));
        let lan = hang_with_guard::<Option<&'static str>>(&dropped);
        let got = lan_or_punch(lan, after(ms(100), Ok("punch"))).await;
        assert!(matches!(got, Ok("punch")), "got {got:?}");
        assert!(
            dropped.load(Ordering::SeqCst),
            "the LAN path was not dropped"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn silent_lan_leaves_the_punch_to_finish() {
        let start = Instant::now();
        let lan = after(ms(2000), None);
        let got = lan_or_punch(lan, after(ms(5000), Ok("punch"))).await;
        assert!(matches!(got, Ok("punch")), "got {got:?}");
        assert_near(start.elapsed(), ms(5000));
    }

    #[tokio::test(start_paused = true)]
    async fn failed_punch_leaves_the_lan_to_finish() {
        let start = Instant::now();
        let punch = async { Err::<&str, _>(HyperDhtError::HolepunchFailed) };
        let got = lan_or_punch(after(ms(1500), Some("lan")), punch).await;
        assert!(matches!(got, Ok("lan")), "got {got:?}");
        assert_near(start.elapsed(), ms(1500));
    }

    #[tokio::test(start_paused = true)]
    async fn both_failing_return_the_punch_error() {
        let lan = || async { None::<&str> };
        // when
        let aborted = lan_or_punch(lan(), async {
            Err::<&str, _>(HyperDhtError::HolepunchAborted)
        })
        .await;
        let failed = lan_or_punch(lan(), async {
            Err::<&str, _>(HyperDhtError::HolepunchFailed)
        })
        .await;
        // then
        assert!(
            matches!(aborted, Err(HyperDhtError::HolepunchAborted)),
            "got {aborted:?}"
        );
        assert!(
            matches!(failed, Err(HyperDhtError::HolepunchFailed)),
            "got {failed:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn both_ready_at_once_lan_wins() {
        let got = lan_or_punch(after(ms(100), Some("lan")), after(ms(100), Ok("punch"))).await;
        assert!(matches!(got, Ok("lan")), "got {got:?}");
    }
}

// ---- setup and helpers ----

/// Resolves to `value` after `delay`.
async fn after<T>(delay: Duration, value: T) -> T {
    sleep(delay).await;
    value
}

/// Raises its flag when dropped.
struct DropFlag(Arc<AtomicBool>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// A future that never resolves and raises `flag` once it is dropped (even if it was never polled).
fn hang_with_guard<R>(flag: &Arc<AtomicBool>) -> impl Future<Output = R> {
    let guard = DropFlag(Arc::clone(flag));
    async move {
        let _guard = guard;
        pending().await
    }
}
