//! Link MTU reading tests: no outside network, loopback only.

#[cfg(any(target_os = "linux", target_vendor = "apple", windows))]
use super::link_mtu::link_mtu;

#[cfg(target_os = "linux")]
#[test]
fn link_mtu_loopback_is_at_least_1500_on_linux() {
    let m = link_mtu("127.0.0.1:9".parse().unwrap()).expect("IP_MTU on loopback");
    assert!(m >= 1500, "loopback mtu {m}");
}

#[cfg(target_vendor = "apple")]
#[test]
fn link_mtu_is_none_on_apple() {
    assert_eq!(link_mtu("127.0.0.1:9".parse().unwrap()), None);
}

#[cfg(windows)]
#[test]
fn link_mtu_is_none_on_windows() {
    assert_eq!(link_mtu("127.0.0.1:9".parse().unwrap()), None);
}
