// JS: dht-rpc/lib/io.js
//! `impl Io`: round-trip token generation and validation with rotating secrets.
//! Neighbours: `datagram` validates tokens; `replies` attaches them.

use blake2::digest::Mac;

use super::state::Io;
use super::types::Blake2bMac256;

impl Io {
    /// Generate a token for the given host using `secret[secret_index]`.
    pub fn token(&mut self, host: &str, secret_index: usize) -> [u8; 32] {
        self.init_secrets_if_needed();
        let secrets = match self.secrets.as_ref() {
            Some(s) => *s,
            None => return [0u8; 32],
        };
        let key = &secrets[secret_index % 2];
        match Blake2bMac256::new_from_slice(key) {
            Ok(mut mac) => {
                mac.update(host.as_bytes());
                let hash = mac.finalize().into_bytes();
                let mut token = [0u8; 32];
                token.copy_from_slice(&hash);
                token
            }
            Err(_) => [0u8; 32],
        }
    }

    /// Validate an incoming token against both secrets.
    pub fn validate_token(&mut self, host: &str, token: &[u8; 32]) -> bool {
        let t0 = self.token(host, 0);
        let t1 = self.token(host, 1);
        &t0 == token || &t1 == token
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    pub(super) fn init_secrets_if_needed(&mut self) {
        if self.secrets.is_none() {
            let s0: [u8; 32] = rand::random();
            let s1: [u8; 32] = rand::random();
            self.secrets = Some([s0, s1]);
        }
    }
}
