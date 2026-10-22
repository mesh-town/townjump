//! Wire: raw TCP client, HTTP/1.1 request and response parsing (no dependency on the server).
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

/// Ceiling for waiting for a response or socket close.
const IO_TIMEOUT: Duration = Duration::from_secs(4);

/// Sends `bytes`, reads until the server closes the socket. An abort (RST) is end of data; if the server does not close the socket within
/// 4 s — panic (hence requests carry `Connection: close`).
pub async fn raw(addr: SocketAddr, bytes: &[u8]) -> Vec<u8> {
    let (mut rd, mut wr) = TcpStream::connect(addr)
        .await
        .expect("connect")
        .into_split();
    // write and read go together: the server may answer and close without reading a large body
    let send = async {
        let _ = wr.write_all(bytes).await;
    };
    let recv = async {
        let mut got = Vec::new();
        let _ = rd.read_to_end(&mut got).await;
        got
    };
    let both = async { tokio::join!(send, recv).1 };
    timeout(IO_TIMEOUT, both)
        .await
        .expect("server did not close the connection within 4 s")
}

/// HTTP/1.1 request; `Connection: close` unless `headers` contains a `Connection` header.
pub fn request(method: &str, target: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let mut req = format!("{method} {target} HTTP/1.1\r\nHost: localhost\r\n");
    if !headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("connection"))
    {
        req.push_str("Connection: close\r\n");
    }
    for (name, value) in headers {
        req.push_str(&format!("{name}: {value}\r\n"));
    }
    if method == "POST" {
        let len = body.len();
        req.push_str(&format!(
            "Content-Type: application/x-www-form-urlencoded\r\nContent-Length: {len}\r\n"
        ));
    }
    let mut bytes = (req + "\r\n").into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

/// Server response parsed from bytes.
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn header(&self, name: &str) -> Option<&str> {
        let found = self
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name));
        found.map(|(_, v)| v.as_str())
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// One response from the start of `buf` and the number of consumed bytes; `None` — the response is not complete yet.
    /// Without length and without chunked the body is everything up to EOF (`eof`).
    pub fn parse(buf: &[u8], eof: bool) -> Option<(Reply, usize)> {
        let head_end = find(buf, b"\r\n\r\n")? + 4;
        let head = String::from_utf8_lossy(&buf[..head_end - 4]).into_owned();
        let mut lines = head.split("\r\n");
        let status = lines.next()?.split(' ').nth(1)?.parse::<u16>().ok()?;
        let headers: Vec<(String, String)> = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.to_string(), v.trim().to_string()))
            .collect();
        let mut reply = Reply {
            status,
            headers,
            body: Vec::new(),
        };
        let rest = &buf[head_end..];
        let chunked = reply.header("transfer-encoding") == Some("chunked");
        let len = reply
            .header("content-length")
            .and_then(|v| v.parse::<usize>().ok());
        let used = match (chunked, len) {
            (true, _) => {
                let (body, used) = dechunk(rest)?;
                reply.body = body;
                used
            }
            (false, Some(len)) => {
                reply.body = rest.get(..len)?.to_vec();
                len
            }
            (false, None) if eof => {
                reply.body = rest.to_vec();
                rest.len()
            }
            (false, None) => return None,
        };
        Some((reply, head_end + used))
    }

    /// The single complete response in `bytes` (after the socket closed); otherwise a panic with the content.
    pub fn from_bytes(bytes: &[u8]) -> Reply {
        match Reply::parse(bytes, true) {
            Some((reply, _)) => reply,
            None => panic!("response not parsed: {:?}", String::from_utf8_lossy(bytes)),
        }
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Chunked body: `(data, bytes consumed)`; `None` — the body is not finished.
fn dechunk(mut rest: &[u8]) -> Option<(Vec<u8>, usize)> {
    let total = rest.len();
    let mut body = Vec::new();
    loop {
        let eol = find(rest, b"\r\n")?;
        let size =
            usize::from_str_radix(std::str::from_utf8(&rest[..eol]).ok()?.trim(), 16).ok()?;
        rest = &rest[eol + 2..];
        if size == 0 {
            rest = rest.strip_prefix(b"\r\n")?;
            return Some((body, total - rest.len()));
        }
        body.extend_from_slice(rest.get(..size)?);
        rest = rest.get(size + 2..)?;
    }
}

/// One socket for several requests (keepalive, silence, close by the server).
pub struct Conn {
    stream: TcpStream,
    buf: Vec<u8>,
}

impl Conn {
    pub async fn open(addr: SocketAddr) -> Conn {
        let stream = TcpStream::connect(addr).await.expect("connect");
        Conn {
            stream,
            buf: Vec::new(),
        }
    }

    pub async fn send(&mut self, bytes: &[u8]) {
        self.stream.write_all(bytes).await.expect("write");
    }

    /// Next complete response; socket close or 4 s without a response — panic.
    pub async fn reply(&mut self) -> Reply {
        let next = async {
            loop {
                if let Some((reply, used)) = Reply::parse(&self.buf, false) {
                    self.buf.drain(..used);
                    return reply;
                }
                let mut chunk = [0u8; 4096];
                let n = self.stream.read(&mut chunk).await.expect("read");
                assert!(n > 0, "server closed the connection before a full response");
                self.buf.extend_from_slice(&chunk[..n]);
            }
        };
        timeout(IO_TIMEOUT, next)
            .await
            .expect("no response within 4 s")
    }

    /// Nothing arrived within `span` and the socket is not closed (EOF does not count as silence).
    pub async fn stays_silent(&mut self, span: Duration) -> bool {
        timeout(span, self.stream.read(&mut [0u8; 1]))
            .await
            .is_err()
    }

    /// Server closed the socket (EOF or RST) within `span`.
    pub async fn closed_within(&mut self, span: Duration) -> bool {
        let read = timeout(span, self.stream.read(&mut [0u8; 64])).await;
        matches!(read, Ok(Ok(0)) | Ok(Err(_)))
    }
}
