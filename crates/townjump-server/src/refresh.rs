// JS: `setInterval(..., 50 * 60 * 1000)` in holesail-server/index.js:start; related: start.rs, record.rs.
// Record refresh every 50 minutes with the same value; an error goes to `Fatal`, the task ends.
use crate::record;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use townjump_dht::{Dht, KeyPair};
use townjump_logger::{Fatal, Logger};

const PERIOD: Duration = Duration::from_secs(50 * 60);

pub(crate) fn spawn<D: Dht>(
    dht: Arc<D>,
    key_pair: KeyPair,
    data: String,
    logger: &Logger,
    fatal: &Fatal,
) -> JoinHandle<()> {
    let (logger, fatal) = (logger.clone(), fatal.clone());
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(PERIOD).await;
            logger.debug(&format!("Refreshing DHT record: {data}"));
            let put = record::put(&*dht, &key_pair, &data, &logger);
            let Err(error) = put.await else {
                continue;
            };
            return fatal.raise(error.to_string());
        }
    })
}
