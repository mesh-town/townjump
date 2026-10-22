use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};

fn resp(status: u16, headers: &[(&str, &str)], body: Body) -> Response {
    let headers = headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    Response {
        status,
        reason: None,
        headers,
        body,
    }
}

/// Date: <anything> -> Date: X
fn mask_date(raw: &[u8]) -> String {
    let s = raw.iter().map(|&b| b as char).collect::<String>();
    let i = s.find("Date: ").expect("no Date");
    let j = i + s[i..].find("\r\n").unwrap();
    format!("{}Date: X{}", &s[..i], &s[j..])
}

async fn run(r: Response, keep: bool, http10: bool) -> (Result<bool, WriteError>, Vec<u8>) {
    let (mut a, mut b) = duplex(1 << 20);
    let res = write_response(&mut a, r, keep, http10).await;
    drop(a);
    let mut out = Vec::new();
    b.read_to_end(&mut out).await.unwrap();
    (res, out)
}

#[tokio::test]
async fn unauthorized_chunked_exact() {
    let r = resp(
        401,
        &[("WWW-Authenticate", "Basic realm=\"Filemanager\"")],
        Body::Bytes(b"Authentication required.".to_vec()),
    );
    let (res, out) = run(r, true, false).await;
    assert!(res.unwrap());
    assert_eq!(
        mask_date(&out),
        "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"Filemanager\"\r\n\
Date: X\r\nConnection: keep-alive\r\nKeep-Alive: timeout=5\r\nTransfer-Encoding: chunked\r\n\r\n\
18\r\nAuthentication required.\r\n0\r\n\r\n"
    );
}

#[tokio::test]
async fn invalid_header_writes_nothing() {
    let r = resp(
        200,
        &[("Content-Disposition", "φαηλ")],
        Body::Bytes(b"x".to_vec()),
    );
    let (res, out) = run(r, true, false).await;
    assert!(matches!(res, Err(WriteError::InvalidHeader(n)) if n == "Content-Disposition"));
    assert!(out.is_empty());
}

#[tokio::test]
async fn content_length_close() {
    let r = resp(200, &[("Content-Length", "2")], Body::Bytes(b"ok".to_vec()));
    let (res, out) = run(r, false, false).await;
    assert!(!res.unwrap());
    assert_eq!(
        mask_date(&out),
        "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nDate: X\r\nConnection: close\r\n\r\nok"
    );
}

#[tokio::test]
async fn http10_without_length_closes_unchunked() {
    let r = resp(200, &[], Body::Bytes(b"hi".to_vec()));
    let (res, out) = run(r, true, true).await;
    assert!(!res.unwrap());
    assert_eq!(
        mask_date(&out),
        "HTTP/1.1 200 OK\r\nDate: X\r\nConnection: close\r\n\r\nhi"
    );
}

#[tokio::test]
async fn http10_keep_alive_with_length() {
    let r = resp(200, &[("Content-Length", "0")], Body::Empty);
    let (res, out) = run(r, true, true).await;
    assert!(res.unwrap());
    assert!(mask_date(&out).contains("Connection: keep-alive\r\nKeep-Alive: timeout=5\r\n"));
}

#[tokio::test]
async fn empty_body_chunked_is_terminator_only() {
    let (_, out) = run(resp(417, &[], Body::Empty), true, false).await;
    assert!(mask_date(&out).ends_with("Transfer-Encoding: chunked\r\n\r\n0\r\n\r\n"));
    assert!(out.starts_with(b"HTTP/1.1 417 Expectation Failed\r\n"));
}

#[tokio::test]
async fn file_body_with_length_and_chunked() {
    let path = std::env::temp_dir().join(format!("fm-resp-{}.tmp", std::process::id()));
    let mut f = tokio::fs::File::create(&path).await.unwrap();
    f.write_all(b"0123456789").await.unwrap();
    drop(f);
    let open = || async { tokio::fs::File::open(&path).await.unwrap() };
    let r = resp(
        200,
        &[("Content-Length", "4")],
        Body::File {
            file: open().await,
            len: 4,
        },
    );
    let (_, out) = run(r, true, false).await;
    assert!(out.ends_with(b"\r\n\r\n0123"));
    let r = resp(
        200,
        &[],
        Body::File {
            file: open().await,
            len: 10,
        },
    );
    let (_, out) = run(r, true, false).await;
    assert!(out.ends_with(b"\r\n\r\na\r\n0123456789\r\n0\r\n\r\n"));
    let r = resp(
        200,
        &[("Content-Length", "20")],
        Body::File {
            file: open().await,
            len: 20,
        },
    );
    let (res, _) = run(r, true, false).await;
    assert!(matches!(res, Err(WriteError::Io(_))));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn reject_literals() {
    assert_eq!(
        reject_bytes(Reject::BadRequest),
        b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n"
    );
    assert_eq!(
        reject_bytes(Reject::HeaderTooLarge),
        b"HTTP/1.1 431 Request Header Fields Too Large\r\nConnection: close\r\n\r\n"
    );
    assert_eq!(
        reject_bytes(Reject::Timeout),
        b"HTTP/1.1 408 Request Timeout\r\nConnection: close\r\n\r\n"
    );
    assert_eq!(reason(416), "Range Not Satisfiable");
}

#[tokio::test]
async fn head_only_has_no_body_no_chunked() {
    let (mut a, mut b) = duplex(1 << 20);
    let r = resp(
        401,
        &[("WWW-Authenticate", "x")],
        Body::Bytes(b"abc".to_vec()),
    );
    let res = write_response_for(&mut a, r, true, false, true).await;
    drop(a);
    let mut out = Vec::new();
    b.read_to_end(&mut out).await.unwrap();
    assert!(matches!(res, Ok(true)));
    assert_eq!(
        mask_date(&out),
        "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: x\r\nDate: X\r\nConnection: keep-alive\r\nKeep-Alive: timeout=5\r\n\r\n"
    );
}

#[tokio::test]
async fn custom_reason_replaces_standard() {
    let mut r = resp(500, &[], Body::Empty);
    r.reason = Some("Found");
    let (_, out) = run(r, true, false).await;
    assert!(out.starts_with(b"HTTP/1.1 500 Found\r\n"));
}
