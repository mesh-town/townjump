use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::{Instant, sleep, sleep_until};

use super::{Attempt, Feed, Raced, first_success, first_success_fed};
use crate::hyperdht::HyperDhtError;

#[derive(Clone)]
enum Outcome {
    Win,
    Swallowed,
    Terminal,
}

#[derive(Clone)]
struct Spec {
    id: u32,
    after_ms: u64,
    outcome: Outcome,
}

fn spec(id: u32, after_ms: u64, outcome: Outcome) -> Spec {
    Spec {
        id,
        after_ms,
        outcome,
    }
}

struct Peak {
    now: AtomicUsize,
    max: AtomicUsize,
}

struct Running<'a>(&'a Peak);

impl<'a> Running<'a> {
    fn enter(peak: &'a Peak) -> Self {
        let now = peak.now.fetch_add(1, SeqCst) + 1;
        peak.max.fetch_max(now, SeqCst);
        Running(peak)
    }
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.0.now.fetch_sub(1, SeqCst);
    }
}

fn attempt<'a>(peak: &'a Peak) -> impl Fn(Spec) -> Attempt<'a, u32> {
    move |s| {
        Box::pin(async move {
            let _running = Running::enter(peak);
            sleep(Duration::from_millis(s.after_ms)).await;
            match s.outcome {
                Outcome::Win => Ok(s.id),
                Outcome::Swallowed => Err(HyperDhtError::HandshakeFailed("silent".into())),
                Outcome::Terminal => Err(HyperDhtError::FirewallRejected),
            }
        })
    }
}

fn peak() -> Peak {
    Peak {
        now: AtomicUsize::new(0),
        max: AtomicUsize::new(0),
    }
}

async fn race(first: Vec<Spec>, limit: usize, peak: &Peak) -> (Raced<u32>, Duration) {
    let started = Instant::now();
    let raced = first_success(
        first,
        std::future::ready(Vec::new()),
        limit,
        attempt(peak),
        HyperDhtError::NoRelayNodes,
    )
    .await;
    (raced, started.elapsed())
}

#[tokio::test(start_paused = true)]
async fn five_silent_candidates_take_three_waves_of_two() {
    let p = peak();
    let silent = (0..5).map(|i| spec(i, 4000, Outcome::Swallowed)).collect();
    let (raced, elapsed) = race(silent, 2, &p).await;
    assert!(matches!(
        raced,
        Raced::Exhausted(HyperDhtError::HandshakeFailed(_))
    ));
    assert_eq!(elapsed, Duration::from_secs(12));
    assert_eq!(p.max.load(SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn hanging_first_does_not_delay_a_quick_second() {
    let p = peak();
    let both = vec![spec(0, 4000, Outcome::Swallowed), spec(1, 10, Outcome::Win)];
    let (raced, elapsed) = race(both, 2, &p).await;
    assert!(matches!(raced, Raced::Won(1)));
    assert_eq!(elapsed, Duration::from_millis(10));
    assert_eq!(p.now.load(SeqCst), 0, "the hanging attempt is dropped");
}

#[tokio::test(start_paused = true)]
async fn two_hanging_then_third_waits_one_timeout() {
    let p = peak();
    let three = vec![
        spec(0, 4000, Outcome::Swallowed),
        spec(1, 4000, Outcome::Swallowed),
        spec(2, 5, Outcome::Win),
    ];
    let (raced, elapsed) = race(three, 2, &p).await;
    assert!(matches!(raced, Raced::Won(2)));
    assert_eq!(elapsed, Duration::from_millis(4005));
}

#[tokio::test(start_paused = true)]
async fn terminal_failure_ends_the_race_at_once() {
    let p = peak();
    let both = vec![spec(0, 100, Outcome::Terminal), spec(1, 5000, Outcome::Win)];
    let (raced, elapsed) = race(both, 2, &p).await;
    assert!(matches!(
        raced,
        Raced::Stopped(HyperDhtError::FirewallRejected)
    ));
    assert_eq!(elapsed, Duration::from_millis(100));
}

#[tokio::test(start_paused = true)]
async fn later_candidates_join_while_the_first_still_hang() {
    let p = peak();
    let later = async {
        sleep(Duration::from_millis(50)).await;
        vec![spec(7, 10, Outcome::Win)]
    };
    let started = Instant::now();
    let raced = first_success(
        vec![spec(0, 4000, Outcome::Swallowed)],
        later,
        3,
        attempt(&p),
        HyperDhtError::NoRelayNodes,
    )
    .await;
    assert!(matches!(raced, Raced::Won(7)));
    assert_eq!(started.elapsed(), Duration::from_millis(60));
}

#[tokio::test(start_paused = true)]
async fn no_candidates_at_all_is_exhausted_with_the_given_error() {
    let p = peak();
    let (raced, _) = race(Vec::new(), 2, &p).await;
    assert!(matches!(
        raced,
        Raced::Exhausted(HyperDhtError::NoRelayNodes)
    ));
}

// ── a stream of later candidates (`first_success_fed`) ──────────────────────────────────

/// Start log: `(id, time since the race began)`.
#[derive(Clone)]
struct Starts {
    began: Instant,
    log: Arc<Mutex<Vec<(u32, Duration)>>>,
}

impl Starts {
    fn new() -> Self {
        Starts {
            began: Instant::now(),
            log: Arc::default(),
        }
    }

    fn attempt<'a>(&'a self) -> impl Fn(Spec) -> Attempt<'a, u32> {
        move |s| {
            self.log.lock().unwrap().push((s.id, self.began.elapsed()));
            Box::pin(async move {
                sleep(Duration::from_millis(s.after_ms)).await;
                match s.outcome {
                    Outcome::Win => Ok(s.id),
                    Outcome::Swallowed => Err(HyperDhtError::HandshakeFailed("silent".into())),
                    Outcome::Terminal => Err(HyperDhtError::FirewallRejected),
                }
            })
        }
    }

    fn started_at(&self, id: u32) -> Duration {
        let log = self.log.lock().unwrap();
        log.iter().find(|(i, _)| *i == id).expect("started").1
    }
}

struct ChannelFeed(mpsc::UnboundedReceiver<Vec<Spec>>);

impl Feed<Spec> for ChannelFeed {
    fn poll_more(&mut self, cx: &mut Context<'_>) -> Poll<Option<Vec<Spec>>> {
        self.0.poll_recv(cx)
    }
}

/// Batches arrive at the given milliseconds, the feed ends at `end_ms`.
fn feed(schedule: Vec<(u64, Vec<Spec>)>, end_ms: u64) -> ChannelFeed {
    let (tx, rx) = mpsc::unbounded_channel();
    let t0 = Instant::now();
    tokio::spawn(async move {
        for (at, batch) in schedule {
            sleep_until(t0 + Duration::from_millis(at)).await;
            let _ = tx.send(batch);
        }
        sleep_until(t0 + Duration::from_millis(end_ms)).await;
        drop(tx);
    });
    ChannelFeed(rx)
}

/// Model of the naive behaviour: collects every batch and hands them out at the end.
struct Gathered<F> {
    inner: F,
    buf: Vec<Spec>,
}

impl<F: Feed<Spec>> Feed<Spec> for Gathered<F> {
    fn poll_more(&mut self, cx: &mut Context<'_>) -> Poll<Option<Vec<Spec>>> {
        loop {
            match self.inner.poll_more(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(batch)) => self.buf.extend(batch),
                Poll::Ready(None) if self.buf.is_empty() => return Poll::Ready(None),
                Poll::Ready(None) => return Poll::Ready(Some(std::mem::take(&mut self.buf))),
            }
        }
    }
}

async fn race_fed(
    later: &mut impl Feed<Spec>,
    limit: usize,
    starts: &Starts,
) -> (Raced<u32>, Duration) {
    let raced = first_success_fed(
        Vec::new(),
        later,
        limit,
        starts.attempt(),
        HyperDhtError::NoRelayNodes,
    )
    .await;
    (raced, starts.began.elapsed())
}

mod a_fed_candidate {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_fed_candidate_starts_before_the_feed_ends() {
        let starts = Starts::new();
        let mut f = feed(vec![(10, vec![spec(1, 20, Outcome::Win)])], 1000);
        let (raced, elapsed) = race_fed(&mut f, 2, &starts).await;
        assert!(matches!(raced, Raced::Won(1)));
        assert_eq!(starts.started_at(1), Duration::from_millis(10));
        assert_eq!(elapsed, Duration::from_millis(30));
    }

    #[tokio::test(start_paused = true)]
    async fn the_gathering_model_starts_only_after_the_feed_ends() {
        let starts = Starts::new();
        let mut f = Gathered {
            inner: feed(vec![(10, vec![spec(1, 20, Outcome::Win)])], 1000),
            buf: Vec::new(),
        };
        let (raced, _) = race_fed(&mut f, 2, &starts).await;
        assert!(matches!(raced, Raced::Won(1)));
        assert_eq!(starts.started_at(1), Duration::from_millis(1000));
    }
}

mod a_failure_with_the_feed_open {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn after_a_failure_the_next_fed_candidate_starts_on_arrival() {
        let starts = Starts::new();
        let mut f = feed(
            vec![
                (10, vec![spec(1, 20, Outcome::Swallowed)]),
                (40, vec![spec(2, 10, Outcome::Win)]),
            ],
            1000,
        );
        let (raced, elapsed) = race_fed(&mut f, 1, &starts).await;
        assert!(matches!(raced, Raced::Won(2)));
        assert_eq!(starts.started_at(2), Duration::from_millis(40));
        assert_eq!(elapsed, Duration::from_millis(50));
    }

    #[tokio::test(start_paused = true)]
    async fn gathered_control_delays_the_next_candidate_after_a_failure() {
        let starts = Starts::new();
        let mut f = Gathered {
            inner: feed(
                vec![
                    (10, vec![spec(1, 20, Outcome::Swallowed)]),
                    (40, vec![spec(2, 10, Outcome::Win)]),
                ],
                1000,
            ),
            buf: Vec::new(),
        };
        let (raced, _) = race_fed(&mut f, 1, &starts).await;
        assert!(matches!(raced, Raced::Won(2)));
        assert_eq!(starts.started_at(2), Duration::from_millis(1020));
    }

    #[tokio::test(start_paused = true)]
    async fn a_terminal_failure_stops_while_the_feed_is_open() {
        let starts = Starts::new();
        let mut f = feed(vec![(10, vec![spec(1, 5, Outcome::Terminal)])], 1000);
        let (raced, elapsed) = race_fed(&mut f, 2, &starts).await;
        assert!(matches!(
            raced,
            Raced::Stopped(HyperDhtError::FirewallRejected)
        ));
        assert_eq!(elapsed, Duration::from_millis(15));
    }
}

mod the_end_of_the_feed {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn an_ended_feed_without_winners_is_exhausted_at_its_end() {
        let starts = Starts::new();
        let mut f = feed(vec![(10, vec![spec(1, 5, Outcome::Swallowed)])], 200);
        let (raced, elapsed) = race_fed(&mut f, 2, &starts).await;
        assert!(matches!(
            raced,
            Raced::Exhausted(HyperDhtError::HandshakeFailed(_))
        ));
        assert_eq!(elapsed, Duration::from_millis(200));
    }
}
