/// Key parsing errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeysError {
    /// `z32.decode`: character outside the alphabet; `position` is the character index in the string.
    #[error("Invalid character in base32 input: \"{ch}\" at position {position}")]
    InvalidCharacter { ch: String, position: usize },
    /// `HyperDHT.keyPair(seed)` with a seed length other than 32 bytes (for example `--lookup` without a value).
    #[error("seed must be 'crypto_sign_SEEDBYTES' bytes")]
    SeedBytes,
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
