//! `settle_within`: parallel jobs under one deadline (virtual time).

use std::future::{Future, pending};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::time::{Instant, sleep};

use super::settle_within;

type Job = Pin<Box<dyn Future<Output = bool> + Send>>;

const DEADLINE: Duration = Duration::from_millis(1500);

fn job(f: impl Future<Output = bool> + Send + 'static) -> Job {
    Box::pin(f)
}

/// Sets the flag when dropped.
struct Guard(Arc<AtomicBool>);

impl Drop for Guard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

mod settle {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn runs_the_jobs_at_once() {
        // given
        let slow = || {
            job(async {
                sleep(Duration::from_millis(200)).await;
                true
            })
        };
        let jobs = vec![slow(), slow(), slow()];
        let start = Instant::now();
        // when
        let n = settle_within(DEADLINE, jobs).await;
        // then
        assert_eq!(n, 3);
        assert!(
            start.elapsed() < Duration::from_millis(300),
            "{:?}",
            start.elapsed()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn counts_only_confirmed_jobs() {
        // given
        let jobs = vec![
            job(async { true }),
            job(async { false }),
            job(async { true }),
        ];
        // when
        let n = settle_within(DEADLINE, jobs).await;
        // then
        assert_eq!(n, 2);
    }

    #[tokio::test(start_paused = true)]
    async fn stops_at_the_deadline() {
        // given
        let jobs = vec![job(pending()), job(async { true })];
        let start = Instant::now();
        // when
        let n = settle_within(DEADLINE, jobs).await;
        // then
        assert_eq!(n, 1);
        assert_eq!(start.elapsed(), DEADLINE);
    }

    #[tokio::test(start_paused = true)]
    async fn drops_the_jobs_cut_by_the_deadline() {
        // given
        let dropped = Arc::new(AtomicBool::new(false));
        let guard = Guard(dropped.clone());
        let jobs = vec![job(async move {
            let _guard = guard;
            pending::<bool>().await
        })];
        // when
        settle_within(DEADLINE, jobs).await;
        // then
        assert!(
            dropped.load(Ordering::SeqCst),
            "the cut job was not dropped"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn returns_at_once_without_jobs() {
        // given
        let start = Instant::now();
        // when
        let n = settle_within::<Job>(DEADLINE, vec![]).await;
        // then
        assert_eq!(n, 0);
        assert_eq!(start.elapsed(), Duration::ZERO);
    }
}
