//! Loss of an MTU probe is not a congestion signal (C `rack_detect_loss`, udx.c:1046-1102, RFC 8899): `Recovery` is entered only if the batch
//! of losses contains a non-probe. The probe counter is a local variable of the call, nothing accumulates between calls.
//! Times are explicit (`set_time_sent`), no sleeping.
use super::test_support::{resend_lost_all, stream_inner};
use super::*;
use std::time::Duration;

/// One packet `seq` (DATA) with `time_sent = at`; `is_probe` as for a sent probe.
fn send_at(inner: &mut StreamInner, seq: u32, is_probe: bool, at: Instant) {
    let packet = build_data_packet(inner.remote_id, seq, 0, b"x");
    inner.register_sent(seq, packet, is_probe);
    inner.outgoing.set_time_sent(Seq(seq), at);
    inner.next_seq = seq.wrapping_add(1);
}

/// Packets 0..9 in send order (times ascending, all in the past); the numbers in `probes` are probes.
fn sent_with_probes(probes: &[u32]) -> StreamInner {
    let mut inner = stream_inner();
    let base = Instant::now() - Duration::from_micros(100);
    for seq in 0..10 {
        let at = base + Duration::from_micros(u64::from(seq));
        send_at(&mut inner, seq, probes.contains(&seq), at);
    }
    inner.remote_acked = 0;
    inner
}

fn sack(inner: &mut StreamInner, start: u32, end: u32) {
    inner.on_sack(&[SackRange { start, end }]);
}

/// Retransmits the lost and pushes the retransmissions into the future: a new SACK does not mark them again (RACK: a retransmission was sent later than everything delivered).
fn resend_and_hold(inner: &mut StreamInner, seqs: &[u32]) {
    resend_lost_all(inner);
    let later = Instant::now() + Duration::from_secs(1);
    seqs.iter()
        .for_each(|&s| inner.outgoing.set_time_sent(Seq(s), later));
}

/// Three packets from `first`: the first is a probe or data per `first_is_probe`, times in the past, below the future retransmissions.
fn send_batch(inner: &mut StreamInner, first: u32, first_is_probe: bool) {
    let base = Instant::now() - Duration::from_micros(50);
    for i in 0..3 {
        let at = base + Duration::from_micros(u64::from(i));
        send_at(inner, first + i, first_is_probe && i == 0, at);
    }
}

/// A hole only on probe 0: the loss count stays the same (1), `ca_state` stays `Open`, `high_seq` is not written.
#[test]
fn lost_probe_alone_does_not_enter_recovery() {
    let mut inner = sent_with_probes(&[0]);
    sack(&mut inner, 1, 10);
    assert_eq!(
        inner.detect_losses(),
        1,
        "the loss count includes the probe (as C stream->lost)"
    );
    assert_eq!(inner.congestion.ca_state, CaState::Open);
    assert_eq!(inner.congestion.high_seq, 0);
}

/// Holes on probe 0 and on data 1: enters `Recovery`, `high_seq` is the highest sent number. The same holds for upstream.
#[test]
fn lost_probe_together_with_data_enters_recovery() {
    let mut inner = sent_with_probes(&[0]);
    sack(&mut inner, 2, 10);
    assert_eq!(inner.detect_losses(), 2);
    assert_eq!(inner.congestion.ca_state, CaState::Recovery);
    assert_eq!(inner.congestion.high_seq, 9);
}

/// A batch of one probe, then a batch with a data loss: the first accumulates nothing, the second enters `Recovery`.
#[test]
fn data_loss_after_a_probe_only_batch_enters_recovery() {
    let mut inner = sent_with_probes(&[0]);
    sack(&mut inner, 1, 10);
    assert_eq!(inner.detect_losses(), 1);
    send_batch(&mut inner, 10, false);
    resend_and_hold(&mut inner, &[0]);
    sack(&mut inner, 11, 13);
    assert_eq!(inner.detect_losses(), 1, "packet 10 lost — data");
    assert_eq!(inner.congestion.ca_state, CaState::Recovery);
}

/// A probe and data in one batch enter `Recovery`; after leaving (`Open`) a batch of a single probe again changes nothing.
#[test]
fn probe_only_loss_after_recovery_exit_stays_open() {
    let mut inner = sent_with_probes(&[0]);
    sack(&mut inner, 2, 10);
    assert_eq!(inner.detect_losses(), 2);
    assert_eq!(inner.congestion.ca_state, CaState::Recovery);
    inner.congestion.ca_state = CaState::Open; // leaving Recovery: all numbers up to high_seq are acked
    send_batch(&mut inner, 10, true);
    resend_and_hold(&mut inner, &[0, 1]);
    sack(&mut inner, 11, 13);
    assert_eq!(inner.detect_losses(), 1, "packet 10 lost — probe");
    assert_eq!(inner.congestion.ca_state, CaState::Open);
}
