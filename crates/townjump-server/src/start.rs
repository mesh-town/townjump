// JS: holesail-server/index.js:HolesailServer.start (order as in server.md «start(args)»); related: server.rs, refresh.rs.
use crate::connection::Acceptor;
use crate::server::{HolesailServer, Running, key_of};
use crate::{args::ServerArgs, error::ServerError, listen::listen_now, refresh, seed};
use std::sync::Arc;
use townjump_dht::{Dht, Firewall, KeyPair, ServerOpts};
use townjump_jsval::record_json;
use townjump_keys::{fingerprint, key_pair_from_seed};

impl<D: Dht> HolesailServer<D> {
    /// Order as in server.md; returns after the first `put`, errors of background tasks go to `Fatal`.
    pub async fn start(&mut self, args: ServerArgs) -> Result<(), ServerError> {
        self.logger.info("Starting server");
        let (seed, seed_hex) = seed::resolve(args.seed_hex.as_deref())?;
        let key_pair = key_pair_from_seed(&seed);
        let masked = fingerprint(&seed_hex);
        self.logger
            .debug(&format!("Generated key pair from seed: {masked}"));
        let opts = self.server_opts(args.secure, &key_pair);
        let (server, incoming) = self.dht.create_server(opts)?;
        self.logger.debug("Server created, awaiting listen");
        let mut running = Running {
            args,
            seed,
            key_pair: key_pair.clone(),
            server: server.clone(),
            tasks: Vec::new(),
        };
        let listening = self.listen(&running, server, key_pair);
        running.tasks.extend(listening);
        running
            .tasks
            .push(tokio::spawn(self.acceptor(&running).run(incoming)));
        let data = self.record(&running);
        self.running = Some(running);
        self.publish_first(data).await
    }

    fn server_opts(&self, secure: bool, key_pair: &KeyPair) -> ServerOpts {
        let firewall = secure.then(|| refuse_others(key_pair.public_key));
        self.logger.info(match secure {
            true => "Using Private Mode",
            false => "Using Public Mode",
        });
        ServerOpts {
            firewall,
            reusable_socket: true,
            ..Default::default()
        }
    }

    /// `server.listen(keyPair)` without waiting: the result is the state and a log line, or `Fatal`.
    fn listen(
        &self,
        running: &Running<D>,
        server: D::Server,
        key_pair: KeyPair,
    ) -> Option<tokio::task::JoinHandle<()>> {
        let (state, logger, fatal) = (self.state.clone(), self.logger.clone(), self.fatal.clone());
        let shown = shown_key(running);
        let done = move |result: Result<(), townjump_dht::DhtError>| match result {
            Ok(()) => {
                *state.lock().unwrap() = Some("listening");
                logger.info(&format!("Server listening on key: {shown}"));
            }
            Err(error) => fatal.raise(error.to_string()),
        };
        listen_now(server, key_pair, done)
    }

    fn acceptor(&self, running: &Running<D>) -> Acceptor {
        let args = &running.args;
        let udp = args.udp.as_ref().is_some_and(|v| v.truthy());
        Acceptor::new(
            self.logger.clone(),
            args.host.clone(),
            args.port.clone(),
            udp,
        )
    }

    fn record(&self, running: &Running<D>) -> String {
        let args = &running.args;
        let data = record_json(Some(&args.host), args.udp.as_ref(), Some(&args.port));
        self.logger
            .debug(&format!("Initializing DHT with host info: {data}"));
        data
    }

    /// The first `put` (its error is the `start` error) and the refresh timer.
    async fn publish_first(&mut self, data: String) -> Result<(), ServerError> {
        self.put(&data).await?;
        let running = self.running.as_mut().ok_or(ServerError::NotStarted)?;
        let key_pair = running.key_pair.clone();
        let task = refresh::spawn(self.dht.clone(), key_pair, data, &self.logger, &self.fatal);
        running.tasks.push(task);
        Ok(())
    }
}

/// Private mode firewall: reject everyone except the own public key (true = reject).
fn refuse_others(own: [u8; 32]) -> Firewall {
    Arc::new(move |remote: &[u8; 32]| remote != &own)
}

/// Key in the `listen` line: a secret is masked, a public key is not.
fn shown_key<D: Dht>(running: &Running<D>) -> String {
    let key = key_of(running);
    match running.args.secure {
        true => fingerprint(&key),
        false => key,
    }
}
