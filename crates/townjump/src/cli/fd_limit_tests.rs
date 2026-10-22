#![cfg(unix)]
use super::raise_nofile_limit;
use rustix::process::{Resource, getrlimit};

#[test]
fn soft_limit_does_not_decrease() {
    let before = getrlimit(Resource::Nofile);
    raise_nofile_limit();
    let after = getrlimit(Resource::Nofile);
    let rank = |v: Option<u64>| v.unwrap_or(u64::MAX);
    assert!(rank(after.current) >= rank(before.current));
}

/// Linux: with a finite hard limit the soft one equals it (macOS may refuse above `kern.maxfilesperproc`).
#[cfg(target_os = "linux")]
#[test]
fn soft_limit_reaches_finite_hard_limit() {
    raise_nofile_limit();
    let after = getrlimit(Resource::Nofile);
    if after.maximum.is_some() {
        assert_eq!(after.current, after.maximum);
    }
}

#[test]
fn search_finds_the_largest_accepted_value() {
    use super::search_limit;
    for cap in [257u64, 1000, 10240, 65535, 99999] {
        assert_eq!(search_limit(256, 1 << 20, |v| v <= cap), cap);
    }
    assert_eq!(search_limit(256, 1 << 20, |_| false), 256);
    assert_eq!(search_limit(256, 257, |_| true), 256);
}
