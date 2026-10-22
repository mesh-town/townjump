use super::*;
use tokio::io::{AsyncWriteExt, DuplexStream, duplex};

async fn feed(input: &[u8]) -> DuplexStream {
    let (mut a, b) = duplex(1 << 16);
    a.write_all(input).await.unwrap();
    drop(a);
    b
}

#[tokio::test]
async fn content_length_body() {
    let mut r = feed(b"abcEXTRA").await;
    let mut buf = Vec::new();
    let got = read_body(&mut r, &mut buf, Framing::Length(3), 100)
        .await
        .unwrap();
    assert_eq!(got, BodyRead::Body(b"abc".to_vec()));
    // the foreign tail is left for the next request
    let mut rest = buf.clone();
    let mut tail = Vec::new();
    r.read_to_end(&mut tail).await.unwrap();
    rest.extend(tail);
    assert_eq!(rest, b"EXTRA");
}

#[tokio::test]
async fn chunked_body() {
    let mut r = feed(b"3\r\nabc\r\n2;ext=1\r\nde\r\n0\r\nX-T: 1\r\n\r\n").await;
    let mut buf = Vec::new();
    let got = read_body(&mut r, &mut buf, Framing::Chunked, 100)
        .await
        .unwrap();
    assert_eq!(got, BodyRead::Body(b"abcde".to_vec()));
}

#[tokio::test]
async fn too_large_by_length_and_by_chunks() {
    let mut r = feed(b"0123456789AB").await;
    let got = read_body(&mut r, &mut Vec::new(), Framing::Length(12), 10)
        .await
        .unwrap();
    assert_eq!(got, BodyRead::TooLarge);
    let mut r = feed(b"8\r\n12345678\r\n8\r\n12345678\r\n0\r\n\r\n").await;
    let got = read_body(&mut r, &mut Vec::new(), Framing::Chunked, 10)
        .await
        .unwrap();
    assert_eq!(got, BodyRead::TooLarge);
}

#[tokio::test]
async fn no_framing_is_empty() {
    let mut r = feed(b"ignored").await;
    let got = read_body(&mut r, &mut Vec::new(), Framing::None, 10)
        .await
        .unwrap();
    assert_eq!(got, BodyRead::Body(Vec::new()));
}

#[tokio::test]
async fn truncated_body_is_error() {
    let mut r = feed(b"ab").await;
    let err = read_body(&mut r, &mut Vec::new(), Framing::Length(5), 10)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    let mut r = feed(b"zz\r\n").await;
    let err = read_body(&mut r, &mut Vec::new(), Framing::Chunked, 10)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[tokio::test]
async fn drain_discards_and_keeps_tail() {
    let mut r = feed(b"3\r\nabc\r\n0\r\n\r\nNEXT").await;
    let mut buf = Vec::new();
    drain_body(&mut r, &mut buf, Framing::Chunked)
        .await
        .unwrap();
    let mut tail = buf.clone();
    r.read_to_end(&mut tail).await.unwrap();
    assert_eq!(tail, b"NEXT");
}

#[test]
fn framing_from_headers() {
    let h = |k: &str, v: &str| Headers(vec![(k.into(), v.into())]);
    assert_eq!(framing(&h("content-length", "7")), Framing::Length(7));
    assert_eq!(framing(&h("content-length", "0")), Framing::None);
    assert_eq!(
        framing(&h("transfer-encoding", "chunked")),
        Framing::Chunked
    );
    assert_eq!(framing(&Headers::default()), Framing::None);
}
