//! Random histories: the map with the send-order index and the map with the old walk behave identically at every
//! step. Fails on a forgotten index hook (`Twin::check`: keys ≠ recount from the map, or a `debug_assert` in
//! `SendOrder::remove`), on a wrong walk and on an unsorted result (`assert_eq!` in `Twin::rack`).
use super::twin::{Lcg, Twin};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::time::Duration;

mod random_histories_match_the_scan {
    use super::*;

    #[test]
    fn same_losses_counters_and_index_for_32_seeds_of_600_steps() {
        // given: the series starts 100 numbers before 2^32 — wraparound in the first sends
        let (rack_losses, blocks) = (1..=32)
            .map(history)
            .fold((0, 0), |(l, b), (dl, db)| (l + dl, b + db));
        // then: every step is checked inside `history`; over 32 series RACK lost packets and blocks marked new ones — not degenerate
        assert!(
            rack_losses > 0,
            "RACK never marked a packet: the series are empty"
        );
        assert!(blocks > 0, "no SACK block marked any new numbers");
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// One series of 600 steps; result — (how many packets RACK marked, how many blocks marked something new). On failure prints the seed and step.
fn history(seed: u64) -> (usize, usize) {
    let mut twin = Twin::starting_at(u32::MAX - 100);
    let mut rng = Lcg(seed);
    for step in 0..600 {
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            random_step(&mut twin, &mut rng);
            twin.check();
        }));
        if let Err(panic) = outcome {
            eprintln!("series {seed}, step {step}");
            resume_unwind(panic);
        }
    }
    (twin.rack_losses, twin.blocks_with_new)
}

/// Time (0–2 ms; 0 gives equal times), then one action by a 0..100 roll.
fn random_step(twin: &mut Twin, rng: &mut Lcg) {
    twin.tick(u64::from(rng.below(3)));
    match rng.below(100) {
        0..30 => twin.send(1 + rng.below(3)),
        30..35 => burst_if_room(twin, rng),
        35..48 => random_sack(twin, rng),
        48..60 => random_block(twin, rng),
        60..70 => random_ack(twin, rng),
        70..85 => checked_rack(twin, rng),
        85..88 => {
            twin.tick(500);
            twin.rto(Duration::ZERO);
        }
        _ => {
            twin.resend_all();
        }
    }
}

/// No more than 300 numbers in flight.
fn burst_if_room(twin: &mut Twin, rng: &mut Lcg) {
    if twin.next.wrapping_sub(twin.acked) < 300 {
        twin.send_burst(1 + rng.below(3));
    }
}

/// SACK of a random number from `[acked, next)`.
fn random_sack(twin: &mut Twin, rng: &mut Lcg) {
    let span = twin.next.wrapping_sub(twin.acked);
    if span > 0 {
        twin.sack(twin.acked.wrapping_add(rng.below(span)));
    }
}

/// A SACK block of width 1..40 inside `[acked, next)` (clipped at `next`).
fn random_block(twin: &mut Twin, rng: &mut Lcg) {
    let span = twin.next.wrapping_sub(twin.acked);
    if span > 0 {
        let off = rng.below(span);
        let width = (1 + rng.below(40)).min(span - off);
        let start = twin.acked.wrapping_add(off);
        twin.sack_block(start, start.wrapping_add(width));
    }
}

/// Cumulative ACK 0..=span/2 forward.
fn random_ack(twin: &mut Twin, rng: &mut Lcg) {
    let span = twin.next.wrapping_sub(twin.acked);
    twin.ack(twin.acked.wrapping_add(rng.below(span / 2 + 1)));
}

/// RACK with a reserve of 0, 1 or 5 ms; with a known `rack.sent` the index walked exactly the matching ones (counted before the call).
fn checked_rack(twin: &mut Twin, rng: &mut Lcg) {
    let reo = Duration::from_millis([0u64, 1, 5][rng.below(3) as usize]);
    let eligible = twin.eligible();
    let has_rack_sent = twin.new.rack().sent.is_some();
    twin.rack(reo);
    if has_rack_sent {
        assert_eq!(
            twin.new.rack_visited, eligible,
            "rack_visited ≠ number of matching"
        );
    }
}
