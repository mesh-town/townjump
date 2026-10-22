// Port extension: at startup Node raises the soft RLIMIT_NOFILE (node.cc:PlatformInit).
// Without it the file server hits 256 descriptors on macOS.

/// The largest value in `(min, max)` accepted by `try_set` (binary search, as in Node); `min` is already accepted.
#[cfg(unix)]
pub(crate) fn search_limit(
    mut min: u64,
    mut max: u64,
    mut try_set: impl FnMut(u64) -> bool,
) -> u64 {
    while min + 1 < max {
        let mid = min + (max - min) / 2;
        if try_set(mid) {
            min = mid;
        } else {
            max = mid;
        }
    }
    min
}

/// Raise the soft open-file limit to the hard one; if the hard one is not accepted (macOS: above
/// `kern.maxfilesperproc`) or is infinite — binary search for the largest accepted (infinite — up to 1 << 20).
/// Errors are silent: the limit stays as it was.
#[cfg(unix)]
pub fn raise_nofile_limit() {
    use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};
    let limit = getrlimit(Resource::Nofile);
    if limit.current == limit.maximum {
        return;
    }
    let set = |value: u64| {
        let attempt = Rlimit {
            current: Some(value),
            maximum: limit.maximum,
        };
        setrlimit(Resource::Nofile, attempt).is_ok()
    };
    let top = limit.maximum.unwrap_or(1 << 20);
    if limit.maximum.is_some() && set(top) {
        return;
    }
    search_limit(limit.current.unwrap_or(0), top, set);
}

#[cfg(not(unix))]
pub fn raise_nofile_limit() {}

#[cfg(test)]
#[path = "fd_limit_tests.rs"]
mod tests;
