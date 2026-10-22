// Tests of the receive helper: which errors are send echoes, when the pause starts, a plain datagram passes.
use super::*;
use std::sync::{Arc, Mutex};
use townjump_logger::SharedOut;

fn error_of(kind: io::ErrorKind) -> io::Error {
    io::Error::from(kind)
}

#[test]
fn reset_and_aborted_are_echoes() {
    assert!(is_send_echo(&error_of(io::ErrorKind::ConnectionReset)));
    assert!(is_send_echo(&error_of(io::ErrorKind::ConnectionAborted)));
}

#[test]
fn other_errors_are_not_echoes() {
    assert!(!is_send_echo(&error_of(io::ErrorKind::WouldBlock)));
    assert!(!is_send_echo(&error_of(io::ErrorKind::ConnectionRefused)));
    assert!(!is_send_echo(&io::Error::other("x")));
}

#[cfg(windows)]
#[test]
fn windows_raw_codes() {
    for code in [10054, 10053, 10052] {
        assert!(is_send_echo(&io::Error::from_raw_os_error(code)), "{code}");
    }
    assert!(!is_send_echo(&io::Error::from_raw_os_error(10040)));
}

#[test]
fn pause_starts_at_the_threshold() {
    assert_eq!(pause_after(0), None);
    assert_eq!(pause_after(SKIPS_BEFORE_PAUSE - 1), None);
    assert_eq!(pause_after(SKIPS_BEFORE_PAUSE), Some(SKIP_PAUSE));
    assert_eq!(pause_after(u32::MAX), Some(SKIP_PAUSE));
}

#[tokio::test]
async fn recv_next_returns_a_plain_datagram() {
    let receiver = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    sender
        .send_to(b"abc", receiver.local_addr().unwrap())
        .await
        .unwrap();
    let out: SharedOut = Arc::new(Mutex::new(io::sink()));
    let logger = Logger::new(Some(0.0), out.clone(), out);
    let mut buf = vec![0u8; RECV_BUF];
    let (n, from) = tokio::time::timeout(
        Duration::from_secs(5),
        recv_next(&receiver, &mut buf, &logger),
    )
    .await
    .expect("datagram within 5 s")
    .unwrap();
    assert_eq!(&buf[..n], b"abc");
    assert_eq!(from, sender.local_addr().unwrap());
}

#[tokio::test]
async fn recv_next_cuts_long_datagram_to_limit() {
    let receiver = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let data: Vec<u8> = (0..4000u32).map(|i| (i % 251) as u8).collect();
    sender
        .send_to(&data, receiver.local_addr().unwrap())
        .await
        .unwrap();
    let out: SharedOut = Arc::new(Mutex::new(io::sink()));
    let logger = Logger::new(Some(0.0), out.clone(), out);
    let mut buf = vec![0u8; RECV_BUF];
    let (n, from) = tokio::time::timeout(
        Duration::from_secs(5),
        recv_next(&receiver, &mut buf, &logger),
    )
    .await
    .expect("datagram within 5 s")
    .unwrap();
    assert_eq!(n, DATAGRAM_LIMIT);
    assert_eq!(&buf[..n], &data[..DATAGRAM_LIMIT]);
    assert_eq!(from, sender.local_addr().unwrap());
}
