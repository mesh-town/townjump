// JS: holesail/src/index.js:Holesail (validateOpts, #initialise, urlParser): mode and keys computed in the constructor.
// See also: facade.rs takes the client seed and server keys from here.
use super::{HolesailError, HolesailOpts};
use townjump_jsval::JsValue;
use townjump_keys::{ServerKeys, parse_url, server_keys, sha256_seed, z32};

/// Outcome of `#initialise`: server keys, or the client key and seed.
pub(super) enum Plan {
    Server {
        keys: ServerKeys,
        secure: bool,
    },
    Client {
        /// `K` after `urlParser`.
        key: String,
        /// Private mode — `z32(sha256(K))`, otherwise `K` itself.
        seed: String,
        secure: bool,
    },
}

impl Plan {
    /// `validateOpts` and `#initialise`; `random` is called only for a server without a key.
    pub(super) fn build(
        opts: &HolesailOpts,
        random: &mut dyn FnMut(&mut [u8]),
    ) -> Result<Plan, HolesailError> {
        let text = key_text(opts);
        if !opts.server && text.is_empty() {
            return Err(HolesailError::KeyEmpty);
        }
        if opts.server && !opts.host.as_ref().is_some_and(JsValue::truthy) {
            return Err(HolesailError::NoHost);
        }
        let parsed = parse_url(&text);
        let secure = parsed.secure.unwrap_or(opts.secure);
        if opts.server {
            let key = Some(parsed.key.as_str());
            let keys = server_keys(key, secure, random);
            return Ok(Plan::Server { keys, secure });
        }
        let seed = client_seed(&parsed.key, secure);
        Ok(Plan::Client {
            key: parsed.key,
            seed,
            secure,
        })
    }

    pub(super) fn secure(&self) -> bool {
        match self {
            Plan::Server { secure, .. } | Plan::Client { secure, .. } => *secure,
        }
    }

    /// Seed and key for the server `info`: `args.seed` and `Holesail.info.key`.
    pub(super) fn server_seed_and_key(&self) -> (Option<String>, String) {
        self.shown()
    }

    /// `(seed, key)` for `info` while there is no information from the node.
    pub(super) fn shown(&self) -> (Option<String>, String) {
        match self {
            Plan::Server { keys, .. } => (keys.seed_hex.clone(), keys.info_key.clone()),
            Plan::Client { .. } => (self.client_seed_hex(), self.client_shown_key()),
        }
    }

    /// A private client shows `K` (empty — the seed), a public one — the seed.
    pub(super) fn client_shown_key(&self) -> String {
        match self {
            Plan::Client {
                key, secure: true, ..
            } if !key.is_empty() => key.clone(),
            Plan::Client { seed, .. } => seed.clone(),
            Plan::Server { .. } => String::new(),
        }
    }

    /// `hex(sha256(K))` — only for a private client.
    pub(super) fn client_seed_hex(&self) -> Option<String> {
        match self {
            Plan::Client {
                key, secure: true, ..
            } => Some(hex::encode(sha256_seed(key))),
            _ => None,
        }
    }
}

/// `String(opts.key)` for a truthy key, otherwise empty.
fn key_text(opts: &HolesailOpts) -> String {
    let key = opts.key.as_ref().filter(|k| k.truthy());
    key.map(JsValue::to_js_string).unwrap_or_default()
}

fn client_seed(key: &str, secure: bool) -> String {
    if secure {
        return z32::encode(&sha256_seed(key));
    }
    key.to_owned()
}
