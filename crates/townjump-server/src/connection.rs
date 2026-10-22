// JS: holesail-server/index.js:HolesailServer.onConnection/handleTCP/handleUDP; splicing is townjump-pipe. Related: start.rs.
// Accounting of `activeConnections` (for logs only) and one task per incoming connection.
use crate::watched::Watched;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use townjump_dht::{DhtConnection, Duplex, Incoming};
use townjump_jsval::{JsValue, to_port};
use townjump_keys::z32;
use townjump_logger::Logger;
use townjump_pipe::{PipeStats, PiperOpts, pipe_tcp_server, pipe_udp_framed_server};

/// Everything connection acceptance needs; cloned into tasks.
#[derive(Clone)]
pub(crate) struct Acceptor {
    pub logger: Logger,
    pub host: JsValue,
    pub port: JsValue,
    pub udp: bool,
    active: Arc<Mutex<HashMap<String, usize>>>,
    stats: Arc<PipeStats>,
}

impl Acceptor {
    pub(crate) fn new(logger: Logger, host: JsValue, port: JsValue, udp: bool) -> Self {
        Acceptor {
            logger,
            host,
            port,
            udp,
            active: Arc::default(),
            stats: Arc::default(),
        }
    }

    /// Accepts connections while the server is open.
    pub(crate) async fn run(self, mut incoming: Incoming) {
        while let Some(connection) = incoming.accept().await {
            self.on_connection(connection);
        }
    }

    fn on_connection(&self, connection: DhtConnection) {
        let key = z32::encode(&connection.remote_public_key);
        self.logger
            .debug(&format!("Incoming connection received from {key}"));
        *self.active.lock().unwrap().entry(key.clone()).or_insert(0) += 1;
        if self.udp {
            return self.handle_udp(connection, key);
        }
        self.handle_tcp(connection, key);
    }

    fn handle_tcp(&self, connection: DhtConnection, key: String) {
        self.logger.debug("Handling TCP connection");
        let (this, opts) = (self.clone(), self.piper_opts());
        let remote = this.watch(connection, key);
        tokio::spawn(async move {
            let Some(local) = this.target() else { return };
            pipe_tcp_server(remote, local, opts, this.stats.clone()).await;
        });
        self.logger.debug("TCP connection piped");
    }

    fn handle_udp(&self, connection: DhtConnection, key: String) {
        self.logger.debug("Handling UDP connection");
        let this = self.clone();
        let remote = this.watch(connection, key);
        tokio::spawn(async move {
            let Some(local) = this.target() else { return };
            let (logger, stats) = (this.logger.clone(), this.stats.clone());
            pipe_udp_framed_server(remote, local, logger, stats).await;
        });
        self.logger.debug("UDP connection framed and piped");
    }

    /// Stream that will report its closing to the connection accounting.
    fn watch(&self, connection: DhtConnection, key: String) -> Watched<Box<dyn Duplex>> {
        let this = self.clone();
        let close_on_drop = connection.reports_close_on_drop();
        Watched::new(connection.stream, close_on_drop, move || {
            this.disconnected(&key)
        })
    }

    fn piper_opts(&self) -> PiperOpts {
        PiperOpts {
            logger: Some(self.logger.clone()),
            on_destroy: None,
        }
    }

    /// Target address; a port that is invalid by Node rules just closes the connection.
    fn target(&self) -> Option<(String, u16)> {
        let port = to_port(&self.port).ok()?;
        Some((self.host.to_js_string(), port))
    }

    /// Stream closed: at zero connections of the key the record goes away and `Disconnected` is written.
    fn disconnected(&self, key: &str) {
        let mut active = self.active.lock().unwrap();
        let Some(count) = active.get_mut(key) else {
            return;
        };
        *count -= 1;
        if *count > 0 {
            return;
        }
        active.remove(key);
        self.logger.debug(&format!("Disconnected from {key}"));
    }
}
