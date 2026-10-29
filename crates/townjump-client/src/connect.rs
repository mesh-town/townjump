// JS: holesail-client/index.js:HolesailClient.connect, handleTCP, handleUDP; order and merge as in client.md «connect(options)».
// Related: client.rs (rest of the API). The local port is bound inside `connect`, a bind error goes to `Fatal`.
use crate::{args::ConnectArgs, client::HolesailClient, error::ClientError};
use std::{future::Future, io, pin::Pin, sync::Arc};
use townjump_dht::{ConnectOpts, Dht, Duplex};
use townjump_jsval::{JsValue, json, nullish, to_port};
use townjump_pipe::{
    PipeStats, PiperOpts, TcpProxy, UdpProxy, create_tcp_proxy, create_udp_framed_proxy,
};

/// Result of merging the flags with the DHT record.
#[derive(Debug, Clone)]
pub struct Merged {
    pub port: JsValue,
    pub host: JsValue,
    pub udp: bool,
}

/// Local entry point of the client; closed in `destroy`.
pub enum Proxy {
    Tcp(TcpProxy),
    Udp(UdpProxy),
}

impl Proxy {
    /// Stops accepting and waits until the socket is actually released.
    pub async fn close(self) {
        let task = match self {
            Proxy::Tcp(p) => p.task,
            Proxy::Udp(p) => p.task,
        };
        task.abort();
        let _ = task.await;
    }
}

type Link = Pin<Box<dyn Future<Output = io::Result<Box<dyn Duplex>>> + Send>>;

impl<D: Dht> HolesailClient<D> {
    /// Order as in client.md; returns after the local port is bound, a bind error goes to `Fatal`.
    pub async fn connect(&mut self, args: ConnectArgs) -> Result<(), ClientError> {
        self.logger.info(&format!(
            "Connecting to key: {}, secure: {}",
            self.shown_key(),
            self.secure
        ));
        let data = self.read_record().await?;
        let merged = merge(&args, &data)?;
        self.merged = Some(merged.clone());
        self.state = Some("waiting");
        self.start_proxy(&merged).await
    }

    /// The private-mode secret never reaches the log: only its fingerprint.
    fn shown_key(&self) -> String {
        if self.secure {
            return townjump_keys::fingerprint(&self.key);
        }
        self.key.clone()
    }

    /// `dhtData`: the parsed record or `{}`; invalid JSON is an error.
    async fn read_record(&self) -> Result<JsValue, ClientError> {
        let Some(record) = self.get().await? else {
            self.logger.warn("No DHT data retrieved");
            return Ok(JsValue::Object(Vec::new()));
        };
        let data = JsValue::parse_json(&String::from_utf8_lossy(&record.value))?;
        let shown = json::stringify(&data).unwrap_or_default();
        self.logger.debug(&format!("Retrieved DHT data: {shown}"));
        Ok(data)
    }

    async fn start_proxy(&mut self, m: &Merged) -> Result<(), ClientError> {
        let listen = (m.host.to_js_string(), to_port(&m.port)?);
        let bound = if m.udp {
            self.bind_udp(&listen).await
        } else {
            self.bind_tcp(&listen).await
        };
        match bound {
            Ok(proxy) => self.proxy = Some(proxy),
            Err(err) => {
                self.fatal.raise(bind_message(&err, &listen));
                return Ok(());
            }
        }
        self.state = Some("listening");
        let suffix = if m.udp { " for UDP" } else { "" };
        self.logger.info(&format!(
            "Proxy listening on {}:{}{suffix}",
            listen.0,
            m.port.to_js_string()
        ));
        Ok(())
    }

    async fn bind_tcp(&self, listen: &(String, u16)) -> io::Result<Proxy> {
        self.logger.debug("Handling TCP connection");
        let opts = PiperOpts {
            logger: Some(self.logger.clone()),
            on_destroy: None,
        };
        let connect = self.connector(ConnectOpts {
            reusable_socket: true,
            ..Default::default()
        });
        let proxy = create_tcp_proxy(
            listen.clone(),
            connect,
            opts,
            Arc::new(PipeStats::default()),
        );
        proxy.await.map(Proxy::Tcp)
    }

    async fn bind_udp(&self, listen: &(String, u16)) -> io::Result<Proxy> {
        self.logger.debug("Handling UDP connection");
        let connect = self.connector(ConnectOpts::default());
        let proxy = create_udp_framed_proxy(listen.clone(), connect, self.logger.clone());
        proxy.await.map(Proxy::Udp)
    }

    /// `() => dht.connect(publicKey, opts)`: a new DHT connection for each local one.
    fn connector(&self, opts: ConnectOpts) -> impl Fn() -> Link + Send + Sync + 'static {
        let (dht, key) = (self.dht.clone(), self.public_key);
        move || {
            let (dht, opts) = (dht.clone(), opts.clone());
            Box::pin(async move {
                let conn = dht.connect(key, opts).await.map_err(io::Error::other)?;
                Ok(conn.stream)
            })
        }
    }
}

/// `options.x ?? dhtData.x ?? fallback`.
fn merge(args: &ConnectArgs, data: &JsValue) -> Result<Merged, ClientError> {
    let port = nullish(args.port.as_ref(), data.get("port")?);
    let host = nullish(args.host.as_ref(), data.get("host")?);
    let udp = nullish(args.udp.as_ref(), data.get("udp")?);
    Ok(Merged {
        port: port.cloned().unwrap_or(JsValue::Number(8989.0)),
        host: host.cloned().unwrap_or(JsValue::String("127.0.0.1".into())),
        udp: udp.is_some_and(JsValue::truthy),
    })
}

/// Text of the `listen` error as in Node (`listen EADDRINUSE: address already in use host:port`).
fn bind_message(err: &io::Error, listen: &(String, u16)) -> String {
    let (code, text) = match err.kind() {
        io::ErrorKind::AddrInUse => ("EADDRINUSE", "address already in use"),
        io::ErrorKind::AddrNotAvailable => ("EADDRNOTAVAIL", "address not available"),
        _ => return err.to_string(),
    };
    format!("listen {code}: {text} {}:{}", listen.0, listen.1)
}
