//! Tests for [`crate::server::table`].

use super::*;

const MS: Duration = Duration::from_millis(1);

fn fresh(table: &mut Table<&'static str>, noise: &[u8], now: Instant) -> u64 {
    match table.begin(noise, now) {
        Begin::Fresh(id) => id,
        Begin::Repeat(_) => panic!("expected a fresh slot"),
    }
}

#[test]
fn ids_start_at_zero_and_grow() {
    let (mut table, now) = (Table::<&str>::new(), Instant::now());
    assert_eq!(table.begin(b"a", now), Begin::Fresh(0));
    assert_eq!(table.begin(b"b", now), Begin::Fresh(1));
}

#[test]
fn holepunch_repeat_same_noise_keeps_first_slot() {
    let (mut table, now) = (Table::new(), Instant::now());
    let a = fresh(&mut table, b"noise", now);
    table.attach(a, "puncher", now + INITIAL_TIMEOUT);
    table.finish(a, Some(b"reply".to_vec()));
    let repeat = table.begin(b"noise", now + 500 * MS);
    assert_eq!(repeat, Begin::Repeat(Some(b"reply".to_vec())));
    assert_eq!(table.get_mut(a), Some(&mut "puncher"));
}

#[test]
fn different_noise_same_key_gets_new_slot() {
    let (mut table, now) = (Table::<&str>::new(), Instant::now());
    let a = fresh(&mut table, b"noise-1", now);
    table.finish(a, Some(b"reply".to_vec()));
    let b = fresh(&mut table, b"noise-2", now);
    assert_ne!(a, b);
}

#[test]
fn rejected_repeat_is_not_reevaluated() {
    let (mut table, now) = (Table::<&str>::new(), Instant::now());
    let a = fresh(&mut table, b"noise", now);
    table.finish(a, None);
    assert_eq!(table.begin(b"noise", now + MS), Begin::Repeat(None));
}

#[test]
fn rejected_key_is_forgotten_clear_wait_after_begin() {
    let (mut table, now) = (Table::<&str>::new(), Instant::now());
    let a = fresh(&mut table, b"noise", now);
    table.finish(a, None);
    table.expire(now + CLEAR_WAIT - MS);
    assert_eq!(table.begin(b"noise", now), Begin::Repeat(None));
    table.expire(now + CLEAR_WAIT + MS);
    assert!(matches!(table.begin(b"noise", now), Begin::Fresh(_)));
}

#[test]
fn expire_after_clear_wait_forgets_key() {
    let (mut table, now) = (Table::<&str>::new(), Instant::now());
    let a = fresh(&mut table, b"noise", now);
    table.finish(a, Some(b"reply".to_vec()));
    table.clear_later(a, now);
    table.expire(now + Duration::from_secs(9));
    assert!(matches!(table.begin(b"noise", now), Begin::Repeat(_)));
    table.expire(now + 10_100 * MS);
    assert!(matches!(table.begin(b"noise", now), Begin::Fresh(_)));
}

#[test]
fn pending_times_out() {
    let (mut table, now) = (Table::new(), Instant::now());
    let a = fresh(&mut table, b"noise", now);
    let deadline = now + INITIAL_TIMEOUT;
    table.attach(a, "pending", deadline);
    assert!(table.expire(deadline - MS).is_empty());
    assert_eq!(table.expire(deadline + MS), vec!["pending"]);
    assert_eq!(table.get_mut(a), None);
}

#[test]
fn expired_pending_keeps_key_for_clear_wait() {
    let (mut table, now) = (Table::new(), Instant::now());
    let a = fresh(&mut table, b"noise", now);
    table.attach(a, "pending", now + MS);
    table.finish(a, Some(b"reply".to_vec()));
    let later = now + 2 * MS;
    assert_eq!(table.expire(later), vec!["pending"]);
    assert!(matches!(table.begin(b"noise", later), Begin::Repeat(_)));
    assert!(table.expire(later + CLEAR_WAIT + MS).is_empty());
    assert!(matches!(table.begin(b"noise", later), Begin::Fresh(_)));
}

#[test]
fn extend_postpones_timeout() {
    let (mut table, now) = (Table::new(), Instant::now());
    let a = fresh(&mut table, b"noise", now);
    let deadline = now + INITIAL_TIMEOUT;
    table.attach(a, "pending", deadline);
    table.extend(a, deadline + INITIAL_TIMEOUT);
    assert!(table.expire(deadline + MS).is_empty());
    assert_eq!(
        table.expire(deadline + INITIAL_TIMEOUT + MS),
        vec!["pending"]
    );
}

#[test]
fn take_keeps_key_until_clear_later() {
    let (mut table, now) = (Table::new(), Instant::now());
    let a = fresh(&mut table, b"noise", now);
    table.attach(a, "pending", now + INITIAL_TIMEOUT);
    table.finish(a, Some(b"reply".to_vec()));
    assert_eq!(table.take(a), Some("pending"));
    assert_eq!(table.take(a), None);
    assert!(table.expire(now + INITIAL_TIMEOUT + MS).is_empty());
    assert!(matches!(table.begin(b"noise", now), Begin::Repeat(_)));
}

#[test]
fn clear_all_returns_pendings() {
    let (mut table, now) = (Table::new(), Instant::now());
    let a = fresh(&mut table, b"one", now);
    let b = fresh(&mut table, b"two", now);
    table.attach(a, "first", now + INITIAL_TIMEOUT);
    table.attach(b, "second", now + INITIAL_TIMEOUT);
    assert_eq!(table.clear_all(), vec!["first", "second"]);
    assert!(matches!(table.begin(b"one", now), Begin::Fresh(_)));
}

#[tokio::test]
async fn reply_socket_lives_with_the_repeat() {
    let rt = libudx::UdxRuntime::new().expect("udx runtime");
    let socket = rt.create_socket().await.expect("socket");
    let (mut table, now) = (Table::<&str>::new(), Instant::now());
    assert!(table.via_of(b"noise").is_none(), "unknown noise has none");
    let id = fresh(&mut table, b"noise", now);
    assert!(table.via_of(b"noise").is_none(), "nothing stored yet");
    table.set_via(id, socket);
    assert!(
        table.via_of(b"noise").is_some(),
        "a repeat finds the socket"
    );
    assert!(table.via_of(b"other").is_none(), "another noise does not");
}

#[test]
fn punching_slot_survives_ticks_past_initial_timeout() {
    let t0 = Instant::now();
    let mut table = Table::new();
    let id = fresh(&mut table, b"noise", t0);
    table.attach(id, "pending", t0 + INITIAL_TIMEOUT);
    table.extend(id, t0 + PUNCHING_TIMEOUT);
    for k in 1..35 {
        assert!(
            table.expire(t0 + Duration::from_secs(k)).is_empty(),
            "tick {k}"
        );
    }
    assert_eq!(table.take(id), Some("pending"));
}

#[test]
fn punching_slot_expires_at_the_window_end() {
    let t0 = Instant::now();
    let mut table = Table::new();
    let id = fresh(&mut table, b"noise", t0);
    table.attach(id, "pending", t0 + INITIAL_TIMEOUT);
    table.extend(id, t0 + PUNCHING_TIMEOUT);
    assert_eq!(table.expire(t0 + PUNCHING_TIMEOUT), vec!["pending"]);
    assert!(table.expire(t0 + PUNCHING_TIMEOUT).is_empty());
}

#[test]
fn punching_timeout_covers_the_probe_window() {
    assert!(PUNCHING_TIMEOUT >= crate::server::punch::PUZZLE_LINGER);
    assert!(PUNCHING_TIMEOUT >= Duration::from_secs(30));
}
