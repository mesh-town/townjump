// JS: node/lib/_http_common + llhttp (request line and header parsing).
// Node: maxHeaderSize 16 KiB -> 431; http.METHODS methods; duplicate headers per _addHeaderLine.
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt};

/// Node 24 header limit (`--max-http-header-size`).
const MAX_HEAD: usize = 16 * 1024;

const METHODS: &[&str] = &[
    "ACL",
    "BIND",
    "CHECKOUT",
    "CONNECT",
    "COPY",
    "DELETE",
    "GET",
    "HEAD",
    "LINK",
    "LOCK",
    "M-SEARCH",
    "MERGE",
    "MKACTIVITY",
    "MKCALENDAR",
    "MKCOL",
    "MOVE",
    "NOTIFY",
    "OPTIONS",
    "PATCH",
    "POST",
    "PROPFIND",
    "PROPPATCH",
    "PURGE",
    "PUT",
    "QUERY",
    "REBIND",
    "REPORT",
    "SEARCH",
    "SOURCE",
    "SUBSCRIBE",
    "TRACE",
    "UNBIND",
    "UNLINK",
    "UNLOCK",
    "UNSUBSCRIBE",
];

/// Headers: lowercase names, duplicates merged by Node's rules.
#[derive(Debug, Clone, Default)]
pub(crate) struct Headers(pub(crate) Vec<(String, String)>);

impl Headers {
    pub(crate) fn get(&self, name: &str) -> Option<&str> {
        let hit = self.0.iter().find(|(k, _)| k.eq_ignore_ascii_case(name));
        hit.map(|(_, v)| v.as_str())
    }

    /// Node `IncomingMessage._addHeaderLine`: first wins / merge.
    fn add(&mut self, name: String, value: String) {
        let Some(i) = self.0.iter().position(|(k, _)| *k == name) else {
            self.0.push((name, value));
            return;
        };
        if first_wins(&name) {
            return;
        }
        let slot = &mut self.0[i].1;
        slot.push_str(if name == "cookie" { "; " } else { ", " });
        slot.push_str(&value);
    }
}

fn first_wins(name: &str) -> bool {
    const LIST: [&str; 16] = [
        "authorization",
        "host",
        "content-type",
        "content-length",
        "user-agent",
        "referer",
        "from",
        "location",
        "max-forwards",
        "proxy-authorization",
        "etag",
        "last-modified",
        "server",
        "age",
        "expires",
        "retry-after",
    ];
    name.starts_with("if-") || LIST.contains(&name)
}

#[derive(Debug, Clone)]
pub(crate) struct RequestHead {
    pub method: String,
    /// Request target, bytes as latin1.
    pub target: String,
    /// 0 for HTTP/1.0, 1 for HTTP/1.1.
    pub minor: u8,
    pub headers: Headers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reject {
    BadRequest,
    HeaderTooLarge,
    Timeout,
    /// Drop without a response (CONNECT).
    Drop,
}

#[derive(Debug)]
pub(crate) enum HeadRead {
    Closed,
    Reject(Reject),
    Head(RequestHead),
}

/// `buf` — accumulator of the connection's raw bytes (the tail after the headers stays in it).
pub(crate) async fn read_head<R: AsyncRead + Unpin>(
    r: &mut R,
    buf: &mut Vec<u8>,
) -> io::Result<HeadRead> {
    loop {
        // llhttp skips empty lines before the request line.
        while buf.starts_with(b"\r\n") {
            buf.drain(..2);
        }
        if let Some(p) = find_end(buf) {
            if p + 4 > MAX_HEAD {
                return Ok(HeadRead::Reject(Reject::HeaderTooLarge));
            }
            let parsed = parse_head(&buf[..p]);
            buf.drain(..p + 4);
            return Ok(match parsed {
                Ok(h) => HeadRead::Head(h),
                Err(rj) => HeadRead::Reject(rj),
            });
        }
        if buf.len() > MAX_HEAD {
            return Ok(HeadRead::Reject(Reject::HeaderTooLarge));
        }
        let mut chunk = [0u8; 8192];
        let n = r.read(&mut chunk).await?;
        if n == 0 {
            return Ok(HeadRead::Closed);
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

fn find_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

/// Lines split by CRLF; the caller's parsing rejects lone CR/LF inside a line.
fn split_crlf(raw: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut start = 0;
    while let Some(i) = raw[start..].windows(2).position(|w| w == b"\r\n") {
        out.push(&raw[start..start + i]);
        start += i + 2;
    }
    out.push(&raw[start..]);
    out
}

fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

fn is_token(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

fn parse_head(raw: &[u8]) -> Result<RequestHead, Reject> {
    let mut lines = split_crlf(raw).into_iter();
    let first = lines.next().ok_or(Reject::BadRequest)?;
    let (method, target, minor) = parse_request_line(first)?;
    let mut headers = Headers::default();
    for line in lines {
        let (name, value) = parse_header_line(line)?;
        if name == "content-length" && headers.get(&name).is_some() {
            return Err(Reject::BadRequest);
        }
        headers.add(name, value);
    }
    check_framing(&headers)?;
    if minor == 1 && headers.get("host").is_none() {
        return Err(Reject::BadRequest);
    }
    Ok(RequestHead {
        method,
        target,
        minor,
        headers,
    })
}

fn parse_request_line(line: &[u8]) -> Result<(String, String, u8), Reject> {
    let parts: Vec<&[u8]> = line.split(|&b| b == b' ').collect();
    let [method, target, version] = parts[..] else {
        return Err(Reject::BadRequest);
    };
    let method = std::str::from_utf8(method).map_err(|_| Reject::BadRequest)?;
    if !METHODS.contains(&method) {
        return Err(Reject::BadRequest);
    }
    if method == "CONNECT" {
        return Err(Reject::Drop);
    }
    let minor = match version {
        b"HTTP/1.1" => 1,
        b"HTTP/1.0" => 0,
        _ => return Err(Reject::BadRequest),
    };
    if target.is_empty() || target.iter().any(|&b| b <= 0x20 || b == 0x7f) {
        return Err(Reject::BadRequest);
    }
    Ok((method.to_string(), latin1(target), minor))
}

fn parse_header_line(line: &[u8]) -> Result<(String, String), Reject> {
    let Some(colon) = line.iter().position(|&b| b == b':') else {
        return Err(Reject::BadRequest);
    };
    let (name, rest) = (&line[..colon], &line[colon + 1..]);
    if name.is_empty() || !name.iter().all(|&b| is_token(b)) {
        return Err(Reject::BadRequest);
    }
    let value = trim_sp_ht(rest);
    if value.iter().any(|&b| (b < 0x20 && b != b'\t') || b == 0x7f) {
        return Err(Reject::BadRequest);
    }
    Ok((latin1(name).to_ascii_lowercase(), latin1(value)))
}

/// Trims only SP and HTAB (not all ASCII whitespace).
fn trim_sp_ht(b: &[u8]) -> &[u8] {
    let is_ws = |c: &u8| *c == b' ' || *c == b'\t';
    let start = b.iter().position(|c| !is_ws(c)).unwrap_or(b.len());
    let end = b.iter().rposition(|c| !is_ws(c)).map_or(start, |i| i + 1);
    &b[start..end]
}

/// llhttp: CL digits only, CL together with TE is not allowed, TE must end with chunked.
fn check_framing(h: &Headers) -> Result<(), Reject> {
    let cl = h.get("content-length");
    let te = h.get("transfer-encoding");
    if let Some(v) = cl
        && (v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()) || v.parse::<u64>().is_err())
    {
        return Err(Reject::BadRequest);
    }
    let Some(te) = te else { return Ok(()) };
    let last = te
        .rsplit(',')
        .next()
        .unwrap_or("")
        .trim_matches([' ', '\t']);
    if cl.is_some() || !last.eq_ignore_ascii_case("chunked") {
        return Err(Reject::BadRequest);
    }
    Ok(())
}

#[cfg(test)]
#[path = "head_tests.rs"]
mod tests;
