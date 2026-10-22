use super::*;

fn fill(w: &mut NatWindow, host: &str, port: u16, n: usize) {
    for _ in 0..n {
        w.add(host, port);
    }
}

/// Window after `a` samples of A:1 followed by `b` samples of B:1.
fn mixed(a: usize, b: usize) -> NatWindow {
    let mut w = NatWindow::new();
    fill(&mut w, "1.1.1.1", 1, a);
    fill(&mut w, "2.2.2.2", 1, b);
    w
}

#[test]
fn empty_window_has_no_host() {
    let w = NatWindow::new();
    assert_eq!(w.host(), None);
    assert_eq!(w.port(), 0);
    assert_eq!(w.size(), 0);
}

#[test]
fn first_sample_sets_host() {
    let mut w = NatWindow::new();
    assert_eq!(w.add("1.1.1.1", 1), 1);
    assert_eq!((w.host(), w.port()), (Some("1.1.1.1"), 1));
}

#[test]
fn two_disagreeing_hosts_give_no_host() {
    assert_eq!(mixed(1, 1).host(), None);
}

#[test]
fn four_samples_tolerate_one_dissenter() {
    assert_eq!(mixed(3, 1).host(), Some("1.1.1.1"));
}

#[test]
fn four_samples_with_two_dissenters_give_none() {
    assert_eq!(mixed(2, 2).host(), None);
}

#[test]
fn eight_samples_tolerate_two() {
    assert_eq!(mixed(6, 2).host(), Some("1.1.1.1"));
    assert_eq!(mixed(5, 3).host(), None);
}

#[test]
fn twelve_samples_tolerate_three() {
    assert_eq!(mixed(9, 3).host(), Some("1.1.1.1"));
    assert_eq!(mixed(8, 4).host(), None);
}

#[test]
fn window_holds_sixteen_samples() {
    let mut w = NatWindow::new();
    fill(&mut w, "1.1.1.1", 1, 40);
    assert_eq!(w.size(), 16);
}

#[test]
fn new_host_takes_over_after_thirteen() {
    let mut w = NatWindow::new();
    fill(&mut w, "1.1.1.1", 1, 16);
    for k in 1..=13 {
        w.add("2.2.2.2", 1);
        match k {
            1..=3 => assert_eq!(w.host(), Some("1.1.1.1"), "k={k}"),
            4..=12 => assert_eq!(w.host(), None, "k={k}"),
            _ => assert_eq!(w.host(), Some("2.2.2.2"), "k={k}"),
        }
    }
}

#[test]
fn same_host_varied_ports_report_port_zero() {
    let mut w = NatWindow::new();
    for p in 1..=4 {
        w.add("1.1.1.1", p);
    }
    assert_eq!((w.host(), w.port()), (Some("1.1.1.1"), 0));
}

#[test]
fn random_ports_after_full_window() {
    let run = |n: u16| {
        let mut w = NatWindow::new();
        fill(&mut w, "1.1.1.1", 9, 16);
        for k in 0..n {
            w.add("2.2.2.2", 40000 + k);
        }
        (w.host().map(str::to_owned), w.port())
    };
    assert_eq!(run(13), (Some("2.2.2.2".to_owned()), 0));
    assert_eq!(run(12), (None, 0));
}

#[test]
fn repeat_within_four_samples_merges() {
    let mut w = NatWindow::new();
    w.add("1.1.1.1", 1);
    for (i, h) in ["3.3.3.1", "3.3.3.2", "3.3.3.3"].iter().enumerate() {
        w.add(h, 10 + i as u16);
    }
    assert_eq!(w.add("1.1.1.1", 1), 2);
}

#[test]
fn repeat_after_four_others_starts_new_entry() {
    let mut w = NatWindow::new();
    w.add("1.1.1.1", 1);
    for (i, h) in ["3.3.3.1", "3.3.3.2", "3.3.3.3", "3.3.3.4"]
        .iter()
        .enumerate()
    {
        w.add(h, 10 + i as u16);
    }
    assert_eq!(w.add("1.1.1.1", 1), 1);
}

#[test]
fn entries_stay_bounded() {
    let mut w = NatWindow::new();
    for i in 0..1000u32 {
        w.add(
            &format!("10.{}.{}.{}", i >> 16, (i >> 8) & 255, i & 255),
            (i % 60000) as u16 + 1,
        );
        assert!(w.entries_len() <= 36, "i={i}");
    }
}

// Generated from nat-sampler 1.0.1 (columns: host, port, expected host, expected port):
// // cd fork/peeroxide/tests/node && node -e "const S=require('nat-sampler'),s=new S(),A=['1.1.1.1',5000],B=['2.2.2.2',6000],q=[];for(let i=0;i<16;i++)q.push(A);for(let i=0;i<3;i++)q.push(B);q.push(A);for(let i=0;i<6;i++)q.push(i%2?A:B);for(let i=0;i<14;i++)q.push(B);for(let i=0;i<5;i++)q.push(['3.3.3.3',40000+i]);q.push(A,['4.4.4.4',1],['4.4.4.5',1],['4.4.4.6',1],A);for(let i=0;i<14;i++)q.push(['3.3.3.3',41000+i]);for(const [h,p] of q){s.add(h,p);console.log(JSON.stringify([h,p,s.host,s.port]))}"
const JS_VECTOR: &[(&str, u16, Option<&str>, u16)] = &[
    ("1.1.1.1", 5000, Some("1.1.1.1"), 5000),
    ("1.1.1.1", 5000, Some("1.1.1.1"), 5000),
    ("1.1.1.1", 5000, Some("1.1.1.1"), 5000),
    ("1.1.1.1", 5000, Some("1.1.1.1"), 5000),
    ("1.1.1.1", 5000, Some("1.1.1.1"), 5000),
    ("1.1.1.1", 5000, Some("1.1.1.1"), 5000),
    ("1.1.1.1", 5000, Some("1.1.1.1"), 5000),
    ("1.1.1.1", 5000, Some("1.1.1.1"), 5000),
    ("1.1.1.1", 5000, Some("1.1.1.1"), 5000),
    ("1.1.1.1", 5000, Some("1.1.1.1"), 5000),
    ("1.1.1.1", 5000, Some("1.1.1.1"), 5000),
    ("1.1.1.1", 5000, Some("1.1.1.1"), 5000),
    ("1.1.1.1", 5000, Some("1.1.1.1"), 5000),
    ("1.1.1.1", 5000, Some("1.1.1.1"), 5000),
    ("1.1.1.1", 5000, Some("1.1.1.1"), 5000),
    ("1.1.1.1", 5000, Some("1.1.1.1"), 5000),
    ("2.2.2.2", 6000, Some("1.1.1.1"), 5000),
    ("2.2.2.2", 6000, Some("1.1.1.1"), 5000),
    ("2.2.2.2", 6000, Some("1.1.1.1"), 5000),
    ("1.1.1.1", 5000, Some("1.1.1.1"), 5000),
    ("2.2.2.2", 6000, None, 0),
    ("1.1.1.1", 5000, None, 0),
    ("2.2.2.2", 6000, None, 0),
    ("1.1.1.1", 5000, None, 0),
    ("2.2.2.2", 6000, None, 0),
    ("1.1.1.1", 5000, None, 0),
    ("2.2.2.2", 6000, None, 0),
    ("2.2.2.2", 6000, None, 0),
    ("2.2.2.2", 6000, None, 0),
    ("2.2.2.2", 6000, None, 0),
    ("2.2.2.2", 6000, None, 0),
    ("2.2.2.2", 6000, None, 0),
    ("2.2.2.2", 6000, None, 0),
    ("2.2.2.2", 6000, None, 0),
    ("2.2.2.2", 6000, None, 0),
    ("2.2.2.2", 6000, Some("2.2.2.2"), 6000),
    ("2.2.2.2", 6000, Some("2.2.2.2"), 6000),
    ("2.2.2.2", 6000, Some("2.2.2.2"), 6000),
    ("2.2.2.2", 6000, Some("2.2.2.2"), 6000),
    ("2.2.2.2", 6000, Some("2.2.2.2"), 6000),
    ("3.3.3.3", 40000, Some("2.2.2.2"), 6000),
    ("3.3.3.3", 40001, Some("2.2.2.2"), 6000),
    ("3.3.3.3", 40002, Some("2.2.2.2"), 6000),
    ("3.3.3.3", 40003, None, 0),
    ("3.3.3.3", 40004, None, 0),
    ("1.1.1.1", 5000, None, 0),
    ("4.4.4.4", 1, None, 0),
    ("4.4.4.5", 1, None, 0),
    ("4.4.4.6", 1, None, 0),
    ("1.1.1.1", 5000, None, 0),
    ("3.3.3.3", 41000, None, 0),
    ("3.3.3.3", 41001, None, 0),
    ("3.3.3.3", 41002, None, 0),
    ("3.3.3.3", 41003, None, 0),
    ("3.3.3.3", 41004, None, 0),
    ("3.3.3.3", 41005, None, 0),
    ("3.3.3.3", 41006, None, 0),
    ("3.3.3.3", 41007, None, 0),
    ("3.3.3.3", 41008, None, 0),
    ("3.3.3.3", 41009, None, 0),
    ("3.3.3.3", 41010, None, 0),
    ("3.3.3.3", 41011, None, 0),
    ("3.3.3.3", 41012, Some("3.3.3.3"), 0),
    ("3.3.3.3", 41013, Some("3.3.3.3"), 0),
];

#[test]
fn matches_js_vector() {
    let mut w = NatWindow::new();
    for (step, &(host, port, want_host, want_port)) in JS_VECTOR.iter().enumerate() {
        w.add(host, port);
        assert_eq!(
            (w.host(), w.port()),
            (want_host, want_port),
            "step {}",
            step + 1
        );
    }
}
