// Tests for `ping` on primitive and array records: as in JS, no `protocol` is added to them.
use crate::support::*;
use crate::{LookupRecord, ping};
use townjump_jsval::JsValue;
use townjump_testkit::mock::MockFactory;

async fn pinged(value: &str) -> Option<LookupRecord> {
    let factory = MockFactory::new();
    publish(&factory, DERIVED_PK, value, 1).await;
    within("ping", ping(&factory, PUBLIC_Z32))
        .await
        .expect("ping")
}

fn record(value: JsValue) -> Option<LookupRecord> {
    Some(LookupRecord { record: value })
}

#[tokio::test]
async fn number_and_string_and_true_are_returned_as_they_are() {
    assert_eq!(pinged("5").await, record(num(5.0)));
    assert_eq!(pinged(r#""str""#).await, record(text("str")));
    assert_eq!(pinged("true").await, record(JsValue::Bool(true)));
}

#[tokio::test]
async fn array_stays_an_array() {
    let got = pinged("[1]").await;
    assert_eq!(got, record(JsValue::Array(vec![num(1.0)])));
}

#[tokio::test]
async fn falsy_primitive_from_a_survives_an_empty_b() {
    // A gave `0` (falsy) - go to B; it is empty, `result` stays `0` (as in JS).
    assert_eq!(pinged("0").await, record(num(0.0)));
}

#[tokio::test]
async fn falsy_primitive_in_attempt_b_is_returned_as_it_is() {
    let factory = MockFactory::new();
    publish(&factory, PUBLIC_PK, "0", 1).await;
    let got = within("ping", ping(&factory, PUBLIC_Z32))
        .await
        .expect("ping");
    assert_eq!(got, record(num(0.0)));
}
