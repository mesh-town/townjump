// JS: node/lib/_http_server (connection loop, timeouts) and livefiles/index.js:createServer/handleRequest.
use crate::http::body::{BodyRead, Framing, drain_body, framing, read_body};
use crate::http::head::{HeadRead, Reject, RequestHead, read_head};
use crate::http::response::{
    Body, CONTINUE, Response, reject_bytes, write_response, write_response_for,
};
use crate::opts::Config;
use crate::route::{Outcome, Step, admit, get::handle_get, post::handle_post, safe};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::time::timeout;

pub(crate) const HEADERS_TIMEOUT: Duration = Duration::from_secs(60);
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
pub(crate) const KEEP_ALIVE: Duration = Duration::from_secs(5);
/// livefiles maxRequestSize: 10 MiB.
const MAX_BODY: usize = 10 * 1024 * 1024;

struct Conn {
    rd: OwnedReadHalf,
    wr: OwnedWriteHalf,
    buf: Vec<u8>,
    cfg: Arc<Config>,
}

pub(crate) async fn serve_conn(stream: TcpStream, cfg: Arc<Config>) {
    let _ = stream.set_nodelay(true);
    let (rd, wr) = stream.into_split();
    let mut conn = Conn {
        rd,
        wr,
        buf: Vec::new(),
        cfg,
    };
    // Node: headersTimeout counts from connect — one deadline for peek and the first request's headers.
    let mut first = Some(Instant::now() + HEADERS_TIMEOUT);
    while conn.next_request(first.take()).await {}
    let _ = conn.wr.shutdown().await;
}

/// Decision point: a JS crash -> `on_fatal` call; under `--safe-mode` a 400 (`safe::bad_request`) instead of a crash.
pub(crate) fn on_crash(cfg: &Config, msg: String) {
    if let Some(hook) = &cfg.on_fatal {
        hook(msg);
    }
}

impl Conn {
    /// true — the connection is alive, wait for the next request.
    async fn next_request(&mut self, first: Option<Instant>) -> bool {
        let Some(head) = self.wait_head(first).await else {
            return false;
        };
        if !self.expect(&head).await {
            return false;
        }
        self.dispatch(head).await
    }

    /// `first` — shared deadline of the first request; later ones wait for keepalive, then a fresh 60 s.
    async fn wait_head(&mut self, first: Option<Instant>) -> Option<RequestHead> {
        let deadline = first.unwrap_or_else(|| Instant::now() + HEADERS_TIMEOUT);
        if self.buf.is_empty() {
            let idle = match first {
                Some(d) => d.saturating_duration_since(Instant::now()),
                None => KEEP_ALIVE,
            };
            let mut probe = [0u8; 1];
            match timeout(idle, self.rd.peek(&mut probe)).await {
                Ok(Ok(n)) if n > 0 => {}
                _ => return None,
            }
        }
        let left = deadline.saturating_duration_since(Instant::now());
        let left = if first.is_some() {
            left
        } else {
            HEADERS_TIMEOUT
        };
        let read = timeout(left, read_head(&mut self.rd, &mut self.buf)).await;
        match read {
            Err(_) => self.send_reject(Reject::Timeout).await,
            Ok(Ok(HeadRead::Head(h))) => return Some(h),
            Ok(Ok(HeadRead::Reject(r))) => self.send_reject(r).await,
            Ok(_) => {}
        }
        None
    }

    async fn send_reject(&mut self, r: Reject) {
        if r == Reject::Drop {
            return;
        }
        let _ = self.wr.write_all(reject_bytes(r)).await;
    }

    /// false — close the connection (other Expect -> 417).
    async fn expect(&mut self, head: &RequestHead) -> bool {
        let Some(value) = head.headers.get("expect") else {
            return true;
        };
        if value.eq_ignore_ascii_case("100-continue") {
            if head.minor == 1 {
                return self.wr.write_all(CONTINUE).await.is_ok();
            }
            return true;
        }
        let resp = Response {
            status: 417,
            reason: None,
            headers: Vec::new(),
            body: Body::Empty,
        };
        let _ = write_response(&mut self.wr, resp, false, head.minor == 0).await;
        false
    }

    async fn dispatch(&mut self, head: RequestHead) -> bool {
        let fr = framing(&head.headers);
        let (outcome, drain) = match admit(&self.cfg, &head) {
            Step::Done(o) => (o, true),
            Step::Get { full, url } => {
                let range = head.headers.get("range");
                (handle_get(&self.cfg, &full, &url, range).await, true)
            }
            Step::Post { url } => match self.post(&url, fr).await {
                Some(o) => (o, false),
                None => return false,
            },
        };
        let keep = wants_keep_alive(&head);
        let head_only = head.method == "HEAD";
        self.finish(
            outcome,
            keep,
            (head.minor == 0, head_only),
            drain.then_some(fr),
        )
        .await
    }

    /// None — the connection is closed or the response was already sent (408).
    async fn post(&mut self, url: &str, fr: Framing) -> Option<Outcome> {
        let read = read_body(&mut self.rd, &mut self.buf, fr, MAX_BODY);
        match timeout(REQUEST_TIMEOUT, read).await {
            Err(_) => {
                self.send_reject(Reject::Timeout).await;
                None
            }
            // JS: index.js:182 — req.on('error') writes 500, but a body abort/EOF means the client
            // is already gone (nobody to read the response); Node cuts malformed chunked earlier, in llhttp, with a 400.
            Ok(Err(e)) => {
                if e.kind() == std::io::ErrorKind::InvalidData {
                    self.send_reject(Reject::BadRequest).await;
                }
                None
            }
            Ok(Ok(BodyRead::TooLarge)) => Some(Outcome::ReplyClose(too_large())),
            Ok(Ok(BodyRead::Body(body))) => Some(handle_post(&self.cfg, url, &body).await),
        }
    }

    /// `drain` — the request body is not read yet; read the rest under keepalive.
    async fn finish(
        &mut self,
        outcome: Outcome,
        keep: bool,
        (http10, head_only): (bool, bool),
        drain: Option<Framing>,
    ) -> bool {
        match outcome {
            Outcome::Reply(r) => self.reply(r, keep, (http10, head_only), drain).await,
            // JS: 413 is written with keepalive headers, then req.destroy() tears down the socket.
            Outcome::ReplyClose(r) => {
                self.reply(r, keep, (http10, head_only), drain).await;
                false
            }
            Outcome::Hang => {
                self.hang().await;
                false
            }
            Outcome::Crash(_) if self.cfg.safe() => {
                self.reply(safe::bad_request(), false, (http10, head_only), None)
                    .await;
                false
            }
            Outcome::Crash(msg) => {
                on_crash(&self.cfg, msg);
                false
            }
        }
    }

    async fn reply(
        &mut self,
        r: Response,
        keep: bool,
        (http10, head_only): (bool, bool),
        drain: Option<Framing>,
    ) -> bool {
        match write_response_for(&mut self.wr, r, keep, http10, head_only).await {
            Ok(true) => {}
            _ => return false,
        }
        let Some(fr) = drain else {
            return true;
        };
        let drained = drain_body(&mut self.rd, &mut self.buf, fr);
        matches!(timeout(REQUEST_TIMEOUT, drained).await, Ok(Ok(())))
    }

    /// No response: read and discard until the client closes.
    async fn hang(&mut self) {
        self.buf.clear();
        let mut chunk = [0u8; 8192];
        while matches!(self.rd.read(&mut chunk).await, Ok(n) if n > 0) {}
    }
}

fn too_large() -> Response {
    Response {
        status: 413,
        reason: None,
        headers: vec![("Content-Type".into(), "text/plain".into())],
        body: Body::Bytes(b"Request entity too large".to_vec()),
    }
}

/// Keepalive by default: HTTP/1.1 yes unless `close`; HTTP/1.0 only with `keep-alive`.
fn wants_keep_alive(head: &RequestHead) -> bool {
    let value = head.headers.get("connection").unwrap_or("");
    let has = |tok: &str| value.split(',').any(|t| t.trim().eq_ignore_ascii_case(tok));
    if head.minor == 0 {
        return has("keep-alive");
    }
    !has("close")
}
