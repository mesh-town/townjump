//! Pacing token-bucket tests: time is set by hand (`Instant` + `Duration`), no sleeping.

use super::*;

const MS: Duration = Duration::from_millis(1);

#[test]
fn starts_with_initial_allowance() {
    let t = Instant::now();
    let mut bucket = TokenBucket::new();
    assert!(bucket.try_consume(25_000, t));
    assert!(!bucket.try_consume(1, t));
}

/// Every whole millisecond REPLACES the remainder with `elapsed_ms * rate`; the fractional tail stays in the time.
#[test]
fn refill_replaces_by_whole_milliseconds() {
    let t0 = Instant::now();
    let mut bucket = TokenBucket::new();
    bucket.set_rate(1000);

    // the first refill sets `last_refill = t0`
    assert!(bucket.try_consume(25_000, t0));
    assert_eq!(bucket.available, 0);

    bucket.refill(t0 + Duration::from_micros(900));
    assert_eq!(bucket.available, 0);

    bucket.refill(t0 + MS);
    assert_eq!(bucket.available, 1000);

    assert!(bucket.try_consume(400, t0 + MS));
    assert_eq!(bucket.available, 600);

    // 1.5 ms since the last refill: one whole millisecond, replacement (not 600 + 1000)
    bucket.refill(t0 + Duration::from_micros(2500));
    assert_eq!(bucket.available, 1000);
    assert_eq!(bucket.last_refill, Some(t0 + 2 * MS));

    assert!(bucket.try_consume(1000, t0 + Duration::from_micros(2500)));
    assert_eq!(bucket.available, 0);

    // exactly 1 ms has passed since t0 + 2 ms (with `last_refill = t0 + 2.5 ms` it would be 0)
    bucket.refill(t0 + 3 * MS);
    assert_eq!(bucket.available, 1000);
}

#[test]
fn delay_is_rest_of_millisecond() {
    let t0 = Instant::now();
    let mut bucket = TokenBucket::new();
    bucket.set_rate(1000);
    assert!(bucket.try_consume(25_000, t0));
    assert_eq!(
        bucket.delay_for(t0 + Duration::from_micros(300)),
        Duration::from_micros(700)
    );
}

#[test]
fn no_rate_never_delays() {
    let t = Instant::now();
    let mut bucket = TokenBucket::new();
    assert!(bucket.try_consume(25_000, t));
    assert_eq!(bucket.available, 0);
    assert_eq!(bucket.delay_for(t), Duration::ZERO);
}

#[test]
fn reset_restores_initial_allowance() {
    let t = Instant::now();
    let mut bucket = TokenBucket::new();
    bucket.set_rate(1000);
    assert!(bucket.try_consume(25_000, t));
    assert_eq!(bucket.available, 0);

    bucket.reset(t + MS);
    assert_eq!(bucket.available, 25_000);
    assert_eq!(bucket.last_refill, Some(t + MS));
}
