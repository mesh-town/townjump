use super::*;

#[test]
fn epoch_and_known_instants() {
    assert_eq!(iso_from_millis(0), "1970-01-01T00:00:00.000Z");
    assert_eq!(
        iso_from_millis(1_767_323_045_678),
        "2026-01-02T03:04:05.678Z"
    );
    // leap day
    assert_eq!(iso_from_millis(951_782_400_000), "2000-02-29T00:00:00.000Z");
    assert_eq!(
        iso_from_millis(1_709_251_199_999),
        "2024-02-29T23:59:59.999Z"
    );
}

#[test]
fn now_has_fixed_width() {
    let s = now_iso();
    assert_eq!(s.len(), 24);
    assert!(s.ends_with('Z'));
}
