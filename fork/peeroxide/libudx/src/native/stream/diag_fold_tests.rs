//! `ZS` fold: new ACK-side keys and step source, window reset.
use super::{AckProbe, Fold};
use std::time::Duration;

fn probe(kick: bool, open: bool, parked: bool, room0: bool) -> AckProbe {
    AckProbe {
        kick,
        open,
        parked,
        room0,
        unsacked: 300_000,
        q_len: 7,
        cwnd: 4,
        lost: 2,
    }
}

#[test]
fn zs_line_has_new_keys() {
    let line = Fold::ZERO.zs_line(Duration::from_millis(10));
    for key in [
        "a_n=",
        "a_kick=",
        "a_open_nokick=",
        "a_open_room0=",
        "a_parked_room0=",
        "unsacked_max=",
        "q_len_max=",
        "cwnd_min=-1",
        "lost_max=",
        "ack_proc_us_max=",
        "far_armed=",
        "step_kick=",
        "step_self=",
        "kick_to_step_n=",
        "kick_to_step_us_sum=",
        "kick_to_step_us_max=",
        "step_inline=",
    ] {
        assert!(line.contains(key), "no {key} in {line}");
    }
    assert!(line.starts_with("ZS t="), "{line}");
}

#[test]
fn ack_counters_follow_the_contract() {
    let mut f = Fold::ZERO;
    for p in [
        probe(false, true, true, true),
        probe(true, true, false, false),
        probe(false, false, true, true),
    ] {
        f.note_ack(&p);
    }
    let line = f.zs_line(Duration::from_millis(10));
    for want in [
        "a_n=3 ",
        "a_kick=1 ",
        "a_open_nokick=1 ",
        "a_open_room0=1 ",
        "a_parked_room0=2 ",
        "unsacked_max=300000 ",
        "q_len_max=7 ",
        "cwnd_min=4 ",
        "lost_max=2 ",
    ] {
        assert!(line.contains(want), "no {want} in {line}");
    }
}

#[test]
fn kick_to_step_sum_and_max() {
    let mut f = Fold::ZERO;
    f.note_kick_to_step(40);
    f.note_kick_to_step(900);
    f.note_kick_to_step(60);
    f.step_inline = 2;
    let line = f.zs_line(Duration::from_millis(10));
    for want in [
        "kick_to_step_n=3 ",
        "kick_to_step_us_sum=1000 ",
        "kick_to_step_us_max=900 ",
        "step_inline=2",
    ] {
        assert!(line.contains(want), "no {want} in {line}");
    }
}
