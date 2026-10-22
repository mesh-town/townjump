use super::*;

fn run_down(a: &mut Adaptive, host: Option<&str>) {
    while a.countdown > 1 {
        assert!(!a.on_tick(true, host));
    }
}

#[test]
fn countdown_fires_on_last_tick() {
    let mut a = Adaptive::new(true, 3);
    assert!(!a.on_tick(true, Some("1.1.1.1")));
    assert!(!a.on_tick(true, Some("1.1.1.1")));
    assert!(a.on_tick(true, Some("1.1.1.1")));
}

#[test]
fn same_host_defers() {
    let mut a = Adaptive::new(true, 3);
    a.begin_check(Some("1.1.1.1"));
    a.countdown = 1;
    assert!(!a.on_tick(true, Some("1.1.1.1")));
    assert_eq!(a.countdown, a.more_stable());
}

#[test]
fn changed_host_fires() {
    let mut a = Adaptive::new(true, 3);
    a.begin_check(Some("1.1.1.1"));
    a.countdown = 1;
    assert!(a.on_tick(true, Some("2.2.2.2")));
}

#[test]
fn none_equals_none() {
    let mut a = Adaptive::new(true, 3);
    a.begin_check(None);
    a.countdown = 1;
    assert!(!a.on_tick(true, None));
    assert_eq!(a.countdown, 9);
}

#[test]
fn persistent_ignores_ticks() {
    let mut a = Adaptive::new(true, 3);
    for _ in 0..10 {
        assert!(!a.on_tick(false, Some("1.1.1.1")));
    }
    assert_eq!(a.countdown, 3);
}

#[test]
fn off_ignores_ticks() {
    let mut a = Adaptive::new(false, 3);
    for _ in 0..10 {
        assert!(!a.on_tick(true, Some("1.1.1.1")));
    }
    assert_eq!(a.countdown, 0);
}

#[test]
fn wakeup_resets_and_reports_demotion() {
    let mut a = Adaptive::new(true, 3);
    a.begin_check(Some("1.1.1.1"));
    run_down(&mut a, Some("1.1.1.1"));
    assert!(a.on_wakeup(false));
    assert_eq!(a.countdown, a.more_stable());
    assert_eq!(a.last_host, None);
    assert!(!a.on_wakeup(true));
}

#[test]
fn wakeup_when_off_does_not_demote() {
    let mut a = Adaptive::new(false, 3);
    assert!(!a.on_wakeup(false));
}

#[test]
fn enable_disable() {
    let mut a = Adaptive::new(true, 3);
    assert!(a.disable());
    assert!(!a.disable());
    assert!(!a.is_on());
    a.enable();
    assert!(a.is_on());
    assert_eq!(a.countdown, 3);
}
