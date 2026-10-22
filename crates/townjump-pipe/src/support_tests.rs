// Crate test helpers: waits with a timeout, log capture, faulty streams, golden frames.
// Included from lib.rs (`#[cfg(test)]`); module tests take them via `crate::support`.
use std::future::Future;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::net::UdpSocket;
use tokio::time::{sleep, timeout};
use townjump_logger::{Logger, SharedOut};

/// Ceiling for waiting on one scenario step.
pub const STEP: Duration = Duration::from_secs(5);
/// Deadline of the negative check "nothing arrived".
pub const QUIET: Duration = Duration::from_millis(250);
/// `duplex` buffer: under 1 MB so that pumping goes in a stream, not in one go.
pub const PIPE_BUF: usize = 64 * 1024;

pub async fn within<F: Future>(what: &str, f: F) -> F::Output {
    match timeout(STEP, f).await {
        Ok(value) => value,
        Err(_) => panic!("timed out after 5 s: {what}"),
    }
}

pub async fn stays_pending<F: Future>(what: &str, f: F) {
    assert!(
        timeout(QUIET, f).await.is_err(),
        "something happened that must not: {what}"
    );
}

pub async fn wait_until(what: &str, cond: impl Fn() -> bool) {
    let waiting = async {
        while !cond() {
            sleep(Duration::from_millis(10)).await;
        }
    };
    within(what, waiting).await;
}

#[path = "support_pipes_tests.rs"]
mod pipes;
pub use pipes::*;

/// "The event happened" flag, shared between the join task and the scenario.
#[derive(Clone, Default)]
pub struct Flag(Arc<AtomicBool>);

impl Flag {
    pub fn raise(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_raised(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    pub async fn wait(&self, what: &str) {
        wait_until(what, || self.is_raised()).await;
    }
}

/// Raises `ended` when `piper` returned.
pub async fn until_end(ended: Flag, piper: impl Future<Output = ()>) {
    piper.await;
    ended.raise();
}

/// Runs `piper` and `scenario` on one task. `piper` is polled first, so a `todo!()` in the code under test
/// panics in the test itself. When the scenario ends, `piper` is cancelled (the test does not hang).
pub async fn drive<T>(piper: impl Future<Output = ()>, scenario: impl Future<Output = T>) -> T {
    tokio::pin!(piper);
    tokio::pin!(scenario);
    let mut piper_running = true;
    loop {
        tokio::select! {
            biased;
            () = &mut piper, if piper_running => piper_running = false,
            out = &mut scenario => return out,
        }
    }
}

/// Bytes with no visible pattern: a permutation or a lost segment will not pass the comparison.
pub fn pattern(len: usize, seed: u32) -> Vec<u8> {
    let mut x = seed.wrapping_mul(2_654_435_761) | 1;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        (x >> 8) as u8
    };
    (0..len).map(|_| next()).collect()
}

/// Frame `[u32 BE length][body]`, built independently of `encode_frame`.
pub fn frame(body: &[u8]) -> Vec<u8> {
    let mut bytes = (body.len() as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(body);
    bytes
}

pub async fn read_n<R: AsyncRead + Unpin>(stream: &mut R, n: usize) -> Vec<u8> {
    let mut buf = vec![0; n];
    let read = within("reading bytes from the stream", stream.read_exact(&mut buf)).await;
    read.unwrap_or_else(|e| panic!("stream failed before delivering {n} bytes: {e}"));
    buf
}

/// Reads one frame and returns its body.
pub async fn read_frame<R: AsyncRead + Unpin>(stream: &mut R) -> Vec<u8> {
    let head = read_n(stream, 4).await;
    let len = u32::from_be_bytes([head[0], head[1], head[2], head[3]]);
    read_n(stream, len as usize).await
}

/// A stream cleanly closed for reading: `read` gives 0 bytes, not an error.
pub async fn expect_eof<R: AsyncRead + Unpin>(stream: &mut R, what: &str) {
    let mut buf = [0u8; 16];
    let read = within(what, stream.read(&mut buf)).await;
    let n = read.unwrap_or_else(|e| panic!("{what}: error {e} instead of EOF"));
    assert_eq!(n, 0, "{what}: data instead of EOF");
}

/// The stream is closed (EOF or error); data that was in flight before the close is skipped.
pub async fn expect_closed<R: AsyncRead + Unpin>(stream: &mut R, what: &str) {
    let draining = async {
        let mut buf = [0u8; 4096];
        while let Ok(n) = stream.read(&mut buf).await {
            if n == 0 {
                return;
            }
        }
    };
    within(what, draining).await;
}

pub fn free_tcp_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    listener.local_addr().expect("local_addr").port()
}

pub async fn udp_socket() -> UdpSocket {
    UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("bind udp loopback")
}

/// Receives one datagram: body and sender address (the buffer is larger than any test datagram).
pub async fn recv(socket: &UdpSocket) -> (Vec<u8>, SocketAddr) {
    let mut buf = vec![0u8; 70_000];
    let received = within("datagram", socket.recv_from(&mut buf)).await;
    let (n, from) = received.expect("recv_from");
    buf.truncate(n);
    (buf, from)
}

/// Log capture: `Logger` writes into a shared buffer; lines are read by level and text.
#[derive(Clone, Default)]
pub struct LogCapture(Arc<Mutex<Vec<u8>>>);

impl LogCapture {
    pub fn logger(&self) -> Logger {
        let out: SharedOut = self.0.clone();
        Logger::new(Some(0.0), out.clone(), out)
    }

    pub fn lines(&self) -> Vec<String> {
        let bytes = self.0.lock().expect("log").clone();
        String::from_utf8_lossy(&bytes)
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn after_level(line: &str, level: &str) -> Option<String> {
        let (_, rest) = line.split_once(&format!("[{level}]"))?;
        Some(rest.trim().to_owned())
    }

    /// Line numbers of level `level` whose text is exactly `msg` (after the level tag).
    pub fn positions(&self, level: &str, msg: &str) -> Vec<usize> {
        let same =
            |line: &String| Self::after_level(line, level).is_some_and(|rest| rest.ends_with(msg));
        let lines = self.lines();
        (0..lines.len()).filter(|i| same(&lines[*i])).collect()
    }

    pub fn has(&self, level: &str, msg: &str) -> bool {
        !self.positions(level, msg).is_empty()
    }

    /// There is a line of level `level` whose text contains `part`.
    pub fn has_containing(&self, level: &str, part: &str) -> bool {
        let lines = self.lines();
        lines
            .iter()
            .any(|l| Self::after_level(l, level).is_some_and(|rest| rest.contains(part)))
    }

    pub async fn wait_has(&self, level: &str, msg: &str) {
        self.wait_for(&format!("[{level}] {msg}"), || self.has(level, msg))
            .await;
    }

    pub async fn wait_containing(&self, level: &str, part: &str) {
        self.wait_for(&format!("[{level}] …{part}…"), || {
            self.has_containing(level, part)
        })
        .await;
    }

    async fn wait_for(&self, what: &str, found: impl Fn() -> bool) {
        let deadline = Instant::now() + STEP;
        while !found() {
            let log = self.lines();
            assert!(
                Instant::now() < deadline,
                "no log line {what}; log: {log:#?}"
            );
            sleep(Duration::from_millis(10)).await;
        }
    }
}
