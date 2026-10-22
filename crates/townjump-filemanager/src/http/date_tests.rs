use super::http_date;
use std::time::{Duration, UNIX_EPOCH};

#[test]
fn epoch() {
    assert_eq!(http_date(UNIX_EPOCH), "Thu, 01 Jan 1970 00:00:00 GMT");
}

#[test]
fn known_instant() {
    let t = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    assert_eq!(http_date(t), "Tue, 14 Nov 2023 22:13:20 GMT");
}

#[test]
fn leap_day() {
    // 2024-02-29 12:00:00 UTC
    let t = UNIX_EPOCH + Duration::from_secs(1_709_208_000);
    assert_eq!(http_date(t), "Thu, 29 Feb 2024 12:00:00 GMT");
}

#[test]
fn before_epoch() {
    let t = UNIX_EPOCH - Duration::from_secs(1);
    assert_eq!(http_date(t), "Wed, 31 Dec 1969 23:59:59 GMT");
}
