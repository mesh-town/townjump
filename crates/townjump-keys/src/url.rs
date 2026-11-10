/// Result of splitting a connection string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedUrl {
    pub key: String,
    /// `Some(true)` for the private marker; never `Some(false)`.
    pub secure: Option<bool>,
}

const PREFIX: &str = "hs://";
/// Prefix plus the four mode characters.
const HEAD_UNITS: usize = 9;

/// Splits `url` into key and private flag. Positions are UTF-16 units, so the
/// cut may land inside a surrogate pair and yield U+FFFD.
pub fn parse_url(url: &str) -> ParsedUrl {
    let units: Vec<u16> = url.encode_utf16().collect();
    let key = if url.starts_with(PREFIX) && units.len() >= HEAD_UNITS {
        String::from_utf16_lossy(&units[HEAD_UNITS..])
    } else {
        url.to_string()
    };
    // The flag is looked at regardless of the prefix.
    let secure = (units.get(5) == Some(&u16::from(b's'))).then_some(true);
    ParsedUrl { key, secure }
}

/// Builds `hs://` + mode marker + key, without any validation.
pub fn format_url(key: &str, secure: bool) -> String {
    let marker = if secure { "s000" } else { "0000" };
    format!("{PREFIX}{marker}{key}")
}

#[cfg(test)]
#[path = "url_tests.rs"]
mod tests;
