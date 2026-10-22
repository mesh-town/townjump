// JS: holesail-client/index.js:HolesailClient.ping (`--lookup`), order of attempts as in dht-records.md «--lookup».
// Related: client.rs; the response is read by the holesail facade (`Holesail::lookup`).
use crate::error::ClientError;
use townjump_dht::{Dht, DhtFactory};
use townjump_jsval::JsValue;
use townjump_keys::{KeysError, key_pair_from_seed, z32};

/// Server record found by `ping`: the parsed JSON plus the added `protocol` key (`udp` or `tcp`), as in JS.
#[derive(Debug, Clone, PartialEq)]
pub struct LookupRecord {
    pub record: JsValue,
}

/// A new node with a random identity: attempt "as private" (`key` is the z32 of a seed), then "as public".
/// Invalid record JSON is skipped; the node is destroyed before returning.
pub async fn ping<F: DhtFactory>(
    factory: &F,
    key: &str,
) -> Result<Option<LookupRecord>, ClientError> {
    let key_bytes: [u8; 32] = z32::decode(key)?
        .try_into()
        .map_err(|_| KeysError::SeedBytes)?;
    let dht = factory.create(None).await?;
    let found = attempts(&dht, key_bytes).await;
    dht.destroy().await?;
    let result = found?.filter(|v| *v != JsValue::Null);
    Ok(result.map(|record| LookupRecord { record }))
}

/// Attempt A (key pair from the seed); if the result is falsy, B (the key itself). As in JS, `result` persists across attempts:
/// an empty B (no record, bad JSON) keeps what A produced, while a `null` from parsing overwrites it.
async fn attempts<D: Dht>(dht: &D, key: [u8; 32]) -> Result<Option<JsValue>, ClientError> {
    let derived = key_pair_from_seed(&key).public_key;
    let first = attempt(dht, derived).await?;
    if first.as_ref().is_some_and(JsValue::truthy) {
        return Ok(first);
    }
    Ok(attempt(dht, key).await?.or(first))
}

/// One read: the `JSON.parse` result (for an object, with `protocol` added); no record or a parse error gives `None`.
async fn attempt<D: Dht>(dht: &D, pk: [u8; 32]) -> Result<Option<JsValue>, ClientError> {
    let record = dht.mutable_get(pk, Some(true)).await?;
    let Some(record) = record else {
        return Ok(None);
    };
    let parsed = JsValue::parse_json(&String::from_utf8_lossy(&record.value));
    Ok(parsed.ok().map(with_protocol))
}

/// `result.protocol = result.udp ? 'udp' : 'tcp'`: for an object it is a key (in place if already present); for other values the assignment is invisible.
fn with_protocol(value: JsValue) -> JsValue {
    let JsValue::Object(mut pairs) = value else {
        return value;
    };
    let udp = pairs
        .iter()
        .find(|(k, _)| k == "udp")
        .is_some_and(|(_, v)| v.truthy());
    let protocol = JsValue::String(if udp { "udp" } else { "tcp" }.to_owned());
    match pairs.iter_mut().find(|(k, _)| k == "protocol") {
        Some(slot) => slot.1 = protocol,
        None => pairs.push(("protocol".to_owned(), protocol)),
    }
    JsValue::Object(pairs)
}

#[cfg(test)]
#[path = "ping_primitive_tests.rs"]
mod primitive_tests;
#[cfg(test)]
#[path = "ping_tests.rs"]
mod tests;
