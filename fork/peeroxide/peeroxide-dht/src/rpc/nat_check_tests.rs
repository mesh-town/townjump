use super::*;

fn peer(host: &str, port: u16) -> Ipv4Peer {
    Ipv4Peer {
        host: host.to_owned(),
        port,
    }
}

fn check(asked: usize, replies: usize, arrival: bool, to: &Ipv4Peer) -> NatCheck {
    let mut c = NatCheck::new(peer("1.1.1.1", 1));
    for _ in 0..asked {
        c.track();
    }
    for _ in 0..replies {
        c.on_reply(arrival, to);
    }
    c
}

#[test]
fn threshold_small() {
    let to = peer("1.1.1.1", 4000);
    let c = check(2, 1, true, &to);
    assert_eq!(c.outcome(Some("1.1.1.1"), 4000), Some(to));
}

#[test]
fn threshold_five() {
    let to = peer("1.1.1.1", 4000);
    assert_eq!(check(5, 2, true, &to).outcome(Some("1.1.1.1"), 4000), None);
    assert_eq!(
        check(5, 3, true, &to).outcome(Some("1.1.1.1"), 4000),
        Some(to)
    );
}

#[test]
fn client_arrival_ignored() {
    let to = peer("1.1.1.1", 4000);
    assert_eq!(check(1, 1, false, &to).outcome(Some("1.1.1.1"), 4000), None);
}

#[test]
fn wrong_host() {
    let to = peer("2.2.2.2", 4000);
    assert_eq!(check(1, 1, true, &to).outcome(Some("1.1.1.1"), 4000), None);
    assert_eq!(check(1, 1, true, &to).outcome(None, 4000), None);
}

#[test]
fn wrong_port() {
    let to = peer("1.1.1.1", 5000);
    assert_eq!(check(1, 1, true, &to).outcome(Some("1.1.1.1"), 4000), None);
}

#[test]
fn no_targets() {
    let c = NatCheck::new(peer("1.1.1.1", 1));
    assert_eq!(c.outcome(Some("1.1.1.1"), 4000), None);
}

#[test]
fn done_and_saturating_timeout() {
    let mut c = NatCheck::new(peer("1.1.1.1", 1));
    c.track();
    c.track();
    assert!(!c.is_done());
    c.on_reply(true, &peer("1.1.1.1", 4000));
    c.on_timeout();
    assert!(c.is_done());
    c.on_timeout();
    assert!(c.is_done());
}

fn nodes(n: u16) -> Vec<(u64, Ipv4Peer)> {
    (0..n)
        .map(|i| (u64::from((i * 7) % 11), peer("9.9.9.9", i)))
        .collect()
}

#[test]
fn targets_top_five_by_recency() {
    let got = check_targets(nodes(7), &[("5.5.5.5".into(), 1)]);
    assert_eq!(got.len(), 5);
    let mut seen: Vec<u64> = nodes(7).iter().map(|(s, _)| *s).collect();
    seen.sort_by_key(|s| std::cmp::Reverse(*s));
    let expect: Vec<u16> = seen[..5]
        .iter()
        .map(|s| nodes(7).into_iter().find(|(x, _)| x == s).unwrap().1.port)
        .collect();
    assert_eq!(got.iter().map(|p| p.port).collect::<Vec<_>>(), expect);
}

#[test]
fn targets_topped_up_with_bootstrap() {
    let bs = vec![("5.5.5.5".to_owned(), 1), ("6.6.6.6".to_owned(), 2)];
    let got = check_targets(nodes(3), &bs);
    assert_eq!(got.len(), 5);
    assert_eq!(&got[3..], &[peer("5.5.5.5", 1), peer("6.6.6.6", 2)]);
}

#[test]
fn targets_add_every_bootstrap_when_table_is_short() {
    let bs: Vec<(String, u16)> = (1..=4).map(|i| (format!("7.7.7.{i}"), i)).collect();
    let got = check_targets(nodes(3), &bs);
    assert_eq!(got.len(), 7);
}

#[test]
fn targets_skip_bootstrap_when_table_is_full() {
    let bs = vec![("5.5.5.5".to_owned(), 1)];
    assert_eq!(check_targets(nodes(5), &bs).len(), 5);
}

#[test]
fn targets_keep_duplicate_of_bootstrap() {
    let bs = vec![("9.9.9.9".to_owned(), 0)];
    let got = check_targets(vec![(1, peer("9.9.9.9", 0))], &bs);
    assert_eq!(got, vec![peer("9.9.9.9", 0), peer("9.9.9.9", 0)]);
}

#[test]
fn targets_empty() {
    assert!(check_targets(vec![], &[]).is_empty());
}

#[test]
fn parses_bootstrap_strings() {
    let raw = vec!["1.2.3.4:5".to_owned(), "bad".to_owned()];
    assert_eq!(parse_bootstrap(&raw), vec![("1.2.3.4".to_owned(), 5)]);
}
