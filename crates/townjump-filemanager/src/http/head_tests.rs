use super::*;
use tokio::io::{AsyncWriteExt, duplex};

async fn read(input: &[u8]) -> (HeadRead, Vec<u8>) {
    let (mut a, mut b) = duplex(1 << 16);
    a.write_all(input).await.unwrap();
    drop(a);
    let mut buf = Vec::new();
    let r = read_head(&mut b, &mut buf).await.unwrap();
    (r, buf)
}

fn head(r: HeadRead) -> RequestHead {
    match r {
        HeadRead::Head(h) => h,
        other => panic!("expected Head, got {other:?}"),
    }
}

fn rejected(r: HeadRead) -> Reject {
    match r {
        HeadRead::Reject(x) => x,
        other => panic!("expected Reject, got {other:?}"),
    }
}

#[tokio::test]
async fn simple_get() {
    let (r, rest) = read(b"GET /a?b HTTP/1.1\r\nHost: x\r\n\r\nTAIL").await;
    let h = head(r);
    assert_eq!(
        (h.method.as_str(), h.target.as_str(), h.minor),
        ("GET", "/a?b", 1)
    );
    assert_eq!(h.headers.get("HOST"), Some("x"));
    assert_eq!(rest, b"TAIL");
}

#[tokio::test]
async fn bad_methods() {
    for m in ["get", "FOO"] {
        let (r, _) = read(format!("{m} / HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes()).await;
        assert_eq!(rejected(r), Reject::BadRequest, "{m}");
    }
}

#[tokio::test]
async fn connect_is_dropped() {
    let (r, _) = read(b"CONNECT a:1 HTTP/1.1\r\nHost: a:1\r\n\r\n").await;
    assert_eq!(rejected(r), Reject::Drop);
}

#[tokio::test]
async fn host_required_only_in_11() {
    let (r, _) = read(b"GET / HTTP/1.1\r\n\r\n").await;
    assert_eq!(rejected(r), Reject::BadRequest);
    let (r, _) = read(b"GET / HTTP/1.0\r\n\r\n").await;
    assert_eq!(head(r).minor, 0);
}

#[tokio::test]
async fn header_too_large() {
    let big = format!(
        "GET / HTTP/1.1\r\nHost: x\r\nX: {}\r\n\r\n",
        "a".repeat(17 * 1024)
    );
    let (r, _) = read(big.as_bytes()).await;
    assert_eq!(rejected(r), Reject::HeaderTooLarge);
    // without a terminator too
    let (r, _) = read("a".repeat(17 * 1024).as_bytes()).await;
    assert_eq!(rejected(r), Reject::HeaderTooLarge);
}

#[tokio::test]
async fn duplicates_and_trim() {
    let raw = b"GET / HTTP/1.1\r\nHost: x\r\nAuthorization: one\r\nauthorization: two\r\n\
Range: a\r\nRANGE: b\r\nX-Pad:  Basic  \t\r\n\r\n";
    let h = head(read(raw).await.0);
    assert_eq!(h.headers.get("authorization"), Some("one"));
    assert_eq!(h.headers.get("range"), Some("a, b"));
    assert_eq!(h.headers.get("x-pad"), Some("Basic"));
}

#[tokio::test]
async fn eof_is_closed() {
    assert!(matches!(read(b"").await.0, HeadRead::Closed));
    assert!(matches!(read(b"GET / HT").await.0, HeadRead::Closed));
}

#[tokio::test]
async fn leading_crlf_skipped() {
    let h = head(read(b"\r\n\r\nGET / HTTP/1.1\r\nHost: x\r\n\r\n").await.0);
    assert_eq!(h.method, "GET");
}

#[tokio::test]
async fn smuggling_shapes_rejected() {
    let cases: [&[u8]; 6] = [
        b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 1\r\nContent-Length: 1\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 1\r\nTransfer-Encoding: chunked\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: -1\r\n\r\n",
        b"GET / HTTP/1.1\r\nHost: x\r\nA : b\r\n\r\n",
        b"GET / HTTP/1.1\r\nHost: x\r\n folded\r\n\r\n",
        b"GET / HTTP/2.0\r\nHost: x\r\n\r\n",
    ];
    for c in cases {
        let (r, _) = read(c).await;
        assert_eq!(
            rejected(r),
            Reject::BadRequest,
            "{}",
            String::from_utf8_lossy(c)
        );
    }
}

#[tokio::test]
async fn high_bytes_in_target_are_latin1() {
    let h = head(read(b"GET /\xd1\x84 HTTP/1.1\r\nHost: x\r\n\r\n").await.0);
    assert_eq!(h.target, "/\u{d1}\u{84}");
}

#[test]
fn get_is_case_insensitive_on_query() {
    let h = Headers(vec![("range".into(), "a".into())]);
    assert_eq!(h.get("Range"), Some("a"));
    assert_eq!(h.get("nope"), None);
}
