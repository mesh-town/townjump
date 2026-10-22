// JS: holesail-client/index.js:HolesailClient (constructor, get, info, pause, resume, destroy); order as in client.md.
// Related: connect.rs (`connect`, merge with the record, proxy), args.rs, info.rs, error.rs, ping.rs.
use crate::{connect::Merged, connect::Proxy, error::ClientError, info::ClientInfo};
use std::sync::Arc;
use townjump_dht::{Dht, DhtFactory, MutableRecord};
use townjump_jsval::JsValue;
use townjump_keys::{KeysError, key_pair_from_seed, z32};
use townjump_logger::{Fatal, Logger};

/// Holesail client on top of the DHT node `D`.
pub struct HolesailClient<D: Dht> {
    pub(crate) dht: Arc<D>,
    pub(crate) logger: Logger,
    pub(crate) fatal: Fatal,
    pub(crate) secure: bool,
    /// `seed` from `Holesail._open` (`info.key` in JS).
    pub(crate) key: String,
    pub(crate) public_key: [u8; 32],
    pub(crate) state: Option<&'static str>,
    pub(crate) merged: Option<Merged>,
    pub(crate) proxy: Option<Proxy>,
}

impl<D: Dht> HolesailClient<D> {
    /// `new HolesailClient({ key })`: `key` is the `seed` from `Holesail._open`: private mode - z32 `sha256(K)`,
    /// public mode - z32 of the server public key (the facade takes it from `townjump_keys::client_identity`).
    /// The key pair is built by `z32::decode` + `key_pair_from_seed`, the node by `factory.create(identity)` (client.md «The `HolesailClient` constructor»).
    pub async fn new<F: DhtFactory<Dht = D>>(
        factory: &F,
        key: &str,
        secure: bool,
        logger: Logger,
        fatal: Fatal,
    ) -> Result<Self, ClientError> {
        let bytes: [u8; 32] = z32::decode(key)?
            .try_into()
            .map_err(|_| KeysError::SeedBytes)?;
        let pair = secure.then(|| key_pair_from_seed(&bytes));
        let public_key = pair.as_ref().map_or(bytes, |p| p.public_key);
        let dht = Arc::new(factory.create(pair).await?);
        Ok(Self {
            dht,
            logger,
            fatal,
            secure,
            key: key.to_owned(),
            public_key,
            state: None,
            merged: None,
            proxy: None,
        })
    }

    /// The DHT node of this client.
    pub fn dht(&self) -> &Arc<D> {
        &self.dht
    }

    /// Server record by public key.
    pub async fn get(&self) -> Result<Option<MutableRecord>, ClientError> {
        self.logger.debug("Getting DHT record");
        let record = self.dht.mutable_get(self.public_key, None).await?;
        match &record {
            Some(r) => self.logger.debug(&format!(
                "DHT get completed: seq={}, value={}",
                r.seq,
                String::from_utf8_lossy(&r.value)
            )),
            None => self.logger.warn("DHT get: no record found"),
        }
        Ok(record)
    }

    pub fn info(&self) -> ClientInfo {
        let merged = self.merged.as_ref();
        ClientInfo {
            kind: "client",
            state: self.state,
            secure: self.secure,
            port: merged.map_or(JsValue::Null, |m| m.port.clone()),
            host: merged.map_or(JsValue::Null, |m| m.host.clone()),
            protocol: merged.map_or("tcp", |m| if m.udp { "udp" } else { "tcp" }),
            key: self.key.clone(),
            public_key: z32::encode(&self.public_key),
        }
    }

    /// `pause()`: INFO `Pausing client`, `Dht::suspend`, `state = paused`, INFO `Client paused` (client.md).
    pub async fn pause(&mut self) -> Result<(), ClientError> {
        self.logger.info("Pausing client");
        self.dht.suspend().await?;
        self.state = Some("paused");
        self.logger.info("Client paused");
        Ok(())
    }

    /// `resume()`: INFO `Resuming client`, `Dht::resume`, `state = listening`, INFO `Client resumed` (client.md).
    pub async fn resume(&mut self) -> Result<(), ClientError> {
        self.logger.info("Resuming client");
        self.dht.resume().await?;
        self.state = Some("listening");
        self.logger.info("Client resumed");
        Ok(())
    }

    /// `Dht::destroy`, closing the proxy, `state = destroyed`.
    pub async fn destroy(&mut self) -> Result<(), ClientError> {
        self.logger.info("Destroying client");
        self.dht.destroy().await?;
        if let Some(proxy) = self.proxy.take() {
            proxy.close().await;
        }
        self.state = Some("destroyed");
        self.logger.info("Client destroyed");
        Ok(())
    }
}

#[cfg(test)]
#[path = "connect_tests.rs"]
mod connect_tests;
#[cfg(test)]
#[path = "pause_tests.rs"]
mod pause_tests;
#[cfg(test)]
#[path = "proxy_tests.rs"]
mod proxy_tests;
#[cfg(test)]
#[path = "record_tests.rs"]
mod record_tests;
#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
