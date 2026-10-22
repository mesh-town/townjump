// Shared client test helpers: fixture "client + mock network + log + Fatal", waits with a timeout, key vectors.
// Included from lib.rs (`#[cfg(test)]`); module tests reach them via `crate::support`.
#![allow(dead_code, reason = "helpers shared by several test files")]
use crate::{ClientError, ClientInfo, ConnectArgs, HolesailClient};
use serde_json::Value;
use std::future::Future;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::{sleep, timeout};
use townjump_dht::{Dht, DhtConnection, DhtFactory, DhtServer, Incoming, KeyPair, ServerOpts};
use townjump_jsval::JsValue;
use townjump_logger::{Fatal, Logger};
use townjump_testkit::capture::Capture;
use townjump_testkit::mock::{MockConfig, MockDht, MockFactory, MockServer, RecordFor};

/// Ceiling for waiting on one scenario step.
pub const STEP: Duration = Duration::from_secs(5);
/// Deadline for the negative check "nothing arrived".
pub const QUIET: Duration = Duration::from_millis(250);

/// Public key from the matrix (`conn-public-z32`): 51 z32 characters, decodes to 32 bytes.
pub const PUBLIC_Z32: &str = "ybndrfg8ejkmcpqxot1uwisza345h769ybndrfg8ejkmcpqxot1";
pub const PUBLIC_PK: &str = "00443214c74254b635cf84653a56d7c675be77df00443214c74254b635cf8464";
/// `z32.encode(publicKey)` is what JS puts into `info.publicKey` (52 characters).
pub const PUBLIC_INFO_KEY: &str = "ybndrfg8ejkmcpqxot1uwisza345h769ybndrfg8ejkmcpqxot1y";
/// Public key of the pair seeded from `PUBLIC_PK` (attempt "as private" in `ping`; `lookup-*` trace).
pub const DERIVED_PK: &str = "f8ae12ba8e821ed77f49edcffe721f67f1698145f491430969f88b0ebc4c8aeb";
/// K = `abc`: `z32(sha256(K))` is what `HolesailClient` receives in private mode (key-derivation.json).
pub const PRIVATE_SEED_Z32: &str = "zjhbpxhxy886wokbedxf5mtnrqaygapd1amzi8fwnd9sdhoynsso";
pub const PRIVATE_PK: &str = "0bdf51005e5a525cc9bfb59f18070af6a8a6013106530eb8944554a5ae4c3cfa";
pub const PRIVATE_INFO_KEY: &str = "bxxinyn6mjjf31p9ssxtobak64wkcyjty3jo7qrweikkmm1c8u7y";

/// Client mode: key from the connection string.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum Mode {
    #[default]
    Public,
    Private,
}

impl Mode {
    /// What the facade passes as `key`.
    pub fn key(self) -> &'static str {
        match self {
            Mode::Public => PUBLIC_Z32,
            Mode::Private => PRIVATE_SEED_Z32,
        }
    }

    pub fn secure(self) -> bool {
        self == Mode::Private
    }
}

/// Scenario preconditions.
#[derive(Default)]
pub struct Opts {
    pub mode: Mode,
    /// Another `key` instead of the mode key (matrix vectors).
    pub key: Option<String>,
    /// Stub for `mutable_get` (`seq` 3) for any key; absent means no record on the network.
    pub record: Option<String>,
}

impl Opts {
    pub fn record(text: impl Into<String>) -> Opts {
        Opts {
            record: Some(text.into()),
            ..Opts::default()
        }
    }

    pub fn private() -> Opts {
        Opts {
            mode: Mode::Private,
            ..Opts::default()
        }
    }
}

/// Client on the mock network; log (both streams into one buffer, order preserved) and the `Fatal` channel.
pub struct Fixture {
    pub factory: MockFactory,
    pub client: HolesailClient<MockDht>,
    pub log: Capture,
    pub fatal: UnboundedReceiver<String>,
}

pub async fn setup(opts: Opts) -> Fixture {
    let factory = match opts.record {
        Some(text) => MockFactory::with_config(MockConfig::default().record(RecordFor::Any, text)),
        None => MockFactory::new(),
    };
    let log = Capture::new();
    let key = opts.key.as_deref().unwrap_or(opts.mode.key());
    let made = try_new(&factory, &log, key, opts.mode.secure()).await;
    let (client, fatal) = made.expect("HolesailClient::new");
    Fixture {
        factory,
        client,
        log,
        fatal,
    }
}

/// `HolesailClient::new` with the log on `log` (level 0 - everything) and a new `Fatal` channel.
pub async fn try_new(
    factory: &MockFactory,
    log: &Capture,
    key: &str,
    secure: bool,
) -> Result<(HolesailClient<MockDht>, UnboundedReceiver<String>), ClientError> {
    let logger = Logger::new(Some(0.0), log.shared(), log.shared());
    let (fatal_tx, fatal_rx) = Fatal::channel();
    let client = HolesailClient::new(factory, key, secure, logger, fatal_tx).await?;
    Ok((client, fatal_rx))
}

pub fn error_of<T>(result: Result<T, ClientError>) -> ClientError {
    match result {
        Ok(_) => panic!("expected an error, got Ok"),
        Err(err) => err,
    }
}

impl Fixture {
    pub async fn connect(&mut self, args: ConnectArgs) -> Result<(), ClientError> {
        within("connect", self.client.connect(args)).await
    }

    pub async fn connect_ok(&mut self, args: ConnectArgs) {
        self.connect(args).await.expect("connect must return Ok");
    }

    /// TCP proxy on a free port; waits for the `listening` state.
    pub async fn start(&mut self) -> u16 {
        let port = free_port();
        self.connect_ok(local(port)).await;
        self.wait_listening().await;
        port
    }

    /// Same for UDP.
    pub async fn start_udp(&mut self) -> u16 {
        let port = free_udp_port();
        self.connect_ok(local_udp(port)).await;
        self.wait_listening().await;
        port
    }

    pub async fn wait_listening(&self) {
        let listening = || self.client.info().state == Some("listening");
        wait_until("state = listening", listening).await;
    }

    pub fn info(&self) -> ClientInfo {
        self.client.info()
    }

    /// Log lines as `[LEVEL] text` (timestamp and ANSI colours dropped).
    pub fn lines(&self) -> Vec<String> {
        log_lines(&self.log.text())
    }

    /// Mock trace without the `opts` field of `HyperDHT.new` (JS and the mock differ there).
    pub fn trace(&self) -> Vec<Value> {
        normalized(self.factory.trace().events())
    }

    pub fn named(&self, ev: &str) -> Vec<Value> {
        let is_named = |e: &Value| e["ev"] == ev;
        self.trace().into_iter().filter(is_named).collect()
    }

    pub async fn next_fatal(&mut self) -> String {
        let got = within("Fatal", self.fatal.recv()).await;
        got.expect("Fatal channel closed")
    }

    pub async fn no_fatal(&mut self) {
        stays_pending("Fatal", self.fatal.recv()).await;
    }
}

/// Holesail server on the mock network: listens on key `target` (hex); incoming connections via `next()`.
pub struct Remote {
    incoming: Incoming,
    _server: MockServer,
    _dht: MockDht,
}

pub async fn remote(factory: &MockFactory, target: &str) -> Remote {
    let dht = factory
        .create(Some(pair(target)))
        .await
        .expect("server node");
    let (server, incoming) = dht
        .create_server(ServerOpts::default())
        .expect("createServer");
    server.listen(pair(target)).await.expect("listen");
    Remote {
        incoming,
        _server: server,
        _dht: dht,
    }
}

impl Remote {
    pub async fn next(&mut self) -> DhtConnection {
        let got = within("incoming DHT connection", self.incoming.accept()).await;
        got.expect("server closed")
    }

    pub async fn none_new(&mut self) {
        stays_pending("incoming DHT connection", self.incoming.accept()).await;
    }
}

/// Puts a record into the mock network under key `pk` (hex), as a server would.
pub async fn publish(factory: &MockFactory, pk: &str, value: &str, seq: u64) {
    let dht = factory.create(None).await.expect("writer node");
    let kp = pair(pk);
    let put = dht.mutable_put(&kp, value.as_bytes(), Some(seq));
    put.await.expect("mutable_put");
}

pub fn key32(hex_key: &str) -> [u8; 32] {
    hex::decode(hex_key)
        .expect("hex")
        .try_into()
        .expect("32 bytes")
}

pub fn pair(pk: &str) -> KeyPair {
    KeyPair {
        public_key: key32(pk),
        secret_key: [0; 64],
    }
}

pub async fn within<F: Future>(what: &str, f: F) -> F::Output {
    match timeout(STEP, f).await {
        Ok(value) => value,
        Err(_) => panic!("did not arrive within 5 s: {what}"),
    }
}

pub async fn stays_pending<F: Future>(what: &str, f: F) {
    let elapsed = timeout(QUIET, f).await.is_err();
    assert!(elapsed, "something happened that must not: {what}");
}

pub async fn wait_until(what: &str, cond: impl Fn() -> bool) {
    let waiting = async {
        while !cond() {
            sleep(Duration::from_millis(10)).await;
        }
    };
    within(what, waiting).await;
}

pub async fn read_n<S: AsyncRead + Unpin>(stream: &mut S, n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    within("reading from the stream", stream.read_exact(&mut buf))
        .await
        .expect("read_exact");
    buf
}

/// Stream closed: a read gives EOF or an error; if data arrived, `false`.
pub async fn is_closed<S: AsyncRead + Unpin>(stream: &mut S) -> bool {
    let mut byte = [0u8; 1];
    let read = within("closing the stream", stream.read(&mut byte)).await;
    matches!(read, Ok(0) | Err(_))
}

pub async fn tcp_connect(port: u16) -> TcpStream {
    let connecting = TcpStream::connect(("127.0.0.1", port));
    within("TCP connection to the proxy", connecting)
        .await
        .expect("the proxy accepts connections")
}

pub async fn udp_socket() -> UdpSocket {
    UdpSocket::bind("127.0.0.1:0").await.expect("udp bind")
}

pub fn free_port() -> u16 {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("bind 0");
    probe.local_addr().expect("local_addr").port()
}

pub fn free_udp_port() -> u16 {
    let probe = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind 0");
    probe.local_addr().expect("local_addr").port()
}

pub fn num(n: f64) -> JsValue {
    JsValue::Number(n)
}

pub fn text(s: &str) -> JsValue {
    JsValue::String(s.to_owned())
}

pub fn obj(pairs: &[(&str, JsValue)]) -> JsValue {
    let entries = pairs.iter().map(|(k, v)| ((*k).to_owned(), v.clone()));
    JsValue::Object(entries.collect())
}

/// `--host 127.0.0.1 --port <port>`.
pub fn local(port: u16) -> ConnectArgs {
    ConnectArgs {
        port: Some(num(f64::from(port))),
        host: Some(text("127.0.0.1")),
        udp: None,
    }
}

/// Same with `--udp`.
pub fn local_udp(port: u16) -> ConnectArgs {
    ConnectArgs {
        udp: Some(JsValue::Bool(true)),
        ..local(port)
    }
}

#[path = "support_text_tests.rs"]
mod text;
pub use text::*;
