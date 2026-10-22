// JS: holesail/src/index.js:Holesail.lookup — `{ ...(await ping(argKey)) || {}, secure }`.
// See also: ping is in townjump-client; cli/run prints the result.
use super::HolesailError;
use townjump_client::{ClientError, ping};
use townjump_dht::DhtFactory;
use townjump_jsval::JsValue;
use townjump_keys::{KeysError, parse_url, sha256_seed, z32};

pub(super) async fn lookup<F: DhtFactory>(
    factory: &F,
    url: &str,
) -> Result<JsValue, HolesailError> {
    let parsed = parse_url(url);
    let arg_key = arg_key(&parsed.key, parsed.secure == Some(true))?;
    check_seed_length(factory, &arg_key).await?;
    let found = ping(factory, &arg_key).await?;
    let record = found.map(|f| f.record).filter(JsValue::truthy);
    Ok(with_secure(record, parsed.secure))
}

/// Private — `z32(sha256(K))`, otherwise the key itself, which must be z32.
fn arg_key(key: &str, secure: bool) -> Result<String, HolesailError> {
    if secure {
        return Ok(z32::encode(&sha256_seed(key)));
    }
    match z32::decode(key) {
        Ok(_) => Ok(key.to_owned()),
        Err(_) => Err(HolesailError::InvalidKeyFormat(key.to_owned())),
    }
}

/// JS creates the node before it parses the key: a z32 key not of 32 bytes yields a seed error only after `HyperDHT.new`.
async fn check_seed_length<F: DhtFactory>(factory: &F, key: &str) -> Result<(), HolesailError> {
    let bytes = z32::decode(key)?;
    if bytes.len() == 32 {
        return Ok(());
    }
    let _left_open = factory.create(None).await.map_err(ClientError::from)?;
    Err(KeysError::SeedBytes.into())
}

/// The `secure` field overrides the found value: present only for `Some(true)`.
fn with_secure(record: Option<JsValue>, secure: Option<bool>) -> JsValue {
    let JsValue::Object(mut entries) = record.unwrap_or(JsValue::Object(vec![])) else {
        return JsValue::Object(vec![]);
    };
    entries.retain(|(name, _)| name != "secure");
    if secure == Some(true) {
        entries.push(("secure".to_owned(), JsValue::Bool(true)));
    }
    JsValue::Object(entries)
}
