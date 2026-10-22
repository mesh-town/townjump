// JS: node/lib/_http_outgoing (writeHead, chunked, keepalive, header character check).
// Node: after writeHead + end(body) without Content-Length -> chunked in one piece; HTTP/1.0 -> close.
use super::date::http_date;
use super::head::Reject;
use std::time::SystemTime;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Response to `Expect: 100-continue` (Node `checkContinue` by default).
pub(crate) const CONTINUE: &[u8] = b"HTTP/1.1 100 Continue\r\n\r\n";

pub(crate) enum Body {
    Empty,
    Bytes(Vec<u8>),
    /// Open file, send `len` bytes from the current position.
    File {
        file: tokio::fs::File,
        len: u64,
    },
}

pub(crate) struct Response {
    pub status: u16,
    /// Custom reason in the status line (Node: `statusMessage` from an interrupted `writeHead(302)` -> `500 Found`).
    pub reason: Option<&'static str>,
    /// Headers in insertion order.
    pub headers: Vec<(String, String)>,
    pub body: Body,
}

#[derive(Debug)]
pub(crate) enum WriteError {
    InvalidHeader(String),
    Io(std::io::Error),
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::InvalidHeader(name) => write!(f, "invalid response header: {name}"),
            WriteError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl From<std::io::Error> for WriteError {
    fn from(e: std::io::Error) -> Self {
        WriteError::Io(e)
    }
}

/// `http10` — an HTTP/1.0 request; `keep_alive` — keep the connection.
/// Ok(true) — the connection stays open (HTTP/1.0 without Content-Length always closes).
pub(crate) async fn write_response<W: AsyncWrite + Unpin>(
    w: &mut W,
    resp: Response,
    keep_alive: bool,
    http10: bool,
) -> Result<bool, WriteError> {
    write_response_for(w, resp, keep_alive, http10, false).await
}

/// `head_only` — response to HEAD: Node (`_hasBody = false`) sends no body, no chunked, no `close` for HTTP/1.0.
pub(crate) async fn write_response_for<W: AsyncWrite + Unpin>(
    w: &mut W,
    resp: Response,
    keep_alive: bool,
    http10: bool,
    head_only: bool,
) -> Result<bool, WriteError> {
    check_headers(&resp.headers)?;
    let has_len = resp
        .headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("content-length"));
    let chunked = !has_len && !http10 && !head_only;
    let keep = keep_alive && (has_len || !http10 || head_only);
    let head = build_head(&resp, keep, chunked);
    w.write_all(&head).await?;
    if head_only {
        w.flush().await?;
        return Ok(keep);
    }
    match resp.body {
        Body::Empty => finish_bytes(w, &[], chunked).await?,
        Body::Bytes(b) => finish_bytes(w, &b, chunked).await?,
        Body::File { mut file, len } => send_file(w, &mut file, len, chunked).await?,
    }
    w.flush().await?;
    Ok(keep)
}

fn check_headers(headers: &[(String, String)]) -> Result<(), WriteError> {
    for (k, v) in headers {
        let name_ok = !k.is_empty() && k.bytes().all(is_token);
        let value_ok = v
            .chars()
            .all(|c| matches!(c, '\t' | ' '..='~' | '\u{80}'..='\u{ff}'));
        if !name_ok || !value_ok {
            return Err(WriteError::InvalidHeader(k.clone()));
        }
    }
    Ok(())
}

fn is_token(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

/// Response head: latin1 values are written as bytes 0x80..0xff.
fn build_head(resp: &Response, keep: bool, chunked: bool) -> Vec<u8> {
    let why = resp.reason.unwrap_or_else(|| reason(resp.status));
    let mut s = format!("HTTP/1.1 {} {}\r\n", resp.status, why);
    for (k, v) in &resp.headers {
        s.push_str(&format!("{k}: {v}\r\n"));
    }
    s.push_str(&format!("Date: {}\r\n", http_date(SystemTime::now())));
    if keep {
        s.push_str("Connection: keep-alive\r\nKeep-Alive: timeout=5\r\n");
    } else {
        s.push_str("Connection: close\r\n");
    }
    if chunked {
        s.push_str("Transfer-Encoding: chunked\r\n");
    }
    s.push_str("\r\n");
    s.chars().map(|c| c as u32 as u8).collect()
}

async fn finish_bytes<W: AsyncWrite + Unpin>(
    w: &mut W,
    data: &[u8],
    chunked: bool,
) -> std::io::Result<()> {
    if !chunked {
        return w.write_all(data).await;
    }
    if !data.is_empty() {
        w.write_all(format!("{:x}\r\n", data.len()).as_bytes())
            .await?;
        w.write_all(data).await?;
        w.write_all(b"\r\n").await?;
    }
    w.write_all(b"0\r\n\r\n").await
}

async fn send_file<W: AsyncWrite + Unpin, R: AsyncRead + Unpin>(
    w: &mut W,
    file: &mut R,
    len: u64,
    chunked: bool,
) -> std::io::Result<()> {
    let mut left = len;
    let mut buf = vec![0u8; 64 * 1024];
    while left > 0 {
        let want = buf.len().min(usize::try_from(left).unwrap_or(usize::MAX));
        let n = file.read(&mut buf[..want]).await?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        left -= n as u64;
        if chunked {
            w.write_all(format!("{n:x}\r\n").as_bytes()).await?;
        }
        w.write_all(&buf[..n]).await?;
        if chunked {
            w.write_all(b"\r\n").await?;
        }
    }
    if chunked {
        w.write_all(b"0\r\n\r\n").await?;
    }
    Ok(())
}

pub(crate) fn reject_bytes(r: Reject) -> &'static [u8] {
    match r {
        Reject::HeaderTooLarge => {
            b"HTTP/1.1 431 Request Header Fields Too Large\r\nConnection: close\r\n\r\n"
        }
        Reject::Timeout => b"HTTP/1.1 408 Request Timeout\r\nConnection: close\r\n\r\n",
        // Drop tears down without a response; for a caller that does not parse it, same as a 400.
        Reject::BadRequest | Reject::Drop => {
            b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n"
        }
    }
}

pub(crate) fn reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        206 => "Partial Content",
        302 => "Found",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        413 => "Payload Too Large",
        416 => "Range Not Satisfiable",
        417 => "Expectation Failed",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        _ => "Unknown",
    }
}

#[cfg(test)]
#[path = "response_tests.rs"]
mod tests;
