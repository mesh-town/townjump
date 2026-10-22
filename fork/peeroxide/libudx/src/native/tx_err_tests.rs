// Tests of send_counted: success delivers bytes and grows TX_OK and TX_OK_BYTES, 70,000 B (above the UDP limit) return false
// and grow TX_ERR and TX_ERR_BYTES by the whole buffer length,
// should_report is a pure "time to write" decision. The counters are process-wide (tests run in parallel), so
// "grew by at least" inequalities are checked, not exact differences.
use super::*;

async fn pair() -> (UdpSocket, UdpSocket) {
    let a = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let b = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    (a, b)
}

#[tokio::test]
async fn success_returns_true_counts_ok_and_bytes_arrive() {
    let (a, b) = pair().await;
    let before = tx_counters();
    let before_b = tx_bytes();
    assert!(send_counted(&a, b"hello", b.local_addr().unwrap(), "stream").await);
    let after = tx_counters();
    assert!(after.0 > before.0);
    assert!(tx_bytes().0 >= before_b.0 + 5);
    let mut buf = [0u8; 16];
    let (n, from) = b.recv_from(&mut buf).await.unwrap();
    assert_eq!((&buf[..n], from), (&b"hello"[..], a.local_addr().unwrap()));
}

#[tokio::test]
async fn oversize_datagram_returns_false_and_counts_err() {
    let (a, b) = pair().await;
    let big = vec![0u8; 70_000];
    // The system error text — for the report (run with --nocapture).
    eprintln!(
        "raw send_to error: {:?}",
        a.send_to(&big, b.local_addr().unwrap()).await
    );
    let before = tx_counters();
    let before_b = tx_bytes();
    assert!(!send_counted(&a, &big, b.local_addr().unwrap(), "stream").await);
    assert!(tx_counters().1 > before.1);
    // Dropped bytes are the whole buffer length: the kernel took nothing.
    assert!(tx_bytes().1 >= before_b.1 + 70_000);
}

#[test]
fn should_report_first_then_throttled() {
    assert!(should_report(1_000, 0));
    assert!(!should_report(2_000, 1_000));
    assert!(!should_report(5_999, 1_000));
    assert!(should_report(6_000, 1_000));
}
