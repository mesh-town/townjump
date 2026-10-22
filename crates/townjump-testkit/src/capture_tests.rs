use super::*;
use std::time::Duration;

#[test]
fn write_appends_and_text_bytes() {
    let mut cap = Capture::new();
    cap.write_all(b"hello ").unwrap();
    cap.write_all(b"world").unwrap();
    cap.flush().unwrap();
    assert_eq!(cap.text(), "hello world");
    assert_eq!(cap.bytes(), b"hello world");
}

#[test]
fn clones_share_buffer() {
    let mut a = Capture::new();
    let b = a.clone();
    a.write_all(b"x").unwrap();
    assert_eq!(b.text(), "x");
}

#[test]
fn shared_out_writes_land_in_capture() {
    let cap = Capture::new();
    let out = cap.shared();
    out.lock().unwrap().write_all(b"via logger").unwrap();
    assert_eq!(cap.text(), "via logger");
    assert!(cap.last_write().is_some());
}

#[test]
fn last_write_updates_on_write() {
    let mut cap = Capture::new();
    assert!(cap.last_write().is_none());
    cap.write_all(b"a").unwrap();
    let first = cap.last_write().unwrap();
    std::thread::sleep(Duration::from_millis(5));
    cap.write_all(b"b").unwrap();
    assert!(cap.last_write().unwrap() > first);
}

#[test]
fn text_is_lossy_bytes_are_raw() {
    let mut cap = Capture::new();
    cap.write_all(&[b'a', 0xff, b'b']).unwrap();
    assert_eq!(cap.bytes(), vec![b'a', 0xff, b'b']);
    assert_eq!(cap.text(), "a\u{fffd}b");
}

#[test]
fn fixed_random_fills_prefix() {
    let mut fill = fixed_random();
    let mut long = [0xAAu8; 32];
    fill(&mut long);
    assert_eq!(long.to_vec(), (0u8..32).collect::<Vec<_>>());
    let mut short = [0xAAu8; 8];
    fill(&mut short);
    assert_eq!(short.to_vec(), (0u8..8).collect::<Vec<_>>());
}

#[test]
fn fixed_random_restarts_and_wraps() {
    let mut fill = fixed_random();
    let (mut first, mut second) = ([0u8; 32], [0u8; 32]);
    fill(&mut first);
    fill(&mut second);
    assert_eq!(first, second);
    let mut long = [0u8; 40];
    fill(&mut long);
    let expected: Vec<u8> = (0..40).map(|i| (i % 32) as u8).collect();
    assert_eq!(long.to_vec(), expected);
}

#[test]
fn fixed_random_matches_mock_random_hex() {
    let mut buf = [0u8; 32];
    fixed_random()(&mut buf);
    // the RND line from testvectors/holesail-cli/run-matrix.mjs
    let rnd = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
    assert_eq!(hex::encode(buf), rnd);
}
