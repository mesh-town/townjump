// JS: holesail-server/index.js:HolesailServer (put, get, key, info, pause, resume, destroy); start — start.rs.
// Related: args.rs, info.rs, error.rs, record.rs (put/get), connection.rs, listen.rs; test scenarios are split into start/firewall/record/pause.
use crate::{args::ServerArgs, error::ServerError, info::ServerInfo, record};
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;
use townjump_dht::{Dht, DhtFactory, DhtServer, KeyPair, MutableRecord};
use townjump_keys::z32;
use townjump_logger::{Fatal, Logger};

/// Shared `state`: `None` before `listen`, then `listening`/`paused`/`destroyed`.
pub(crate) type State = Arc<Mutex<Option<&'static str>>>;

/// What appears after `start`.
pub(crate) struct Running<D: Dht> {
    pub args: ServerArgs,
    pub seed: [u8; 32],
    pub key_pair: KeyPair,
    /// Keeps the DHT server alive until `destroy`.
    pub server: D::Server,
    pub tasks: Vec<JoinHandle<()>>,
}

/// Holesail server on top of the DHT node `D`.
pub struct HolesailServer<D: Dht> {
    pub(crate) dht: Arc<D>,
    pub(crate) logger: Logger,
    pub(crate) fatal: Fatal,
    pub(crate) state: State,
    pub(crate) running: Option<Running<D>>,
}

impl<D: Dht> HolesailServer<D> {
    /// `new HolesailServer()`: creates the node `factory.create(None)` (random identity).
    pub async fn new<F: DhtFactory<Dht = D>>(
        factory: &F,
        logger: Logger,
        fatal: Fatal,
    ) -> Result<Self, ServerError> {
        let dht = Arc::new(factory.create(None).await?);
        Ok(HolesailServer {
            dht,
            logger,
            fatal,
            state: State::default(),
            running: None,
        })
    }

    /// The DHT node of this server.
    pub fn dht(&self) -> &Arc<D> {
        &self.dht
    }

    /// `put(data)`: reads the record, chooses `seq`, writes; returns the written `seq`.
    pub async fn put(&self, data: &str) -> Result<u64, ServerError> {
        let running = self.running.as_ref().ok_or(ServerError::NotStarted)?;
        record::put(&*self.dht, &running.key_pair, data, &self.logger).await
    }

    /// Record under its own public key.
    pub async fn get(&self) -> Result<Option<MutableRecord>, ServerError> {
        let running = self.running.as_ref().ok_or(ServerError::NotStarted)?;
        record::read(&*self.dht, running.key_pair.public_key, None, &self.logger).await
    }

    /// `z32(seed)` in private mode, otherwise `z32(public_key)`.
    pub fn key(&self) -> String {
        self.running.as_ref().map_or_else(String::new, key_of)
    }

    pub fn info(&self) -> ServerInfo {
        let state = *self.state.lock().unwrap();
        let Some(running) = &self.running else {
            return ServerInfo::unstarted(state);
        };
        ServerInfo::of(running, state, key_of(running))
    }

    /// `pause()`: INFO `Pausing server`, `Dht::suspend`, `state = paused`, INFO `Server paused` (server.md).
    pub async fn pause(&mut self) -> Result<(), ServerError> {
        self.logger.info("Pausing server");
        self.dht.suspend().await?;
        *self.state.lock().unwrap() = Some("paused");
        self.logger.info("Server paused");
        Ok(())
    }

    /// `resume()`: INFO `Resuming server`, `Dht::resume`, `state = listening`, INFO `Server resumed` (server.md).
    pub async fn resume(&mut self) -> Result<(), ServerError> {
        self.logger.info("Resuming server");
        self.dht.resume().await?;
        *self.state.lock().unwrap() = Some("listening");
        self.logger.info("Server resumed");
        Ok(())
    }

    /// Drops the tasks and the timer, `Dht::destroy`, `state = destroyed`.
    pub async fn destroy(&mut self) -> Result<(), ServerError> {
        self.logger.info("Destroying server");
        self.stop_tasks();
        self.close_server().await?;
        self.logger.info("Cleared DHT refresh interval");
        self.dht.destroy().await?;
        *self.state.lock().unwrap() = Some("destroyed");
        self.logger.info("Server destroyed");
        Ok(())
    }

    /// JS closes the servers inside `dht.destroy()`; here it is explicit, before it.
    async fn close_server(&self) -> Result<(), ServerError> {
        let Some(running) = &self.running else {
            return Ok(());
        };
        Ok(running.server.close().await?)
    }

    fn stop_tasks(&mut self) {
        let Some(running) = &mut self.running else {
            return;
        };
        running.tasks.drain(..).for_each(|task| task.abort());
    }
}

/// Private mode is `z32(seed)`, public mode is `z32(public_key)`.
pub(crate) fn key_of<D: Dht>(running: &Running<D>) -> String {
    match running.args.secure {
        true => z32::encode(&running.seed),
        false => z32::encode(&running.key_pair.public_key),
    }
}

#[cfg(test)]
#[path = "data_tests.rs"]
mod data_tests;
#[cfg(test)]
#[path = "firewall_tests.rs"]
mod firewall_tests;
#[cfg(test)]
#[path = "fixture_tests.rs"]
mod fixture;
#[cfg(test)]
#[path = "pause_tests.rs"]
mod pause_tests;
#[cfg(test)]
#[path = "record_tests.rs"]
mod record_tests;
#[cfg(test)]
#[path = "refresh_tests.rs"]
mod refresh_tests;
#[cfg(test)]
#[path = "start_tests.rs"]
mod start_tests;
#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
