/// Secret fingerprint for the log: the first 8 characters, the rest `*`.
/// Precisely: the first `min(8, n)` characters of the string (by `char`), then one `*` for each following one; the string length is preserved.
pub fn fingerprint(secret: &str) -> String {
    let head = secret.chars().take(8);
    let tail = secret.chars().skip(8).map(|_| '*');
    head.chain(tail).collect()
}

#[cfg(test)]
#[path = "fingerprint_tests.rs"]
mod tests;
