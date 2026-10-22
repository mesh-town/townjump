use super::*;
use std::io::{Error, ErrorKind};

#[test]
fn echo_kinds_are_skipped() {
    assert!(is_send_echo(&Error::from(ErrorKind::ConnectionReset)));
    assert!(is_send_echo(&Error::from(ErrorKind::ConnectionAborted)));
}

#[test]
fn other_kinds_are_fatal() {
    assert!(!is_send_echo(&Error::from(ErrorKind::WouldBlock)));
    assert!(!is_send_echo(&Error::from(ErrorKind::ConnectionRefused)));
    assert!(!is_send_echo(&Error::other("x")));
}

#[cfg(windows)]
#[test]
fn winsock_echo_codes_are_skipped() {
    assert!(is_send_echo(&Error::from_raw_os_error(10054)));
    assert!(is_send_echo(&Error::from_raw_os_error(10052)));
    assert!(is_send_echo(&Error::from_raw_os_error(10053)));
}

#[cfg(windows)]
#[test]
fn winsock_msgsize_is_fatal() {
    assert!(!is_send_echo(&Error::from_raw_os_error(10040)));
}

#[test]
fn no_pause_below_threshold() {
    assert_eq!(pause_after(0), None);
    assert_eq!(pause_after(SKIPS_BEFORE_PAUSE - 1), None);
}

#[test]
fn pause_from_threshold() {
    assert_eq!(pause_after(SKIPS_BEFORE_PAUSE), Some(SKIP_PAUSE));
    assert_eq!(pause_after(u32::MAX), Some(SKIP_PAUSE));
}
