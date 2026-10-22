// Tests of the `reports_close_on_drop` flag of `DhtConnection`: a real stream sends `close` on drop, the JS fake does not.
use super::*;

fn stream() -> Box<dyn Duplex> {
    Box::new(tokio::io::duplex(8).0)
}

#[test]
fn new_connection_reports_close_on_drop() {
    let conn = DhtConnection::new([3; 32], stream());

    assert!(conn.reports_close_on_drop());
    assert_eq!(conn.remote_public_key, [3; 32]);
}

#[test]
fn silent_close_turns_the_report_off_and_keeps_the_key() {
    let conn = DhtConnection::new([4; 32], stream()).silent_close();

    assert!(!conn.reports_close_on_drop());
    assert_eq!(conn.remote_public_key, [4; 32]);
}
