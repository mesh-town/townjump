// Tests of the relay path label on `DhtConnection`: absent without a relay, live with one.
use super::*;
use crate::relay::RelayPath;
use tokio::sync::watch;

fn connection() -> DhtConnection {
    DhtConnection::new([1; 32], Box::new(tokio::io::duplex(8).0))
}

#[test]
fn connection_without_relay_has_no_path_and_is_not_relayed() {
    let conn = connection();

    assert!(conn.relay_path().is_none());
    assert!(!conn.is_relayed());
}

#[test]
fn connection_with_relay_path_follows_the_sender() {
    let (tx, rx) = watch::channel(RelayPath::Relayed);
    let conn = connection().with_relay_path(rx);

    let before = conn.is_relayed();
    tx.send(RelayPath::Direct).unwrap();
    let after = conn.is_relayed();

    assert!(before);
    assert!(!after);
    assert_eq!(*conn.relay_path().unwrap().borrow(), RelayPath::Direct);
}
