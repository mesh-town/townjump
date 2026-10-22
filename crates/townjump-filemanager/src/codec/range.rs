// JS: livefiles/index.js:604-620 (Range parsing) and fs.createReadStream checks.
use super::text::{js_num, parse_int10, replace_first};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RangePlan {
    Full,
    Partial { start: u64, end: u64 },
    Unsatisfiable,
    Crash(String),
}

const OUT_OF_RANGE: &str = "RangeError [ERR_OUT_OF_RANGE]: The value of ";

pub(crate) fn plan_range(header: Option<&str>, size: u64) -> RangePlan {
    let h = match header {
        Some(h) if !h.is_empty() => h,
        _ => return RangePlan::Full,
    };
    let stripped = replace_first(h, "bytes=", "");
    let mut parts = stripped.split('-');
    let start = parse_int10(parts.next().unwrap_or(""));
    let end = match parts.next() {
        Some(p) if !p.is_empty() => parse_int10(p),
        _ => size as f64 - 1.0,
    };
    if start >= size as f64 || end >= size as f64 {
        return RangePlan::Unsatisfiable;
    }
    stream_checks(start, end)
}

/// The `fs.createReadStream({ start, end })` checks, in order.
fn stream_checks(start: f64, end: f64) -> RangePlan {
    if start.is_nan() {
        return crash(format!(
            "\"start\" is out of range. It must be an integer. Received {}",
            js_num(start)
        ));
    }
    if end.is_nan() {
        return crash(format!(
            "\"end\" is out of range. It must be an integer. Received {}",
            js_num(end)
        ));
    }
    if start > end {
        let tail = format!(
            "\"start\" is out of range. It must be <= \"end\" (here: {}). Received {}",
            js_num(end),
            js_num(start)
        );
        return crash(tail);
    }
    RangePlan::Partial {
        start: start as u64,
        end: end as u64,
    }
}

fn crash(tail: String) -> RangePlan {
    RangePlan::Crash(format!("{OUT_OF_RANGE}{tail}"))
}

#[cfg(test)]
#[path = "range_tests.rs"]
mod tests;
