use super::*;

fn crash_text(tail: &str) -> RangePlan {
    RangePlan::Crash(format!(
        "RangeError [ERR_OUT_OF_RANGE]: The value of {tail}"
    ))
}

#[test]
fn full_without_header() {
    assert_eq!(plan_range(None, 10), RangePlan::Full);
    assert_eq!(plan_range(Some(""), 10), RangePlan::Full);
}

#[test]
fn partial_ranges() {
    assert_eq!(
        plan_range(Some("bytes=0-3"), 10),
        RangePlan::Partial { start: 0, end: 3 }
    );
    assert_eq!(
        plan_range(Some("bytes=5-"), 10),
        RangePlan::Partial { start: 5, end: 9 }
    );
    assert_eq!(
        plan_range(Some("bytes=9-9"), 10),
        RangePlan::Partial { start: 9, end: 9 }
    );
    assert_eq!(
        plan_range(Some("bytes= 5-9"), 10),
        RangePlan::Partial { start: 5, end: 9 }
    );
}

#[test]
fn unsatisfiable() {
    assert_eq!(plan_range(Some("bytes=10-"), 10), RangePlan::Unsatisfiable);
    assert_eq!(plan_range(Some("bytes=0-10"), 10), RangePlan::Unsatisfiable);
}

#[test]
fn capital_bytes_is_not_stripped() {
    let want = crash_text("\"start\" is out of range. It must be an integer. Received NaN");
    assert_eq!(plan_range(Some("Bytes=0-1"), 10), want);
}

#[test]
fn nan_start_and_end() {
    let start = crash_text("\"start\" is out of range. It must be an integer. Received NaN");
    let end = crash_text("\"end\" is out of range. It must be an integer. Received NaN");
    assert_eq!(plan_range(Some("bytes=-5"), 10), start);
    assert_eq!(plan_range(Some("bytes=0-x"), 10), end);
}

#[test]
fn start_after_end() {
    let want = crash_text("\"start\" is out of range. It must be <= \"end\" (here: 2). Received 5");
    assert_eq!(plan_range(Some("bytes=5-2"), 10), want);
}

#[test]
fn empty_file_is_unsatisfiable() {
    assert_eq!(plan_range(Some("bytes=0-"), 0), RangePlan::Unsatisfiable);
}
