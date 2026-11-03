use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};

use crate::interfaces::ipv4_interfaces;
use crate::{InterfaceSource, LanConfig, Responder};

/// A source whose list a test switches at will.
fn switchable(initial: Vec<Ipv4Addr>) -> (InterfaceSource, Arc<Mutex<Vec<Ipv4Addr>>>) {
    let list = Arc::new(Mutex::new(initial));
    let shared = list.clone();
    (
        InterfaceSource::new(move || shared.lock().unwrap().clone()),
        list,
    )
}

fn cfg(port: u16) -> LanConfig {
    LanConfig {
        port,
        ..LanConfig::default()
    }
}

/// The interfaces of the host; `None` (with a note) when it has no non-loopback IPv4 one.
fn host_interfaces() -> Option<Vec<Ipv4Addr>> {
    let all = ipv4_interfaces();
    if all.is_empty() {
        eprintln!("skipped: no non-loopback IPv4 interface on this host");
        return None;
    }
    let mut sorted = all;
    sorted.sort();
    Some(sorted)
}

#[tokio::test]
async fn refresh_joins_an_interface_that_appeared() {
    let Some(all) = host_interfaces() else { return };
    let (source, list) = switchable(Vec::new());
    let r = Responder::start_with(&cfg(28701), source).unwrap();
    assert!(r.joined_interfaces().is_empty());
    *list.lock().unwrap() = ipv4_interfaces();
    r.refresh_interfaces();
    assert_eq!(r.joined_interfaces(), all);
}

#[tokio::test]
async fn refresh_forgets_a_vanished_interface_and_joins_it_again() {
    let Some(all) = host_interfaces() else { return };
    let (source, list) = switchable(ipv4_interfaces());
    let r = Responder::start_with(&cfg(28702), source).unwrap();
    assert_eq!(r.joined_interfaces(), all);
    list.lock().unwrap().clear();
    r.refresh_interfaces();
    assert!(r.joined_interfaces().is_empty());
    *list.lock().unwrap() = ipv4_interfaces();
    r.refresh_interfaces();
    assert_eq!(r.joined_interfaces(), all);
}

#[tokio::test]
async fn a_second_refresh_without_changes_keeps_the_membership() {
    let Some(all) = host_interfaces() else { return };
    let (source, _list) = switchable(ipv4_interfaces());
    let r = Responder::start_with(&cfg(28703), source).unwrap();
    r.refresh_interfaces();
    r.refresh_interfaces();
    assert_eq!(r.joined_interfaces(), all);
}

#[tokio::test]
async fn a_failed_join_is_not_counted_and_is_retried() {
    let Some(all) = host_interfaces() else { return };
    // TEST-NET-1: no interface of the host has this address, so the join fails.
    let missing = Ipv4Addr::new(192, 0, 2, 1);
    let (source, list) = switchable(vec![missing]);
    let r = Responder::start_with(&cfg(28704), source).unwrap();
    assert!(r.joined_interfaces().is_empty());
    let mut both = vec![missing];
    both.extend(ipv4_interfaces());
    *list.lock().unwrap() = both;
    r.refresh_interfaces();
    assert_eq!(r.joined_interfaces(), all);
}
