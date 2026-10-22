// JS: node/lib/_http_incoming + llhttp (Content-Length, chunked).
use super::head::Headers;
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt};

/// Limit for a chunk-size or trailer line (llhttp cuts earlier; this is memory protection).
const MAX_LINE: usize = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Framing {
    None,
    Length(u64),
    Chunked,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BodyRead {
    Body(Vec<u8>),
    TooLarge,
}

/// Headers were already checked by `read_head` (CL is digits, TE ends with chunked).
pub(crate) fn framing(headers: &Headers) -> Framing {
    if headers.get("transfer-encoding").is_some() {
        return Framing::Chunked;
    }
    match headers
        .get("content-length")
        .and_then(|v| v.parse::<u64>().ok())
    {
        Some(n) if n > 0 => Framing::Length(n),
        _ => Framing::None,
    }
}

/// Whole body into memory; more than `limit` bytes -> `TooLarge` (the rest is not read).
/// As in JS (`totalSize > maxRequestSize` in `data`), the decision is made on bytes received, not on
/// `Content-Length`: a 10 MiB + 1 byte request gets 413 only when the last byte is received (JS reference behavior).
pub(crate) async fn read_body<R: AsyncRead + Unpin>(
    r: &mut R,
    buf: &mut Vec<u8>,
    framing: Framing,
    limit: usize,
) -> io::Result<BodyRead> {
    let mut body = Vec::new();
    let mut too_large = false;
    consume(r, buf, framing, |part| {
        if body.len() + part.len() > limit {
            too_large = true;
            return false;
        }
        body.extend_from_slice(part);
        true
    })
    .await?;
    Ok(if too_large {
        BodyRead::TooLarge
    } else {
        BodyRead::Body(body)
    })
}

/// Read and discard the body (nothing accumulates in memory).
pub(crate) async fn drain_body<R: AsyncRead + Unpin>(
    r: &mut R,
    buf: &mut Vec<u8>,
    framing: Framing,
) -> io::Result<()> {
    consume(r, buf, framing, |_| true).await
}

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

/// Read more bytes into `buf`; end of stream in the middle of the body is an error.
async fn fill<R: AsyncRead + Unpin>(r: &mut R, buf: &mut Vec<u8>) -> io::Result<()> {
    let mut chunk = [0u8; 16384];
    let n = r.read(&mut chunk).await?;
    if n == 0 {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    buf.extend_from_slice(&chunk[..n]);
    Ok(())
}

/// Line up to CRLF (excluding it), removed from the start of `buf`.
async fn read_line<R: AsyncRead + Unpin>(r: &mut R, buf: &mut Vec<u8>) -> io::Result<Vec<u8>> {
    loop {
        if let Some(i) = buf.windows(2).position(|w| w == b"\r\n") {
            let line = buf[..i].to_vec();
            buf.drain(..i + 2);
            return Ok(line);
        }
        if buf.len() > MAX_LINE {
            return Err(invalid("chunked line too long"));
        }
        fill(r, buf).await?;
    }
}

/// Passes body chunks to `sink`; `false` from `sink` means stop (the caller knows why).
async fn consume<R, F>(
    r: &mut R,
    buf: &mut Vec<u8>,
    framing: Framing,
    mut sink: F,
) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    F: FnMut(&[u8]) -> bool,
{
    match framing {
        Framing::None => Ok(()),
        Framing::Length(n) => take_exact(r, buf, n, &mut sink).await.map(|_| ()),
        Framing::Chunked => consume_chunked(r, buf, &mut sink).await,
    }
}

/// `false` — `sink` stopped the read.
async fn take_exact<R, F>(
    r: &mut R,
    buf: &mut Vec<u8>,
    mut left: u64,
    sink: &mut F,
) -> io::Result<bool>
where
    R: AsyncRead + Unpin,
    F: FnMut(&[u8]) -> bool,
{
    while left > 0 {
        if buf.is_empty() {
            fill(r, buf).await?;
        }
        let k = buf.len().min(usize::try_from(left).unwrap_or(usize::MAX));
        let keep_going = sink(&buf[..k]);
        buf.drain(..k);
        left -= k as u64;
        if !keep_going {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn consume_chunked<R, F>(r: &mut R, buf: &mut Vec<u8>, sink: &mut F) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    F: FnMut(&[u8]) -> bool,
{
    loop {
        let line = read_line(r, buf).await?;
        let size = parse_chunk_size(&line)?;
        if size == 0 {
            // trailers up to the empty line
            while !read_line(r, buf).await?.is_empty() {}
            return Ok(());
        }
        if !take_exact(r, buf, size, sink).await? {
            return Ok(());
        }
        if !read_line(r, buf).await?.is_empty() {
            return Err(invalid("no CRLF after chunk"));
        }
    }
}

fn parse_chunk_size(line: &[u8]) -> io::Result<u64> {
    let hex = line.split(|&b| b == b';').next().unwrap_or(&[]);
    let hex = std::str::from_utf8(hex)
        .map_err(|_| invalid("chunk size"))?
        .trim();
    if hex.is_empty() || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("chunk size"));
    }
    u64::from_str_radix(hex, 16).map_err(|_| invalid("chunk size"))
}

#[cfg(test)]
#[path = "body_tests.rs"]
mod tests;
