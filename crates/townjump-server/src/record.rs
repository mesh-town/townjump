// JS: holesail-server/index.js:HolesailServer.put/get (choice of seq, DEBUG lines); related: server.rs, refresh.rs.
use crate::error::ServerError;
use townjump_dht::{Dht, KeyPair, MutableRecord};
use townjump_logger::Logger;

/// `get(opts)`: reads the record by its own key; a found record is written to the log.
pub(crate) async fn read<D: Dht>(
    dht: &D,
    public_key: [u8; 32],
    latest: Option<bool>,
    logger: &Logger,
) -> Result<Option<MutableRecord>, ServerError> {
    let record = dht.mutable_get(public_key, latest).await?;
    let Some(found) = &record else {
        return Ok(record);
    };
    let value = String::from_utf8_lossy(&found.value);
    logger.debug(&format!(
        "Existing DHT record found: seq={}, value={value}",
        found.seq
    ));
    Ok(record)
}

/// `put(data)`: reads the record, chooses `seq`, writes; returns the written `seq`.
pub(crate) async fn put<D: Dht>(
    dht: &D,
    key_pair: &KeyPair,
    data: &str,
    logger: &Logger,
) -> Result<u64, ServerError> {
    logger.debug(&format!("Putting DHT record: {data}"));
    logger.debug(&format!("Incoming data type: string, value: {data}"));
    logger.debug("Checking for existing DHT record");
    let old = read(dht, key_pair.public_key, Some(true), logger).await?;
    let seq = next_seq(old.as_ref(), data.as_bytes(), logger);
    let written = dht.mutable_put(key_pair, data.as_bytes(), seq).await?;
    logger.debug(&format!("DHT put completed with seq: {written}"));
    Ok(written)
}

fn next_seq(old: Option<&MutableRecord>, value: &[u8], logger: &Logger) -> Option<u64> {
    let Some(old) = old else {
        logger.debug("No existing DHT record found, creating new");
        return None;
    };
    let same = old.value == value;
    let seq = if same { old.seq } else { old.seq + 1 };
    logger.debug(&format!(
        "Existing record found, putting with seq: {seq} (same: {same})"
    ));
    Some(seq)
}
