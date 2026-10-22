// JS: holesail/src/index.js:Holesail (constructor, #initialise, connect, info, lookup, pause, resume, _close).
// See also: cli/ uses the facade; keys — townjump-keys, server and client — townjump-server and townjump-client.
mod lookup;
mod plan;

use plan::Plan;
use std::sync::Arc;
use townjump_client::{ClientError, ConnectArgs, HolesailClient};
use townjump_dht::{Dht, DhtFactory};
use townjump_jsval::JsValue;
use townjump_keys::{KeysError, format_url};
use townjump_logger::{Fatal, Logger};
use townjump_server::{HolesailServer, ServerArgs, ServerError};

/// Facade errors; `Display` is the JS exception `.message`.
#[derive(Debug, thiserror::Error)]
pub enum HolesailError {
    /// `connect()` while `running`.
    #[error("Already connected")]
    AlreadyConnected,
    /// `Holesail.lookup`: the key is not z32 (text with the key after `urlParser`).
    #[error("Invalid key format: {0}")]
    InvalidKeyFormat(String),
    /// `validateOpts`: a client without a key.
    #[error("Key is empty")]
    KeyEmpty,
    /// `validateOpts`: a server without an address.
    #[error("No host specified")]
    NoHost,
    #[error(transparent)]
    Keys(#[from] KeysError),
    #[error(transparent)]
    Server(#[from] ServerError),
    #[error(transparent)]
    Client(#[from] ClientError),
}

/// Options of `new Holesail(opts)`; flag values are as from minimist.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HolesailOpts {
    /// `server: true`; otherwise a client.
    pub server: bool,
    pub port: Option<JsValue>,
    pub host: Option<JsValue>,
    /// Key before `urlParser` (`--key`, `--connect` or positional).
    pub key: Option<JsValue>,
    pub udp: Option<JsValue>,
    /// `opts.secure` by truthiness: server `!public`, client `public` (inverted, as in JS).
    pub secure: bool,
}

/// `Holesail.info` (what the banner uses).
#[derive(Debug, Clone, PartialEq)]
pub struct HolesailInfo {
    /// `type`: `server` or `client`.
    pub kind: &'static str,
    pub state: Option<&'static str>,
    pub secure: bool,
    pub port: JsValue,
    pub host: JsValue,
    /// `udp` or `tcp`.
    pub protocol: &'static str,
    /// Server — `args.seed`; client in private mode — `hex(sha256(K))`.
    pub seed: Option<String>,
    /// Private mode — `K` itself, otherwise `z32(public_key)`.
    pub key: String,
    /// `hs://s000<K>` or `hs://0000<z32>`.
    pub url: String,
    pub public_key: String,
}

enum Node<D: Dht> {
    Server(HolesailServer<D>),
    Client(HolesailClient<D>),
}

/// The `Holesail` class: a server or a client behind one face.
pub struct Holesail<F: DhtFactory> {
    factory: F,
    opts: HolesailOpts,
    logger: Logger,
    fatal: Fatal,
    plan: Plan,
    node: Option<Node<F::Dht>>,
    running: bool,
}

impl<F: DhtFactory> Holesail<F> {
    /// Constructor and `#initialise`: `urlParser`, mode (`url.secure ?? opts.secure`), keys; `random` is `randomBytes`.
    /// An empty client key is an error here; a key outside the z32 alphabet only in `ready` (where the client is created).
    pub fn new(
        factory: F,
        opts: HolesailOpts,
        logger: Logger,
        fatal: Fatal,
        random: &mut dyn FnMut(&mut [u8]),
    ) -> Result<Self, HolesailError> {
        let plan = Plan::build(&opts, random)?;
        Ok(Holesail {
            factory,
            opts,
            logger,
            fatal,
            plan,
            node: None,
            running: false,
        })
    }

    /// `_open` and `connect()`: creates and starts the server or client; the return is as in `start`/`connect`.
    pub async fn ready(&mut self) -> Result<(), HolesailError> {
        if self.node.is_some() {
            return Ok(());
        }
        let node = self.open().await?;
        self.node = Some(node);
        self.connect().await
    }

    async fn open(&self) -> Result<Node<F::Dht>, HolesailError> {
        let (logger, fatal) = (self.logger.clone(), self.fatal.clone());
        match &self.plan {
            Plan::Server { .. } => {
                let server = HolesailServer::new(&self.factory, logger, fatal).await?;
                Ok(Node::Server(server))
            }
            Plan::Client { seed, secure, .. } => {
                let client = HolesailClient::new(&self.factory, seed, *secure, logger, fatal);
                Ok(Node::Client(client.await?))
            }
        }
    }

    /// `connect()`: `running` → `Already connected`, otherwise node `start` or `connect`.
    async fn connect(&mut self) -> Result<(), HolesailError> {
        if self.running {
            return Err(HolesailError::AlreadyConnected);
        }
        let (opts, plan) = (&self.opts, &self.plan);
        match (&mut self.node, plan) {
            (Some(Node::Server(server)), Plan::Server { keys, .. }) => {
                server.start(server_args(opts, plan, keys)).await?;
            }
            (Some(Node::Client(client)), _) => client.connect(connect_args(opts)).await?,
            _ => {}
        }
        self.running = true;
        Ok(())
    }

    /// The DHT node behind the server or client; `None` before `ready`.
    pub fn dht(&self) -> Option<Arc<F::Dht>> {
        match &self.node {
            Some(Node::Server(server)) => Some(server.dht().clone()),
            Some(Node::Client(client)) => Some(client.dht().clone()),
            None => None,
        }
    }

    pub fn info(&self) -> HolesailInfo {
        match &self.node {
            Some(Node::Server(server)) => self.server_info(server.info()),
            Some(Node::Client(client)) => self.client_info(client.info()),
            None => self.bare_info(),
        }
    }

    fn server_info(&self, info: townjump_server::ServerInfo) -> HolesailInfo {
        let (seed, key) = self.plan.server_seed_and_key();
        HolesailInfo {
            kind: info.kind,
            state: info.state,
            secure: info.secure,
            port: info.port,
            host: info.host,
            protocol: info.protocol,
            seed,
            url: format_url(&key, info.secure),
            key,
            public_key: info.public_key,
        }
    }

    fn client_info(&self, info: townjump_client::ClientInfo) -> HolesailInfo {
        let key = self.plan.client_shown_key();
        HolesailInfo {
            kind: info.kind,
            state: info.state,
            secure: info.secure,
            port: info.port,
            host: info.host,
            protocol: info.protocol,
            seed: self.plan.client_seed_hex(),
            url: format_url(&key, info.secure),
            key,
            public_key: info.public_key,
        }
    }

    /// Before `ready`: there is no node yet, the network fields are empty.
    fn bare_info(&self) -> HolesailInfo {
        let (seed, key) = self.plan.shown();
        let secure = self.plan.secure();
        HolesailInfo {
            kind: if self.opts.server { "server" } else { "client" },
            state: None,
            secure,
            port: JsValue::Null,
            host: JsValue::Null,
            protocol: "tcp",
            seed,
            url: format_url(&key, secure),
            key,
            public_key: String::new(),
        }
    }

    /// `pause()`: delegates to the node (server or client), which calls `Dht::suspend`.
    pub async fn pause(&mut self) -> Result<(), HolesailError> {
        match &mut self.node {
            Some(Node::Server(server)) => Ok(server.pause().await?),
            Some(Node::Client(client)) => Ok(client.pause().await?),
            None => Ok(()),
        }
    }

    /// `resume()`: delegates to the node, which calls `Dht::resume`.
    pub async fn resume(&mut self) -> Result<(), HolesailError> {
        match &mut self.node {
            Some(Node::Server(server)) => Ok(server.resume().await?),
            Some(Node::Client(client)) => Ok(client.resume().await?),
            None => Ok(()),
        }
    }

    /// `_close`: node `destroy`, `running = false`.
    pub async fn close(&mut self) -> Result<(), HolesailError> {
        self.running = false;
        match &mut self.node {
            Some(Node::Server(server)) => Ok(server.destroy().await?),
            Some(Node::Client(client)) => Ok(client.destroy().await?),
            None => Ok(()),
        }
    }

    /// `Holesail.lookup(url)`: `ping(argKey) || {}` and `secure` from the URL; the result is a JS object (`host`, `port`, `protocol`, `secure`).
    pub async fn lookup(factory: &F, url: &str) -> Result<JsValue, HolesailError> {
        lookup::lookup(factory, url).await
    }
}

fn server_args(opts: &HolesailOpts, plan: &Plan, keys: &townjump_keys::ServerKeys) -> ServerArgs {
    ServerArgs {
        port: opts.port.clone().unwrap_or(JsValue::Null),
        host: opts.host.clone().unwrap_or(JsValue::Null),
        seed_hex: Some(keys.generated_seed_hex.clone()),
        secure: plan.secure(),
        udp: opts.udp.clone(),
    }
}

fn connect_args(opts: &HolesailOpts) -> ConnectArgs {
    ConnectArgs {
        port: opts.port.clone(),
        host: opts.host.clone(),
        udp: opts.udp.clone(),
    }
}

#[cfg(test)]
#[path = "facade_tests.rs"]
mod tests;
