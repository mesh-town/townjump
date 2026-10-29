// Shared server test scaffolding: `setup` with preconditions, `Fixture`, log line parsing, z32 and local services.
// Network is `MockFactory` with `MockConfig` settings (gates and errors); ports are 127.0.0.1 only.
use super::*;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::timeout;
use townjump_dht::{ConnectOpts, DhtConnection, DhtError, KeyPair};
use townjump_jsval::JsValue;
use townjump_testkit::capture::Capture;
use townjump_testkit::key_pair_from_seed;
use townjump_testkit::mock::{MockConfig, MockDht, MockFactory, MockTrace};

/// `hex(sha256("abc"))`: the seed of the `key-derivation.json` vectors.
pub(super) const SEED_HEX: &str =
    "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
/// `z32(seed)` of the same vector: the private mode key.
pub(super) const PRIVATE_KEY_Z32: &str = "zjhbpxhxy886wokbedxf5mtnrqaygapd1amzi8fwnd9sdhoynsso";
/// `z32(publicKey)` of the same vector: the public mode key, `public_key` in `info`.
pub(super) const PUBLIC_KEY_Z32: &str = "bxxinyn6mjjf31p9ssxtobak64wkcyjty3jo7qrweikkmm1c8u7y";
/// `dht.connect` options of a private and public mode client: `reusableSocket` (TCP).
pub(super) fn tunnel_opts() -> ConnectOpts {
    ConnectOpts {
        reusable_socket: true,
        ..Default::default()
    }
}
/// Scenario wait limit.
pub(super) const WAIT: Duration = Duration::from_secs(5);
/// "Nothing arrived".
pub(super) const SILENCE: Duration = Duration::from_millis(300);

/// Server preconditions; by default a private TCP server on `127.0.0.1:3000` with the vector seed.
pub(super) struct Setup {
    pub secure: bool,
    pub port: JsValue,
    pub host: JsValue,
    pub udp: Option<JsValue>,
    pub seed_hex: Option<String>,
    pub mock: MockConfig,
}

impl Default for Setup {
    fn default() -> Self {
        Setup {
            secure: true,
            port: JsValue::Number(3000.0),
            host: JsValue::String("127.0.0.1".to_string()),
            udp: None,
            seed_hex: Some(SEED_HEX.to_string()),
            mock: MockConfig::default(),
        }
    }
}

/// Server, network, trace, log output and the `Fatal` channel.
pub(super) struct Fixture {
    pub server: HolesailServer<MockDht>,
    pub factory: MockFactory,
    pub trace: MockTrace,
    pub out: Capture,
    pub err: Capture,
    pub fatal: UnboundedReceiver<String>,
    pub args: ServerArgs,
}

/// Creates a server (`new`, no `start`) with a DEBUG-level log.
pub(super) async fn setup(opts: Setup) -> Fixture {
    let factory = MockFactory::with_config(opts.mock);
    let (out, err) = (Capture::new(), Capture::new());
    let logger = Logger::new(Some(0.0), out.shared(), err.shared());
    let (fatal_sender, fatal) = Fatal::channel();
    let server = HolesailServer::new(&factory, logger, fatal_sender)
        .await
        .expect("new returned an error");
    let args = ServerArgs {
        port: opts.port,
        host: opts.host,
        seed_hex: opts.seed_hex,
        secure: opts.secure,
        udp: opts.udp,
    };
    let trace = factory.trace();
    Fixture {
        server,
        factory,
        trace,
        out,
        err,
        fatal,
        args,
    }
}

impl Fixture {
    /// `setup` and a successful `start`.
    pub(super) async fn started(opts: Setup) -> Fixture {
        let mut fx = setup(opts).await;
        fx.start().await;
        fx
    }

    pub(super) async fn start(&mut self) {
        let args = self.args.clone();
        self.server.start(args).await.expect("start failed");
    }

    /// Client node on the same network with identity `identity`.
    pub(super) async fn client(&self, identity: KeyPair) -> MockDht {
        self.factory.create(Some(identity)).await.expect("create")
    }

    /// One client connection with identity `identity` to the server with seed `SEED_HEX`.
    pub(super) async fn connect_as(&self, identity: KeyPair) -> Result<DhtConnection, DhtError> {
        let client = self.client(identity).await;
        client
            .connect(server_pair().public_key, tunnel_opts())
            .await
    }

    /// Log stdout lines as "level, text" pairs; a line with the wrong format panics.
    pub(super) fn log(&self) -> Vec<(String, String)> {
        let text = self.out.text();
        let parse = |line: &str| {
            parse_line(line).unwrap_or_else(|| {
                panic!("log line is not `<ISO> [Holesail] [LEVEL] msg`: {line:?}")
            })
        };
        text.lines().map(parse).collect()
    }

    pub(super) fn has(&self, level: &str, msg: &str) -> bool {
        self.log().iter().any(|(l, m)| l == level && m == msg)
    }

    /// Texts of `level` records in order.
    pub(super) fn messages(&self, level: &str) -> Vec<String> {
        let of_level = |(l, m): &(String, String)| (l == level).then(|| m.clone());
        self.log().iter().filter_map(of_level).collect()
    }

    pub(super) async fn wait_log(&self, level: &str, msg: &str) {
        eventually(msg, || self.has(level, msg)).await;
    }

    pub(super) async fn wait_listening(&self) {
        eventually("state = listening", || {
            self.server.info().state == Some("listening")
        })
        .await;
    }

    /// Trace event names in order.
    pub(super) fn names(&self) -> Vec<String> {
        let name = |e: &serde_json::Value| e["ev"].as_str().unwrap_or_default().to_string();
        self.trace.events().iter().map(name).collect()
    }

    /// The `value` field of the first `mutablePut`.
    pub(super) fn put_value(&self) -> String {
        let puts = self.trace.named("mutablePut");
        let first = puts.first().expect("no mutablePut in the trace");
        first["value"].as_str().expect("no value").to_string()
    }
}

/// Server pair with seed `SEED_HEX`.
pub(super) fn server_pair() -> KeyPair {
    key_pair_from_seed(&seed_bytes(SEED_HEX))
}

pub(super) fn seed_bytes(hex_text: &str) -> [u8; 32] {
    let bytes = hex::decode(hex_text).expect("hex");
    bytes.try_into().expect("32 bytes")
}

/// Waits until `ready()` becomes true; 5 s, then a panic with `what`.
pub(super) async fn eventually(what: &str, mut ready: impl FnMut() -> bool) {
    let poll = async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    };
    if timeout(WAIT, poll).await.is_err() {
        panic!("did not arrive within 5 s: {what}");
    }
}

/// `<ISO> [Holesail] [LEVEL] msg` (ANSI colours stripped) -> `(LEVEL, msg)`.
pub(super) fn parse_line(raw: &str) -> Option<(String, String)> {
    let plain = strip_ansi(raw);
    let (stamp, rest) = plain.split_once(' ')?;
    let (level, msg) = rest.strip_prefix("[Holesail] [")?.split_once("] ")?;
    let known = ["DEBUG", "INFO", "WARN", "ERROR"].contains(&level);
    (is_iso_stamp(stamp) && known).then(|| (level.to_string(), msg.to_string()))
}

fn strip_ansi(raw: &str) -> String {
    let mut plain = String::new();
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            let _skipped = chars.by_ref().find(|next| *next == 'm');
        } else {
            plain.push(c);
        }
    }
    plain
}

/// `2026-01-02T03:04:05.678Z`.
fn is_iso_stamp(stamp: &str) -> bool {
    let shape = b"dddd-dd-ddTdd:dd:dd.dddZ";
    let fits = |(b, m): (u8, &u8)| {
        if *m == b'd' {
            b.is_ascii_digit()
        } else {
            b == *m
        }
    };
    stamp.len() == shape.len() && stamp.bytes().zip(shape).all(fits)
}

const Z32: &[u8; 32] = b"ybndrfg8ejkmcpqxot1uwisza345h769";

/// z32 (own, independent of `townjump-keys`); checked against the `key-derivation.json` vectors.
pub(super) fn z32(bytes: &[u8]) -> String {
    let (mut text, mut acc, mut bits) = (String::new(), 0u32, 0u32);
    for byte in bytes {
        acc = (acc << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            text.push(char::from(Z32[((acc >> (bits - 5)) & 31) as usize]));
            bits -= 5;
        }
        acc &= (1 << bits) - 1;
    }
    if bits > 0 {
        text.push(char::from(Z32[((acc << (5 - bits)) & 31) as usize]));
    }
    text
}

/// Inverse of `z32`; extra tail bits are dropped.
pub(super) fn z32_decode(text: &str) -> Vec<u8> {
    let (mut bytes, mut acc, mut bits) = (Vec::new(), 0u32, 0u32);
    for ch in text.bytes() {
        let index = Z32.iter().position(|c| *c == ch).expect("not z32");
        acc = (acc << 5) | index as u32;
        bits += 5;
        if bits >= 8 {
            bytes.push((acc >> (bits - 8)) as u8);
            bits -= 8;
            acc &= (1 << bits) - 1;
        }
    }
    bytes
}

/// Secret in the log: the first 8 characters, the rest `*`.
pub(super) fn masked(secret: &str) -> String {
    let mask = |(i, c): (usize, char)| if i < 8 { c } else { '*' };
    secret.chars().enumerate().map(mask).collect()
}

/// Local TCP service the server forwards traffic to.
pub(super) struct Backend(TcpListener);

pub(super) async fn backend() -> Backend {
    Backend(TcpListener::bind("127.0.0.1:0").await.expect("bind"))
}

impl Backend {
    pub(super) fn port(&self) -> JsValue {
        let port = self.0.local_addr().expect("local_addr").port();
        JsValue::Number(f64::from(port))
    }

    /// Next accepted connection (5 s).
    pub(super) async fn accept(&self) -> TcpStream {
        let accepted = timeout(WAIT, self.0.accept()).await;
        accepted
            .expect("service received no connection")
            .expect("accept")
            .0
    }
}

/// A port nobody listens on: taken and immediately released.
pub(super) async fn free_port() -> JsValue {
    let holder = backend().await;
    let port = holder.port();
    drop(holder);
    port
}

pub(super) async fn read_n<R: AsyncRead + Unpin>(stream: &mut R, n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    let read = timeout(WAIT, stream.read_exact(&mut buf)).await;
    read.expect("read did not finish within 5 s")
        .expect("read error");
    buf
}

/// Stream closed: the read returned EOF or an error within 5 s.
pub(super) async fn is_closed<R: AsyncRead + Unpin>(stream: &mut R) -> bool {
    let mut byte = [0u8; 1];
    let read = timeout(WAIT, stream.read(&mut byte)).await;
    matches!(read, Ok(Ok(0) | Err(_)))
}
